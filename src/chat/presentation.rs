//! Presentation-only grouping. Transcript messages and tool IDs stay unchanged.
use super::state::Message;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryRow {
    pub start: usize,
    pub end: usize,
    pub tools: bool,
}
pub fn is_tool(message: &Message) -> bool {
    !matches!(
        message.role.as_str(),
        "You" | "Claude" | "Notice" | "Reminder"
    )
}
pub fn rows(messages: &[Message]) -> Vec<HistoryRow> {
    let mut rows: Vec<HistoryRow> = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        let tools = is_tool(message);
        if tools && let Some(last) = rows.last_mut().filter(|r| r.tools) {
            last.end = index + 1;
        } else {
            rows.push(HistoryRow {
                start: index,
                end: index + 1,
                tools,
            });
        }
    }
    rows
}
/// Describe observed operations, without attributing success to missing results.
pub fn operations(messages: &[Message]) -> String {
    let mut counts = [0usize; 6];
    for m in messages.iter().filter(|m| !m.id.starts_with("result:")) {
        let kind = match m.role.as_str() {
            "Read" | "read_file" => 0,
            "Bash" | "exec_command" | "shell_command" => 1,
            "Edit" | "Write" | "apply_patch" => 2,
            "Glob" | "Grep" | "search" => 3,
            "TodoWrite" | "update_plan" => 4,
            _ => 5,
        };
        counts[kind] += 1;
    }
    let names = [
        "读取文件",
        "运行命令",
        "编辑文件",
        "搜索",
        "更新计划",
        "其他操作",
    ];
    counts
        .iter()
        .zip(names)
        .filter(|(n, _)| **n > 0)
        .map(|(n, name)| format!("{name} {n}"))
        .collect::<Vec<_>>()
        .join(" · ")
}
#[derive(Debug, PartialEq, Eq)]
pub struct PlanStep {
    pub text: String,
    pub status: String,
}
/// Only display explicit plan data emitted by the worker, never infer a plan.
pub fn plan(messages: &[Message]) -> Option<(String, Vec<PlanStep>)> {
    messages.iter().rev().find_map(|m| {
        let field = match m.role.as_str() {
            "TodoWrite" => "todos",
            "update_plan" => "plan",
            _ => return None,
        };
        let value: Value = serde_json::from_str(&m.text).ok()?;
        let steps = value[field]
            .as_array()?
            .iter()
            .filter_map(|s| {
                Some(PlanStep {
                    text: s
                        .get("content")
                        .or_else(|| s.get("step"))?
                        .as_str()?
                        .to_owned(),
                    status: s["status"].as_str().unwrap_or("pending").to_owned(),
                })
            })
            .collect::<Vec<_>>();
        if steps.is_empty() {
            None
        } else {
            Some((m.id.clone(), steps))
        }
    })
}
pub fn summary(message: &Message) -> String {
    let parsed = serde_json::from_str::<Value>(&message.text).ok();
    let fallback = match message.role.as_str() {
        "inspect_sessions" => "检查所有会话的进展".into(),
        "list_sessions" => "读取会话与任务台账".into(),
        "plan_followup" => "安排下一次主动跟进".into(),
        "record_task" => "更新任务进度".into(),
        "get_checkin_settings" => "读取主动跟进设置".into(),
        "Bash" => "运行命令".into(),
        "Read" => "读取文件".into(),
        "Tool result" | "Tool error" => format!("执行结果 · {} 行", message.text.lines().count()),
        name => format!("调用 {name}"),
    };
    if matches!(
        message.role.as_str(),
        "plan_followup" | "record_task" | "get_checkin_settings"
    ) {
        return fallback;
    }
    let text = parsed
        .as_ref()
        .and_then(|v| {
            [
                "description",
                "title",
                "command",
                "file_path",
                "pattern",
                "address",
                "path",
                "reason",
            ]
            .iter()
            .find_map(|key| v.get(*key).and_then(Value::as_str))
        })
        .unwrap_or_else(|| {
            if parsed.is_none() {
                message
                    .text
                    .lines()
                    .find(|s| !s.trim().is_empty())
                    .unwrap_or(&fallback)
            } else {
                &fallback
            }
        });
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(150)
        .collect()
}
/// Keep shell/file requests readable while retaining the raw transcript for copying.
pub fn input_detail(message: &Message) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(&message.text)
        && let Some(command) = v.get("command").and_then(Value::as_str)
    {
        return format!("```sh\n{command}\n```");
    }
    format!("```json\n{}\n```", message.text)
}
pub fn entries(messages: &[Message]) -> Vec<usize> {
    let ids = messages
        .iter()
        .map(|m| m.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    messages
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.id.strip_prefix("result:")
                .is_none_or(|id| !ids.contains(id))
        })
        .map(|(i, _)| i)
        .collect()
}
pub fn elapsed(message: &Message, result: Option<&Message>, now: u64) -> Option<String> {
    if message.at == 0 || message.id.starts_with("result:") || result.is_some_and(|r| r.at == 0) {
        return None;
    }
    let end = result.map(|r| r.at).filter(|t| *t > 0).unwrap_or(now);
    let seconds = end.saturating_sub(message.at) / 1000;
    Some(if seconds < 60 {
        format!("{seconds}秒")
    } else {
        format!("{}分{}秒", seconds / 60, seconds % 60)
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn message(id: &str, role: &str) -> Message {
        Message {
            id: id.into(),
            role: role.into(),
            ..Default::default()
        }
    }
    #[test]
    fn operation_counts_exclude_results_and_plan_uses_only_explicit_steps() {
        let mut messages = vec![
            message("r", "Read"),
            message("b", "Bash"),
            message("e", "Edit"),
            message("result:b", "Tool error"),
        ];
        assert_eq!(
            operations(&messages),
            "读取文件 1 · 运行命令 1 · 编辑文件 1"
        );
        assert!(plan(&messages).is_none());
        let mut first = message("p1", "TodoWrite");
        first.text = r#"{"todos":[{"content":"读取配置","status":"completed"}]}"#.into();
        messages.push(first);
        let mut last = message("p2", "update_plan");
        last.text =
            r#"{"plan":[{"step":"验证修复","status":"in_progress"},{"bad":"ignored"}]}"#.into();
        messages.push(last);
        let (id, steps) = plan(&messages).unwrap();
        assert_eq!(id, "p2");
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].text, "验证修复");
        assert_eq!(steps[0].status, "in_progress");
        assert!(
            plan(&[Message {
                role: "Bash".into(),
                text: r#"{"todos":[{"content":"fake"}]}"#.into(),
                ..Default::default()
            }])
            .is_none()
        );
        let mut followup = message("f", "plan_followup");
        followup.text = r#"{"reason":"internal diagnostic detail","after_minutes":null}"#.into();
        assert_eq!(summary(&followup), "安排下一次主动跟进");
    }
    #[test]
    fn calls_and_results_group_without_crossing_human_or_assistant_text() {
        let messages = vec![
            message("a", "You"),
            message("t1", "Bash"),
            message("result:t1", "Tool result"),
            message("t2", "Read"),
            message("result:t2", "Tool error"),
            message("reply", "Claude"),
            message("t3", "Bash"),
        ];
        assert_eq!(
            rows(&messages),
            vec![
                HistoryRow {
                    start: 0,
                    end: 1,
                    tools: false
                },
                HistoryRow {
                    start: 1,
                    end: 5,
                    tools: true
                },
                HistoryRow {
                    start: 5,
                    end: 6,
                    tools: false
                },
                HistoryRow {
                    start: 6,
                    end: 7,
                    tools: true
                }
            ]
        );
        assert_eq!(messages.len(), 7);
    }
    #[test]
    fn incoming_results_keep_the_group_anchor_and_descriptions_are_readable() {
        let mut messages = vec![message("t1", "Bash")];
        let before = rows(&messages);
        messages.push(message("result:t1", "Tool result"));
        assert_eq!(rows(&messages)[0].start, before[0].start);
        messages[0].text =
            r#"{"description":"Check deployment status","command":"sensitive detail"}"#.into();
        assert_eq!(summary(&messages[0]), "Check deployment status");
    }
    #[test]
    fn orphan_results_are_retained_and_elapsed_time_freezes_at_completion() {
        let mut call = message("a", "Bash");
        call.at = 1000;
        let mut result = message("result:a", "Tool result");
        result.at = 28000;
        assert_eq!(elapsed(&call, None, 27000).as_deref(), Some("26秒"));
        assert_eq!(
            elapsed(&call, Some(&result), 99000).as_deref(),
            Some("27秒")
        );
        let messages = vec![message("result:earlier", "Tool error"), call, result];
        assert_eq!(entries(&messages), vec![0, 1]);
        assert_eq!(
            summary(&message("inspect", "inspect_sessions")),
            "检查所有会话的进展"
        );
    }
}
