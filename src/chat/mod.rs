//! Native editing and history, backed by local/remote execution services.
mod attachments;
mod composer;
pub(crate) mod decisions;
mod presentation;
mod state;
mod status_line;
mod transport;
mod view;
use serde::{Deserialize, Serialize};
pub use view::{ChatEvent, ChatView};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatConfig {
    pub id: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub folder: String,
    #[serde(default = "model")]
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<String>,
}
fn model() -> String {
    "opus".into()
}
pub fn fresh_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{:032x}",
        t + u128::from(NEXT.fetch_add(1, Ordering::Relaxed))
    )
}
pub fn install(host: &str) -> Result<(), String> {
    transport::install(host)
}

pub const SUPER_ID: &str = "73757065726167656e74000000000001";
