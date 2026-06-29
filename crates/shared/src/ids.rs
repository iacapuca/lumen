//! Content-addressed ids. Canonicalization (deterministic serde encoding) is the
//! linchpin: it makes query ids and the whole-DEP content hash dedup-correct
//! regardless of field ordering in the source dashboard JSON.

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! string_id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl From<String> for $name {
            fn from(s: String) -> Self {
                $name(s)
            }
        }
        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                $name(s.to_owned())
            }
        }
    };
}

string_id!(
    /// Dashboard / DEP id, e.g. `"sales"`.
    DashboardId
);
string_id!(
    /// Positional widget id within a DEP, e.g. `"w_0"`.
    WidgetId
);
string_id!(
    /// Content-addressed query id, e.g. `"q_3a8f12…"`.
    QueryId
);
string_id!(
    /// Chart-spec id, e.g. `"c_kpi_rev"`.
    ChartId
);
string_id!(
    /// Full content hash, e.g. `"blake3:<64hex>"`.
    Hash
);

impl WidgetId {
    pub fn positional(i: usize) -> Self {
        WidgetId(format!("w_{i}"))
    }
}

impl QueryId {
    /// blake3 over the canonical JSON encoding of a query, truncated to 16 bytes.
    /// Truncation is safe for intra-DEP dedup; the manifest keeps the full
    /// content hash for global addressing.
    pub fn of(q: &crate::Query) -> Self {
        let canon = serde_json::to_vec(q).expect("Query is always serializable");
        let h = blake3::hash(&canon);
        QueryId(format!("q_{}", hex::encode(&h.as_bytes()[..16])))
    }
}

impl Hash {
    /// `"blake3:<64 hex>"` over arbitrary bytes — the content-addressing primitive.
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Hash(format!("blake3:{}", hex::encode(blake3::hash(bytes).as_bytes())))
    }
}
