//! Herdr-owned OS notifications, including a windowless sender for the login clock.
use gpui::{SystemNotification, SystemNotificationAction};
use serde_json::Value;

pub fn notification(value: &Value) -> Result<SystemNotification, String> {
    let id = value["id"].as_str().unwrap_or_default();
    let title = value["title"].as_str().unwrap_or_default();
    let text = value["text"].as_str().unwrap_or_default();
    if id.is_empty()
        || id.len() > 180
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-@".contains(&c))
        || title.is_empty()
        || title.chars().count() > 160
        || text.chars().count() > 1000
    {
        return Err("Invalid reminder notification".into());
    }
    Ok(SystemNotification {
        tag: format!("herdr-reminder:{id}").into(),
        title: title.to_owned().into(),
        body: text.to_owned().into(),
        actions: vec![SystemNotificationAction {
            id: "open".into(),
            label: "查看提醒".into(),
        }],
    })
}

pub fn cli(value: Value) -> i32 {
    let notification = match notification(&value) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    // The running app owns the notification center, its app name and its icon.
    if crate::control::client(&[
        "do".into(),
        "supervisor.reminder_notify".into(),
        value.to_string(),
    ]) == 0
    {
        return 0;
    }
    #[cfg(target_os = "macos")]
    {
        use gpui::*;
        let Ok(exe) = std::env::current_exe() else {
            return 1;
        };
        let bundle = exe
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent());
        if !bundle.is_some_and(|p| {
            p.extension().is_some_and(|s| s == "app") && p.join("Contents/Info.plist").exists()
        }) {
            eprintln!("Use the installed Herdr App bundle for native notifications");
            return 1;
        }
        // No workspace, core, model or window is started by this sender.
        gpui_platform::application().run(move |cx| {
            cx.set_activation_policy(ActivationPolicy::Accessory);
            cx.on_system_notification_response(|_, cx| {
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe).spawn();
                }
                cx.quit();
            });
            cx.show_system_notification(notification);
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(5))
                    .await;
                let _ = cx.update(|cx| cx.quit());
            })
            .detach();
        });
        0
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = notification;
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn notification_identity_is_stable_and_actions_only_open_the_app() {
        let v =
            json!({"id":"job_test@1793926800000","title":"审批提醒","text":"审批与 /apply 是两步"});
        let n = notification(&v).unwrap();
        assert_eq!(n, notification(&v).unwrap());
        assert_eq!(n.tag.as_ref(), "herdr-reminder:job_test@1793926800000");
        assert_eq!(n.actions.len(), 1);
        assert_eq!(n.actions[0].id.as_ref(), "open");
        assert!(notification(&json!({"id":"bad/id","title":"x","text":"x"})).is_err());
    }
}
