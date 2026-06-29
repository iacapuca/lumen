//! `lumen-auth` — JWT verification and the multi-tenant [`Principal`].
//!
//! MVP uses HS256 with a shared secret. The path to production is swapping the
//! `DecodingKey`/`Validation` for RS256/ES256 + a JWKS endpoint with `kid`
//! rotation — the claim shape and [`Principal`] are stable across that change.

use std::collections::HashSet;

use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use lumen_shared::SecurityContext;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid or expired token: {0}")]
    Invalid(#[from] jsonwebtoken::errors::Error),
    #[error("missing token")]
    Missing,
}

/// The verified, tenant-scoped principal for a request. `sc_hash` is computed
/// once and is a mandatory segment of every downstream cache key.
#[derive(Debug, Clone)]
pub struct Principal {
    pub tenant_id: String,
    pub sub: String,
    pub roles: Vec<String>,
    pub permissions: HashSet<String>,
    /// Forwarded verbatim to the semantic layer for row-level security.
    pub security_context: SecurityContext,
    pub sc_hash: String,
}

impl Principal {
    pub fn has_permission(&self, perm: &str) -> bool {
        self.permissions.contains(perm)
    }
}

/// JWT claim shape. `security_context` is opaque to Lumen and forwarded to Cube.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aud: Option<String>,
    pub sub: String,
    pub exp: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nbf: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iat: Option<i64>,
    pub tenant_id: String,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub security_context: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub secret: Vec<u8>,
    pub issuer: Option<String>,
    pub audience: Option<String>,
}

impl AuthConfig {
    pub fn new(secret: impl Into<Vec<u8>>) -> Self {
        AuthConfig {
            secret: secret.into(),
            issuer: None,
            audience: None,
        }
    }
}

/// Verify a token and resolve a [`Principal`]. Fail-closed: signature, `exp` and
/// `nbf` are always checked.
pub fn verify(token: &str, cfg: &AuthConfig) -> Result<Principal, AuthError> {
    let mut v = Validation::new(Algorithm::HS256);
    v.validate_exp = true;
    v.validate_nbf = true;
    v.leeway = 30; // bounded clock skew, not unbounded
    v.set_required_spec_claims(&["exp"]);

    match &cfg.issuer {
        Some(iss) => v.set_issuer(&[iss]),
        None => {}
    }
    match &cfg.audience {
        Some(aud) => v.set_audience(&[aud]),
        None => v.validate_aud = false,
    }

    let data = decode::<Claims>(token, &DecodingKey::from_secret(&cfg.secret), &v)?;
    let c = data.claims;
    let sc = SecurityContext(c.security_context);
    let sc_hash = sc.sc_hash();
    Ok(Principal {
        tenant_id: c.tenant_id,
        sub: c.sub,
        roles: c.roles,
        permissions: c.permissions.into_iter().collect(),
        security_context: sc,
        sc_hash,
    })
}

/// Input for minting a token (used by the dev `/embed` demo + tests).
pub struct TokenInput {
    pub tenant_id: String,
    pub sub: String,
    pub roles: Vec<String>,
    pub permissions: Vec<String>,
    pub security_context: serde_json::Value,
    pub ttl_secs: i64,
}

/// Mint an HS256 token. In production, tenants mint their own embed tokens
/// server-side with their copy of the shared secret.
pub fn mint_token(cfg: &AuthConfig, input: &TokenInput) -> Result<String, AuthError> {
    let now = chrono::Utc::now().timestamp();
    let claims = Claims {
        iss: cfg.issuer.clone(),
        aud: cfg.audience.clone(),
        sub: input.sub.clone(),
        exp: now + input.ttl_secs,
        nbf: Some(now),
        iat: Some(now),
        tenant_id: input.tenant_id.clone(),
        roles: input.roles.clone(),
        permissions: input.permissions.clone(),
        security_context: input.security_context.clone(),
    };
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(&cfg.secret),
    )?;
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> AuthConfig {
        AuthConfig::new(b"test-secret".to_vec())
    }

    #[test]
    fn mint_then_verify_round_trips() {
        let token = mint_token(
            &cfg(),
            &TokenInput {
                tenant_id: "acme".into(),
                sub: "user_1".into(),
                roles: vec!["viewer".into()],
                permissions: vec!["dashboard:sales:read".into()],
                security_context: serde_json::json!({"tenant_id":"acme","region":"us"}),
                ttl_secs: 300,
            },
        )
        .unwrap();

        let p = verify(&token, &cfg()).unwrap();
        assert_eq!(p.tenant_id, "acme");
        assert!(p.has_permission("dashboard:sales:read"));
        assert!(!p.sc_hash.is_empty());
    }

    #[test]
    fn rejects_wrong_secret() {
        let token = mint_token(
            &cfg(),
            &TokenInput {
                tenant_id: "acme".into(),
                sub: "u".into(),
                roles: vec![],
                permissions: vec![],
                security_context: serde_json::json!({}),
                ttl_secs: 300,
            },
        )
        .unwrap();
        let other = AuthConfig::new(b"different".to_vec());
        assert!(verify(&token, &other).is_err());
    }

    #[test]
    fn rejects_expired() {
        let token = mint_token(
            &cfg(),
            &TokenInput {
                tenant_id: "acme".into(),
                sub: "u".into(),
                roles: vec![],
                permissions: vec![],
                security_context: serde_json::json!({}),
                ttl_secs: -3600, // already expired
            },
        )
        .unwrap();
        assert!(verify(&token, &cfg()).is_err());
    }
}
