//! Provider selection — turns neutral config into an `Arc<dyn SemanticLayer>`.
//!
//! This is the single place that knows the set of concrete adapters. Callers
//! (the runtime, the CLI) construct a [`SemanticConfig`] from the environment
//! and call [`build`]; they never name `CubeClient` / `DbtClient` directly.

use std::sync::Arc;

use crate::{cube::CubeClient, dbt::DbtClient, SemanticLayer};

/// Which semantic-layer backend to talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Cube,
    Dbt,
}

impl Provider {
    /// Parse from the `SEMANTIC_PROVIDER` env value. Defaults to `Cube` (the
    /// historical behavior) for any unset/unrecognized value.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "dbt" | "dbt-sl" | "metricflow" => Provider::Dbt,
            _ => Provider::Cube,
        }
    }
}

/// Neutral, provider-agnostic config. Only the fields for the selected
/// `provider` need to be populated.
#[derive(Debug, Clone, Default)]
pub struct SemanticConfig {
    pub provider: Option<Provider>,
    // --- Cube ---
    pub cube_url: String,
    pub cube_secret: String,
    // --- dbt Semantic Layer ---
    pub dbt_graphql_url: String,
    pub dbt_service_token: String,
    pub dbt_environment_id: String,
}

impl SemanticConfig {
    /// Build the concrete adapter for the selected provider.
    pub fn build(&self) -> Arc<dyn SemanticLayer> {
        match self.provider.unwrap_or(Provider::Cube) {
            Provider::Cube => {
                Arc::new(CubeClient::new(self.cube_url.clone(), self.cube_secret.clone()))
            }
            Provider::Dbt => Arc::new(DbtClient::new(
                self.dbt_graphql_url.clone(),
                self.dbt_service_token.clone(),
                self.dbt_environment_id.clone(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_parses_dbt_aliases_else_cube() {
        assert_eq!(Provider::parse("dbt"), Provider::Dbt);
        assert_eq!(Provider::parse("MetricFlow"), Provider::Dbt);
        assert_eq!(Provider::parse("cube"), Provider::Cube);
        assert_eq!(Provider::parse(""), Provider::Cube);
        assert_eq!(Provider::parse("whatever"), Provider::Cube);
    }
}
