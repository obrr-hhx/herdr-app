//! Presentation of native CLI usage facts. Missing values remain unknown.
use serde_json::Value;

fn percent(v: &Value) -> String {
    v.as_f64()
        .filter(|n| n.is_finite() && *n >= 0. && *n <= 100.)
        .map(|n| format!("{n:.0}%"))
        .unwrap_or_else(|| "—".into())
}
fn duration(seconds: u64) -> String {
    let minutes = seconds.saturating_add(59) / 60;
    if minutes >= 1440 {
        format!("{}d {}h", minutes / 1440, minutes % 1440 / 60)
    } else if minutes >= 60 {
        format!("{}h {}m", minutes / 60, minutes % 60)
    } else {
        format!("{minutes}m")
    }
}
fn quota(data: &Value, name: &str, now: u64) -> (String, bool) {
    let mut label = format!("{name} used {}", percent(&data["used_pct"]));
    if let Some(reset) = data["resets_at"]
        .as_f64()
        .filter(|n| n.is_finite() && *n >= 0.)
    {
        if reset as u64 > now {
            label.push_str(&format!(" · resets in {}", duration(reset as u64 - now)));
        } else {
            label.push_str(" · reset due");
        }
    }
    (label, data["used_pct"].as_f64().is_some_and(|n| n >= 80.))
}
pub fn segments(data: &Value, fallback_model: &str, now: u64) -> Vec<(String, bool)> {
    let model = data["model"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_model);
    let mut out = vec![(model.to_owned(), false)];
    if let Some(branch) = data["branch"].as_str().filter(|s| !s.is_empty()) {
        out.push((branch.to_owned(), false));
    }
    out.push((
        format!("Context {}", percent(&data["context_pct"])),
        data["context_pct"].as_f64().is_some_and(|n| n >= 80.),
    ));
    out.push(quota(&data["five_hour"], "5h", now));
    out.push(quota(&data["seven_day"], "7d", now));
    if let Some(cost) = data["cost_usd"]
        .as_f64()
        .filter(|n| n.is_finite() && *n >= 0.)
    {
        out.push((format!("Session ${cost:.2} est."), false));
    }
    out
}
pub fn details(data: &Value, now: u64) -> String {
    let tokens = |key: &str| {
        data[key]
            .as_u64()
            .map(|n| n.to_string())
            .unwrap_or_else(|| "—".into())
    };
    let mut info = format!(
        "Context tokens: {} in / {} out · window {} · cache read {}",
        tokens("input_tokens"),
        tokens("output_tokens"),
        tokens("context_size"),
        tokens("cache_read_tokens")
    );
    if let Some(ms) = data["duration_ms"].as_u64() {
        info.push_str(&format!("\nSession time {}", duration(ms / 1000)));
    }
    if let Some(added) = data["lines_added"].as_u64() {
        info.push_str(&format!(
            " · changes +{added} / −{}",
            tokens("lines_removed")
        ));
    }
    if let Some(at) = data["updated_at"].as_u64() {
        let age = now.saturating_sub(at / 1000);
        let label = if age < 60 {
            format!("{age}s")
        } else {
            duration(age)
        };
        info.push_str(&format!(" · updated {label} ago"));
    }
    info.push_str("\nUsage comes from Claude Code's status line. Session cost is a list-price estimate, not an additional Pro subscription charge.");
    info
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn missing_quota_never_looks_like_zero_usage() {
        let labels = segments(&Value::Null, "Opus", 1000);
        assert!(labels.iter().any(|(s, _)| s == "5h used —"));
        assert!(!labels.iter().any(|(s, _)| s.contains("$0")));
    }
    #[test]
    fn native_zero_usage_resets_and_cost_are_preserved() {
        let data = json!({"model":"Opus 5.5","context_pct":12,"five_hour":{"used_pct":0,"resets_at":4600},"seven_day":{"used_pct":89,"resets_at":2000},"cost_usd":0.27});
        let labels = segments(&data, "opus", 1000);
        assert!(labels.contains(&("5h used 0% · resets in 1h 0m".into(), false)));
        assert!(labels.contains(&("7d used 89% · resets in 17m".into(), true)));
        assert!(labels.iter().any(|(s, _)| s == "Session $0.27 est."));
        assert!(details(&data, 1000).contains("not an additional Pro"));
    }
}
