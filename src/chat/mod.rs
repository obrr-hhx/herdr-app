//! Native conversation client. All execution stays in the remote interactive Claude service.
mod state;
mod transport;
mod view;
use serde::{Deserialize, Serialize};
pub use view::ChatView;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatConfig {
    pub id: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub folder: String,
    #[serde(default = "model")]
    pub model: String,
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
