//! Usage metering — the billing substrate. Lumen is usage-priced, never
//! seat-priced: tenants are billed on summed **compute credits**.
//!
//! The deliberate pricing incentive: a cache hit costs only its widget base
//! (no query base, no compute term), so warm dashboards are cheap and cold,
//! heavy analytical queries cost more.

use serde::{Deserialize, Serialize};

use crate::{ChartKind, WidgetKind};

/// 1 credit per executed (non-cached) query.
pub const QUERY_BASE: u32 = 1;
/// 1 credit per MiB of egress.
pub const BYTES_PER_CREDIT: u64 = 1_048_576;
/// Flat floor billed for a 304 / fully-warm render.
pub const RENDER_FLOOR: u32 = 1;

/// Layout cost per widget kind.
pub fn widget_base(kind: WidgetKind) -> u32 {
    match kind {
        WidgetKind::Kpi => 1,
        WidgetKind::Table => 2,
        WidgetKind::LineChart => 2,
        WidgetKind::BarChart => 2,
    }
}

/// Layout cost per compiled chart kind (used by the compiler's credit estimate).
pub fn chart_base(kind: ChartKind) -> u32 {
    match kind {
        ChartKind::Kpi => 1,
        ChartKind::Table => 2,
        ChartKind::Line => 2,
        ChartKind::Bar => 2,
    }
}

/// Compute-credit contribution of a single executed query.
pub fn query_credits(exec_ms: u64) -> u32 {
    QUERY_BASE + (exec_ms.div_ceil(100)) as u32
}

/// One metered event. `credits` is the contribution of *this* event; a request
/// produces one `DashboardRender` event plus per-query events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeterEvent {
    pub event_id: String, // uuid, idempotency key
    pub occurred_at: chrono::DateTime<chrono::Utc>,
    pub tenant_id: String,
    pub user_sub: String,
    pub dashboard_id: String,
    pub request_id: String,
    pub kind: MeterKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exec_time_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<i64>,
    pub credits: i64,
    pub sc_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeterKind {
    DashboardRender,
    SemanticCompute,
    CacheHit,
    CacheMiss,
}

impl MeterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MeterKind::DashboardRender => "dashboard_render",
            MeterKind::SemanticCompute => "semantic_compute",
            MeterKind::CacheHit => "cache_hit",
            MeterKind::CacheMiss => "cache_miss",
        }
    }
}
