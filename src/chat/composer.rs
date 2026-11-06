//! Local completion; only the final chosen reference is sent with a prompt.
use std::ops::Range;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Menu {
    Slash,
    Files,
    Model,
    Effort,
}
#[derive(Clone, Debug)]
pub struct Choice {
    pub value: String,
    pub label: String,
    pub detail: String,
}
pub const SLASH: &[(&str, &str)] = &[
    ("help", "Show commands and keyboard shortcuts"),
    ("model", "Choose the model for this conversation"),
    ("effort", "Choose reasoning effort"),
    ("compact", "Compact context, optionally with instructions"),
    ("new", "Start a new chat; keep this conversation"),
    ("status", "Show connection and conversation details"),
    ("stop", "Interrupt the current turn"),
];
pub fn trigger(text: &str, cursor: usize) -> Option<(Menu, Range<usize>, String)> {
    if cursor > text.len() || !text.is_char_boundary(cursor) {
        return None;
    }
    let before = &text[..cursor];
    if before.starts_with('/') && !before.contains(char::is_whitespace) {
        return Some((Menu::Slash, 0..cursor, before[1..].into()));
    }
    let start = before
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(0, |(i, c)| i + c.len_utf8());
    let token = &before[start..];
    if let Some(query) = token.strip_prefix('@').filter(|s| !s.contains('"')) {
        let end = text[cursor..]
            .find(char::is_whitespace)
            .map_or(text.len(), |i| cursor + i);
        return Some((Menu::Files, start..end, query.into()));
    }
    None
}
pub fn file_reference(path: &str) -> String {
    if path
        .chars()
        .any(|c| c.is_whitespace() || c == '"' || c == '\\')
    {
        format!("@\"{}\"", path.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        format!("@{path}")
    }
}
pub fn files(files: &[String], query: &str) -> Vec<Choice> {
    let q = query.to_lowercase();
    let mut matches: Vec<_> = files
        .iter()
        .filter(|p| p.to_lowercase().contains(&q))
        .collect();
    matches.sort_by_key(|p| (!p.to_lowercase().starts_with(&q), p.len()));
    matches
        .into_iter()
        .take(8)
        .map(|p| Choice {
            value: p.clone(),
            label: p.rsplit('/').next().unwrap_or(p).into(),
            detail: p
                .rsplit_once('/')
                .map(|(d, _)| d.to_owned())
                .unwrap_or_else(|| "Project root".into()),
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mentions_replace_the_whole_token_and_preserve_unicode() {
        let t = "看看 @src/文.rs 后面";
        let pos = t.find(".rs").unwrap();
        let (kind, r, q) = trigger(t, pos).unwrap();
        assert_eq!(kind, Menu::Files);
        assert_eq!(&t[r], "@src/文.rs");
        assert_eq!(q, "src/文");
        assert!(trigger("me@example.com", 14).is_none());
        assert_eq!(file_reference("中文 file.rs"), "@\"中文 file.rs\"");
    }
    #[test]
    fn command_arguments_are_not_completed_as_command_names() {
        assert_eq!(trigger("/mod", 4).unwrap().0, Menu::Slash);
        assert!(trigger("/compact focus", 14).is_none());
        assert!(trigger("中文", 1).is_none());
    }
}
