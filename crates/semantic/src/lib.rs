//! `lumen-semantic` — the [`SemanticLayer`] seam and its adapters.
//!
//! Lumen never owns metrics or modeling; it delegates to a semantic layer. The
//! neutral [`Query`] IR is translated at execution time, so nothing
//! provider-specific is ever frozen into a compiled DEP. Each backend is a new
//! `impl SemanticLayer`:
//!
//! - [`cube::CubeClient`] — Cube.dev REST (`/load` + `/meta`), RLS via the
//!   security-context JWT that Cube enforces server-side.
//! - [`dbt::DbtClient`] — dbt Semantic Layer / MetricFlow GraphQL, RLS injected
//!   as explicit `where` filters (dbt has no server-side equivalent).
//!
//! Callers don't name adapters directly — they build a [`provider::SemanticConfig`]
//! from the environment and call [`provider::SemanticConfig::build`].

use async_trait::async_trait;
use lumen_shared::{Data, Query, SecurityContext};
use serde_json::Value;

pub mod cube;
pub mod dbt;
pub mod provider;

pub use cube::CubeClient;
pub use dbt::DbtClient;
pub use provider::{Provider, SemanticConfig};

#[derive(Debug, thiserror::Error)]
pub enum SemanticError {
    #[error("transport error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("token signing error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
    #[error("semantic layer error: {0}")]
    Backend(String),
    #[error("query timed out after {0} retries (Continue wait)")]
    Timeout(u32),
}

/// The swappable semantic-layer seam. `load` *requires* a [`SecurityContext`] so
/// "forgot to forward RLS" is a compile error, not a runtime bug.
#[async_trait]
pub trait SemanticLayer: Send + Sync {
    async fn load(&self, query: &Query, sc: &SecurityContext) -> Result<Data, SemanticError>;
    /// Raw data-model metadata (e.g. Cube's `/meta` cubes + views, or dbt's
    /// metrics listing). Shape is provider-specific; nothing in a compiled DEP
    /// depends on it.
    async fn meta(&self) -> Result<Value, SemanticError>;
}
