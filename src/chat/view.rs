mod decision_sidebar;
use super::attachments;
use super::composer::{self, Choice, Menu};
use super::presentation::{self, HistoryRow};
use super::{
    ChatConfig, fresh_id,
    state::Conversation,
    transport::{self, Connection},
};
use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_base::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_base::text::TextView;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
fn color(name: &str) -> Hsla {
    crate::settings::color_or_literal(match name {
        "bg" => "surface",
        "text" => "foreground",
        other => other,
    })
}

#[cfg(test)]
fn sync_history_list(
    list: &ListState,
    old: usize,
    new: usize,
    dirty: impl IntoIterator<Item = usize>,
) {
    if new > old {
        list.splice(old..old, new - old);
    }
    for row in dirty {
        if row < old {
            list.remeasure_items(row..row + 1);
        }
    }
}

fn sync_presentation_list(
    list: &ListState,
    previous: &[HistoryRow],
    next: &[HistoryRow],
    dirty: &std::collections::BTreeSet<usize>,
) {
    let common = previous
        .iter()
        .zip(next)
        .take_while(|(a, b)| a.start == b.start && a.tools == b.tools)
        .count();
    if common < previous.len() || next.len() < previous.len() {
        list.splice(common..previous.len(), next.len() - common);
    } else if next.len() > previous.len() {
        list.splice(previous.len()..previous.len(), next.len() - previous.len());
    }
    for (index, row) in next.iter().enumerate().take(common) {
        if previous[index] != *row || dirty.iter().any(|m| *m >= row.start && *m < row.end) {
            list.remeasure_items(index..index + 1);
        }
    }
}
pub enum ChatEvent {
    OpenDecisionSource {
        address: String,
    },
    NewConversation {
        host: String,
        folder: String,
        model: String,
    },
}
impl EventEmitter<ChatEvent> for ChatView {}
pub struct ChatView {
    config: ChatConfig,
    host: Entity<InputState>,
    folder: Entity<InputState>,
    editor: Entity<TextareaState>,
    conversation: Conversation,
    list: ListState,
    rows: Vec<HistoryRow>,
    pane_height: std::rc::Rc<std::cell::Cell<f32>>,
    pane_width: std::rc::Rc<std::cell::Cell<f32>>,
    decision_rows: Vec<Value>,
    decision_error: String,
    decision_open: Option<bool>,
    decision_completed: bool,
    decision_scroll: ScrollHandle,
    decision_inputs: HashMap<String, Entity<InputState>>,
    decision_choices: HashMap<String, String>,
    connection: Option<Connection>,
    expanded: HashSet<String>,
    answers: HashMap<String, HashMap<String, String>>,
    decisions_in_flight: HashSet<String>,
    history_ok: bool,
    free_answers: HashMap<(String, String), Entity<InputState>>,
    installing: bool,
    notice: String,
    menu: Option<Menu>,
    menu_scroll: ScrollHandle,
    menu_query: String,
    menu_index: usize,
    workspace_requested: bool,
    workspace_request: String,
    requested_query: String,
    connection_open: bool,
    status_expanded: bool,
    editor_max_rows: usize,
    draft_images: Vec<String>,
    preview_image: Option<String>,
    preview_focus: FocusHandle,
    _input: Subscription,
}
impl ChatView {
    pub fn new(mut config: ChatConfig, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dir = transport::local_dir(&config.id);
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(bytes) = std::fs::read(dir.join("connection.json")) {
            if let Ok(saved) = serde_json::from_slice::<ChatConfig>(&bytes) {
                if saved.id == config.id {
                    config = saved;
                }
            }
        }
        let style = gpui_base::input::InputEditorStyle {
            foreground: color("text"),
            muted_foreground: color("dim"),
            selection: color("accent").opacity(0.3),
            caret: color("text"),
            ..Default::default()
        };
        let host = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("SSH host, e.g. forge-vm")
                .default_value(config.host.clone())
        });
        let folder = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Absolute folder on remote machine")
                .default_value(config.folder.clone())
        });
        let draft = std::fs::read_to_string(dir.join("draft.txt")).unwrap_or_default();
        let editor = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 6)
                .submit_on_enter(true)
                .placeholder("Ask anything, @ to add files, / for commands")
                .default_value(draft)
        });
        host.update(cx, |s, _| s.set_editor_style(style.clone()));
        folder.update(cx, |s, _| s.set_editor_style(style.clone()));
        editor.update(cx, |s, _| s.set_editor_style(style));
        let input = cx.subscribe_in(&editor, window, |this, _, e: &InputEvent, window, cx| {
            if matches!(e, InputEvent::Change) {
                let value = this.editor.read(cx).value();
                let cursor = this.editor.read(cx).cursor();
                if let Some((menu, _range, query)) = composer::trigger(&value, cursor) {
                    this.menu = Some(menu);
                    this.menu_query = query;
                    this.menu_index = 0;
                    this.load_workspace(cx);
                    this.search_workspace(cx);
                } else if !(matches!(this.menu, Some(Menu::Model | Menu::Effort))
                    && value.trim().is_empty())
                {
                    this.menu = None;
                }
                cx.notify();
                let text = this.editor.read(cx).value();
                let _ = std::fs::write(
                    transport::local_dir(&this.config.id).join("draft.txt"),
                    text.as_bytes(),
                );
            }
            if matches!(
                e,
                InputEvent::PressEnter {
                    secondary: false,
                    shift: false
                }
            ) {
                if this.menu.is_some() && !this.choices().is_empty() {
                    this.accept_choice(this.menu_index, window, cx);
                } else {
                    this.send(window, cx);
                }
            }
        });
        let mut conversation = Conversation::default();
        let history = transport::load_history(&dir.join("events.jsonl"));
        let cache_error = history.is_err();
        if let Ok(history) = history {
            for e in history {
                conversation.apply(&e);
            }
        }
        if let Ok(bytes) = std::fs::read(dir.join("outbox.json")) {
            if let Ok(pending) =
                serde_json::from_slice::<std::collections::BTreeMap<String, Value>>(&bytes)
            {
                for (id, command) in pending {
                    if command["action"] == "send" {
                        if let Some(text) = command["text"].as_str() {
                            conversation.apply(&json!({"type":"user","id":id,"text":text,"images":command["images"]}));
                        }
                    }
                }
            }
        }
        conversation.take_dirty_rows();
        let rows = presentation::rows(&conversation.messages);
        let list = ListState::new(rows.len(), ListAlignment::Bottom, px(1000.));
        list.set_follow_mode(FollowMode::Tail);
        let observer = cx.entity().downgrade();
        list.set_scroll_handler(move |_, _, cx| {
            let _ = observer.update(cx, |_, cx| cx.notify());
        });
        let mut this = Self {
            config,
            host,
            folder,
            editor,
            conversation,
            list,
            rows,
            pane_height: std::rc::Rc::new(std::cell::Cell::new(0.)),
            pane_width: std::rc::Rc::new(std::cell::Cell::new(0.)),
            decision_rows: super::decisions::read().unwrap_or_default(),
            decision_error: String::new(),
            decision_open: None,
            decision_completed: false,
            decision_scroll: ScrollHandle::new(),
            decision_inputs: HashMap::new(),
            decision_choices: HashMap::new(),
            connection: None,
            expanded: HashSet::new(),
            answers: HashMap::new(),
            decisions_in_flight: HashSet::new(),
            history_ok: !cache_error,
            free_answers: HashMap::new(),
            installing: false,
            notice: String::new(),
            menu: None,
            menu_scroll: ScrollHandle::new(),
            menu_query: String::new(),
            menu_index: 0,
            workspace_requested: false,
            workspace_request: String::new(),
            requested_query: String::new(),
            connection_open: false,
            status_expanded: false,
            editor_max_rows: 6,
            draft_images: attachments::load_draft(&dir),
            preview_image: None,
            preview_focus: cx.focus_handle(),
            _input: input,
        };
        if this.config.id == super::SUPER_ID {
            cx.spawn(async move |view, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(2)).await;
                    if view
                        .update(cx, |this, cx| match super::decisions::read() {
                            Ok(rows) if rows != this.decision_rows => {
                                this.decision_rows = rows;
                                this.decision_error.clear();
                                cx.notify();
                            }
                            Ok(_) => {}
                            Err(e) if e != this.decision_error => {
                                this.decision_error = e;
                                cx.notify();
                            }
                            Err(_) => {}
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        }
        if cache_error {
            this.notice="Saved history is damaged. Keep this chat for recovery and open a new chat; nothing was overwritten.".into();
        }
        if this.valid() && !cache_error {
            this.connect(cx);
        }
        this
    }
    fn paste_images(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let mut sources = Vec::new();
        for entry in item.entries {
            match entry {
                ClipboardEntry::Image(image) => sources.push(Ok(image.bytes)),
                ClipboardEntry::ExternalPaths(paths) => {
                    for p in paths.0 {
                        if p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                            ["png", "jpg", "jpeg", "tiff", "tif", "bmp"]
                                .contains(&e.to_ascii_lowercase().as_str())
                        }) {
                            sources.push(
                                if std::fs::metadata(&p)
                                    .is_ok_and(|m| m.len() <= attachments::MAX_SOURCE_BYTES)
                                {
                                    std::fs::read(p)
                                        .map_err(|_| "Cannot read the copied image.".to_owned())
                                } else {
                                    Err("Source image exceeds 64 MB or cannot be read.".into())
                                },
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        if sources.is_empty() {
            return false;
        }
        for source in sources {
            if self.draft_images.len() >= attachments::MAX_IMAGES {
                self.notice = "Up to 4 images per message. Existing attachments are kept.".into();
                break;
            }
            match source
                .and_then(|bytes| attachments::save(&transport::local_dir(&self.config.id), &bytes))
            {
                Ok(id) => {
                    if !self.draft_images.contains(&id) {
                        self.draft_images.push(id);
                    }
                    self.notice.clear();
                }
                Err(message) => self.notice = message,
            }
        }
        attachments::save_draft(&transport::local_dir(&self.config.id), &self.draft_images);
        cx.notify();
        true
    }
    fn image_strip(&self, ids: &[String], removable: bool, cx: &mut Context<Self>) -> AnyElement {
        let mut row = div()
            .id("images-strip")
            .w_full()
            .min_w_0()
            .flex()
            .gap_2()
            .max_w_full();
        for id in ids {
            let id = id.clone();
            let preview = id.clone();
            let remove = id.clone();
            let path = attachments::path(&transport::local_dir(&self.config.id), &id);
            row = row.child(
                div()
                    .id(SharedString::from(format!("image-{removable}-{id}")))
                    .flex_1()
                    .min_w_0()
                    .max_w(px(90.))
                    .relative()
                    .rounded_md()
                    .border_1()
                    .border_color(color("border"))
                    .child(
                        img(path)
                            .w_full()
                            .h(px(if removable && self.status_expanded {
                                48.
                            } else {
                                64.
                            }))
                            .object_fit(ObjectFit::Contain)
                            .cursor_pointer(),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.preview_image = Some(preview.clone());
                        this.preview_focus.focus(window, cx);
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .when(removable, |el| {
                        el.child(
                            div()
                                .id(SharedString::from(format!("remove-{id}")))
                                .absolute()
                                .top_0()
                                .right_0()
                                .px_1()
                                .bg(color("bg"))
                                .rounded_full()
                                .cursor_pointer()
                                .child("×")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.draft_images.retain(|s| s != &remove);
                                    attachments::save_draft(
                                        &transport::local_dir(&this.config.id),
                                        &this.draft_images,
                                    );
                                    cx.stop_propagation();
                                    cx.notify();
                                })),
                        )
                    }),
            );
        }
        row.into_any_element()
    }
    fn valid(&self) -> bool {
        transport::valid_host(&self.config.host)
            && self.config.folder.starts_with('/')
            && self.config.id.bytes().all(|b| b.is_ascii_hexdigit())
    }
    fn save_config(&self) {
        let dir = transport::local_dir(&self.config.id);
        if let Ok(text) = serde_json::to_vec_pretty(&self.config) {
            let _ = std::fs::write(dir.join("connection.json"), text);
        }
    }
    fn connect(&mut self, cx: &mut Context<Self>) {
        if !self.history_ok {
            self.notice = "Saved history needs recovery; open a new chat to continue.".into();
            cx.notify();
            return;
        }
        if !self.valid() {
            self.notice = "Choose an SSH host and an absolute remote folder.".into();
            cx.notify();
            return;
        }
        self.save_config();
        self.connection = None;
        let (connection, events) = Connection::start(self.config.clone(), self.conversation.cursor);
        self.connection = Some(connection);
        cx.spawn(async move |view, cx| {
            while let Ok(first) = events.recv().await {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let mut batch = vec![first];
                while batch.len() < 256 {
                    match events.try_recv() {
                        Ok(e) => batch.push(e),
                        Err(_) => break,
                    }
                }
                if view
                    .update(cx, |this, cx| {
                        let mut changed = false;
                        for e in batch {
                            if e["type"] == "reminder_ack" {
                                if let Some(id) = e["id"].as_str() {
                                    cx.dismiss_system_notification(&format!("herdr-reminder:{id}"));
                                }
                            }
                            if e["type"] == "workspace"
                                && e["request_id"].as_str() != Some(this.workspace_request.as_str())
                            {
                                continue;
                            }
                            if e["type"] == "prompt_confirmed" || e["type"] == "permission_resolved"
                            {
                                this.notice.clear();
                            }
                            if e["type"] == "settings" && e["applied"] == true {
                                this.notice.clear();
                            }
                            if e["type"] == "connected"
                                && !this.conversation.workspace_error.is_empty()
                            {
                                this.workspace_requested = false;
                            }
                            if e["type"] == "upload" {
                                this.notice = e["message"].as_str().unwrap_or_default().into();
                            }
                            if e["type"] == "permission_resolved" {
                                if let Some(id) = e["id"].as_str() {
                                    this.decisions_in_flight.remove(id);
                                }
                            }
                            if e["type"] == "command_error" {
                                this.decisions_in_flight.clear();
                            }
                            changed |= this.conversation.apply(&e);
                        }
                        if this
                            .conversation
                            .capabilities
                            .iter()
                            .any(|c| c == "workspace")
                            && !this.workspace_requested
                        {
                            this.load_workspace(cx);
                        }
                        this.search_workspace(cx);
                        this.menu_index =
                            this.menu_index.min(this.choices().len().saturating_sub(1));
                        if changed {
                            this.sync_rows();
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }
    fn configure(&mut self, cx: &mut Context<Self>) {
        let host = self.host.read(cx).value().trim().to_owned();
        let folder = self.folder.read(cx).value().trim().to_owned();
        if self.conversation.cursor > 0
            && (host != self.config.host || folder != self.config.folder)
        {
            self.notice="Open a new chat to use a different machine or folder; this history stays with its original target.".into();
            cx.notify();
            return;
        }
        self.config.host = host;
        self.config.folder = folder;
        self.connect(cx);
    }
    fn command(&mut self, mut value: Value, cx: &mut Context<Self>) -> bool {
        value["request_id"] = json!(fresh_id());
        let id = value["request_id"].as_str().unwrap_or_default().to_owned();
        let text = value["text"].as_str().map(str::to_owned);
        let images = value["images"].clone();
        let sent = self.connection.as_ref().is_some_and(|c| c.send(value));
        if sent {
            if let Some(text) = text {
                self.conversation
                    .apply(&json!({"type":"user","id":id,"text":text,"images":images}));
                self.sync_rows();
            }
        }
        if !sent {
            self.notice = "Connect to the remote machine first. Your draft is kept.".into();
            cx.notify();
        }
        sent
    }
    fn request_workspace(&mut self, query: String, cx: &mut Context<Self>) {
        let id = fresh_id();
        if self
            .connection
            .as_ref()
            .is_some_and(|c| c.send(json!({"action":"workspace","request_id":id,"query":query})))
        {
            self.workspace_requested = true;
            self.workspace_request = id;
            self.requested_query = query;
        }
        cx.notify();
    }
    fn load_workspace(&mut self, cx: &mut Context<Self>) {
        if self.workspace_requested
            || !self
                .conversation
                .capabilities
                .iter()
                .any(|c| c == "workspace")
        {
            return;
        }
        self.request_workspace(String::new(), cx);
    }
    fn search_workspace(&mut self, cx: &mut Context<Self>) {
        if self.menu != Some(Menu::Files)
            || !self.conversation.workspace_loaded
            || !((!self.conversation.files_query.is_empty()
                && !self.menu_query.starts_with(&self.conversation.files_query))
                || (self.conversation.files_truncated && self.choices().is_empty()))
            || self.menu_query == self.requested_query
        {
            return;
        }
        let query = self.menu_query.clone();
        cx.spawn(async move |view, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(180))
                .await;
            let _ = view.update(cx, |this, cx| {
                if this.menu == Some(Menu::Files)
                    && this.menu_query == query
                    && this.requested_query != query
                {
                    this.request_workspace(query, cx);
                }
            });
        })
        .detach();
    }
    fn choices(&self) -> Vec<Choice> {
        match self.menu {
            Some(Menu::Model) if self.conversation.provider == "codex" => self
                .conversation
                .models
                .iter()
                .filter_map(|m| {
                    Some(Choice {
                        value: m["model"].as_str()?.into(),
                        label: m["displayName"]
                            .as_str()
                            .unwrap_or(m["model"].as_str()?)
                            .into(),
                        detail: m["description"].as_str().unwrap_or_default().into(),
                    })
                })
                .collect(),
            Some(Menu::Effort) if self.conversation.provider == "codex" => {
                std::iter::once(Choice {
                    value: "".into(),
                    label: "Default".into(),
                    detail: "Model default".into(),
                })
                .chain(
                    self.conversation
                        .models
                        .iter()
                        .find(|m| m["model"].as_str() == self.conversation.model.as_deref())
                        .into_iter()
                        .flat_map(|m| {
                            m["supportedReasoningEfforts"]
                                .as_array()
                                .into_iter()
                                .flatten()
                        })
                        .filter_map(|e| {
                            Some(Choice {
                                value: e["reasoningEffort"].as_str()?.into(),
                                label: e["reasoningEffort"].as_str()?.into(),
                                detail: e["description"].as_str().unwrap_or_default().into(),
                            })
                        }),
                )
                .collect()
            }
            Some(Menu::Model) => [
                ("opus", "Opus", "Most capable"),
                ("sonnet", "Sonnet", "Balanced"),
                ("haiku", "Haiku", "Fast and lightweight"),
            ]
            .into_iter()
            .map(|(value, label, detail)| Choice {
                value: value.into(),
                label: label.into(),
                detail: detail.into(),
            })
            .collect(),
            Some(Menu::Effort) => [
                ("", "Default"),
                ("low", "Low"),
                ("medium", "Medium"),
                ("high", "High"),
                ("xhigh", "Extra high"),
                ("max", "Maximum"),
            ]
            .into_iter()
            .map(|(value, label)| Choice {
                value: value.into(),
                label: label.into(),
                detail: "Applies to the next turn".into(),
            })
            .collect(),
            Some(Menu::Files) => composer::files(&self.conversation.files, &self.menu_query),
            Some(Menu::Slash) => {
                let mut rows: Vec<Choice> = composer::SLASH
                    .iter()
                    .map(|(v, d)| Choice {
                        value: (*v).into(),
                        label: format!("/{v}"),
                        detail: (*d).into(),
                    })
                    .collect();
                for c in &self.conversation.commands {
                    if let Some(n) = c["name"].as_str() {
                        if !rows.iter().any(|r| r.value == n) {
                            rows.push(Choice {
                                value: n.into(),
                                label: format!("/{n}"),
                                detail: c["description"]
                                    .as_str()
                                    .unwrap_or("Project command")
                                    .into(),
                            });
                        }
                    }
                }
                rows.into_iter()
                    .filter(|c| {
                        c.value
                            .to_lowercase()
                            .contains(&self.menu_query.to_lowercase())
                    })
                    .take(8)
                    .collect()
            }
            None => vec![],
        }
    }
    fn change_runtime(&mut self, model: String, effort: String, cx: &mut Context<Self>) {
        if !self
            .conversation
            .capabilities
            .iter()
            .any(|c| c == "settings")
        {
            self.notice =
                "Update the remote service from Connection to enable model selection.".into();
            cx.notify();
            return;
        }
        if !matches!(self.conversation.status.as_str(), "idle" | "starting")
            || !self.conversation.permissions.is_empty()
        {
            self.notice =
                "Wait for the current turn to finish before changing model or effort.".into();
            cx.notify();
            return;
        }
        if self.command(
            json!({"action":"configure","model":model,"effort":effort}),
            cx,
        ) {
            self.notice = "Applying settings to this conversation…".into();
        }
        cx.notify();
    }
    fn accept_choice(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(choice) = self.choices().get(index).cloned() else {
            return;
        };
        let menu = self.menu.take();
        match menu {
            Some(Menu::Model) => self.change_runtime(
                choice.value,
                if self.conversation.provider == "codex" {
                    String::new()
                } else {
                    self.conversation.effort.clone()
                },
                cx,
            ),
            Some(Menu::Effort) => self.change_runtime(
                self.conversation
                    .model
                    .clone()
                    .unwrap_or_else(|| self.config.model.clone()),
                choice.value,
                cx,
            ),
            Some(Menu::Files) => {
                let value = self.editor.read(cx).value();
                let cursor = self.editor.read(cx).cursor();
                if let Some((Menu::Files, range, _)) = composer::trigger(&value, cursor) {
                    let insertion = format!("{} ", composer::file_reference(&choice.value));
                    self.editor.update(cx, |input, cx| {
                        input.set_selected_range(range, cx);
                        input.replace(insertion, window, cx);
                    });
                }
            }

            Some(Menu::Slash) => {
                self.set_draft(&format!("/{}", choice.value), window, cx);
                self.menu = None;
                self.send(window, cx);
            }
            None => {}
        }
        if !matches!(self.menu, Some(Menu::Model | Menu::Effort)) {
            self.menu = None;
        }
        self.editor.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }
    fn slash(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(command) = text.strip_prefix('/') else {
            return false;
        };
        let (name, args) = command
            .split_once(char::is_whitespace)
            .unwrap_or((command, ""));
        match name {
            "help" => {
                self.menu = None;
                self.notice="/model · /effort · /compact · /new · /status · /stop. Type / to search project skills too. ↑↓ choose · Tab/Enter select · Esc dismiss · Shift+Enter newline.".into();
            }
            "model" => {
                if args.trim().is_empty() {
                    self.menu = Some(Menu::Model);
                    self.menu_index = 0;
                } else {
                    self.change_runtime(args.trim().into(), self.conversation.effort.clone(), cx);
                }
            }
            "effort" => {
                if args.trim().is_empty() {
                    self.menu = Some(Menu::Effort);
                    self.menu_index = 0;
                } else {
                    self.change_runtime(
                        self.conversation
                            .model
                            .clone()
                            .unwrap_or_else(|| self.config.model.clone()),
                        args.trim().into(),
                        cx,
                    );
                }
            }
            "new" | "clear" => {
                cx.emit(ChatEvent::NewConversation {
                    host: self.config.host.clone(),
                    folder: self.config.folder.clone(),
                    model: self
                        .conversation
                        .model
                        .clone()
                        .unwrap_or_else(|| self.config.model.clone()),
                });
            }
            "status" => {
                self.connection_open = true;
                self.notice = format!(
                    "{} · {} messages · {} · {}",
                    self.config.host,
                    self.conversation.messages.len(),
                    self.conversation
                        .model
                        .as_deref()
                        .unwrap_or(&self.config.model),
                    self.conversation.status
                );
            }
            "stop" => {
                self.command(json!({"action":"interrupt"}), cx);
            }
            "compact" => return false,
            n if self.conversation.commands.iter().any(|c| c["name"] == n) => return false,
            _ => {
                self.notice="This command requires Claude's terminal interface. Use / to see commands supported in this chat.".into();
                cx.notify();
                return true;
            }
        }
        self.editor.update(cx, |s, cx| s.set_value("", window, cx));
        let _ = std::fs::write(transport::local_dir(&self.config.id).join("draft.txt"), "");
        cx.notify();
        true
    }
    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).value().to_string();
        if self.installing {
            self.notice = "Wait for the service update; your draft is kept.".into();
            cx.notify();
            return;
        }
        if text.trim().is_empty() && self.draft_images.is_empty() {
            return;
        }
        if !self.draft_images.is_empty() {
            if !self.conversation.capabilities.iter().any(|c| c == "images") {
                self.notice =
                    "Connect or update the remote service to send images. Your draft is kept."
                        .into();
                cx.notify();
                return;
            }
            if text.trim().starts_with('/') {
                self.notice = "Send images with a message, rather than a slash command.".into();
                cx.notify();
                return;
            }
        }
        if self.slash(text.trim(), window, cx) {
            return;
        }
        if text.trim().starts_with("/compact")
            && !self
                .conversation
                .capabilities
                .iter()
                .any(|c| c == "compact")
        {
            self.notice = "Update the remote service before compacting context.".into();
            cx.notify();
            return;
        }
        self.menu = None;
        if self.command(
            json!({"action":"send","text":text,"images":self.draft_images}),
            cx,
        ) {
            self.editor
                .update(cx, |input, cx| input.set_value("", window, cx));
            let _ = std::fs::write(transport::local_dir(&self.config.id).join("draft.txt"), "");
            self.draft_images.clear();
            attachments::save_draft(&transport::local_dir(&self.config.id), &self.draft_images);
            self.list.set_follow_mode(FollowMode::Tail);
            self.notice = "Sending…".into();
            cx.notify();
        }
    }
    /// Background delegation never edits the person's draft or scroll position.
    pub fn delegate(
        &mut self,
        text: &str,
        id: &str,
        cx: &mut Context<Self>,
    ) -> Result<Value, String> {
        if text.trim().is_empty()
            || text.len() > 256 * 1024
            || text
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err("Invalid message".into());
        }
        if !self.editor.read(cx).value().trim().is_empty() || !self.draft_images.is_empty() {
            return Err("User draft in progress; no message sent".into());
        }
        if self.conversation.status != "idle" || !self.conversation.permissions.is_empty() {
            return Err("Session is not idle; no message sent".into());
        }
        let c = json!({"action":"send","request_id":id,"text":text,"images":[]});
        if !self.connection.as_ref().is_some_and(|conn| conn.send(c)) {
            return Err("Could not queue message".into());
        }
        cx.notify();
        Ok(json!({"queued":true,"request_id":id,"confirmed":false}))
    }
    pub fn manager_provider(
        &mut self,
        provider: &str,
        cx: &mut Context<Self>,
    ) -> Result<Value, String> {
        if self.config.id != super::SUPER_ID || !matches!(provider, "codex" | "claude") {
            return Err("Choose Codex or Claude for Super Agent".into());
        }
        if !self.command(json!({"action":"provider","provider":provider}), cx) {
            return Err("Could not queue provider change".into());
        }
        Ok(json!({"queued":true,"provider":provider}))
    }
    pub fn answer_question(
        &mut self,
        id: &str,
        intent: &Value,
        cx: &mut Context<Self>,
    ) -> Result<Value, String> {
        let p = self
            .conversation
            .permissions
            .get(id)
            .ok_or("Question is no longer pending")?;
        let answer = crate::supervisor::question_reply(p, intent)?;
        if !self.command(json!({"action":"permission","id":id,"decision":"allow","answers":{"User reply":answer}}),cx){return Err("Could not queue question reply".into());}
        Ok(json!({"queued":true,"question_id":id}))
    }
    pub fn history(&self, offset: usize, limit: usize) -> Value {
        let messages = &self.conversation.messages;
        let end = messages.len().saturating_sub(offset);
        let start = end.saturating_sub(limit.clamp(1, 40));
        json!({"messages":messages[start..end].iter().map(|m|json!({"id":m.id,"role":m.role,"text":m.text.chars().take(12000).collect::<String>(),"images":m.images})).collect::<Vec<_>>(),"next_offset":(start>0).then_some(offset+end-start),"status":self.conversation.status,"cursor":self.conversation.cursor,"pending_permissions":self.conversation.permissions})
    }
    pub fn agent_word(&self) -> Option<&'static str> {
        (self.conversation.cursor > 0).then(|| self.conversation.agent_word())
    }
    pub fn attention_token(&self) -> String {
        self.conversation
            .permissions
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(":")
    }
    pub fn completion(&self) -> u64 {
        self.conversation.completion
    }
    pub fn latest_visible(&self) -> bool {
        self.list.is_following_tail()
    }
    pub fn has_completed_result(&self) -> bool {
        self.conversation.agent_word() == "idle"
            && self.conversation.status == "idle"
            && self
                .conversation
                .messages
                .iter()
                .any(|m| m.role == "Claude")
    }
    pub fn agent_detail(&self) -> &str {
        &self.conversation.status
    }
    pub fn set_draft(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.editor
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        cx.notify();
    }
    pub fn status(&self, cx: &App) -> Value {
        json!({"history_rows":self.rows.len(),"tool_groups":self.rows.iter().filter(|r|r.tools).count(),"open_tool_groups":self.expanded.iter().filter(|id|id.starts_with("tools:")).count(),"sending_decisions":self.decisions_in_flight,"provider":self.conversation.provider,"host":self.config.host,"folder":self.config.folder,"conversation":self.config.id,"cursor":self.conversation.cursor,"messages":self.conversation.messages.len(),"pending_permissions":self.conversation.permissions.len(),"unread_reminders":self.conversation.reminders.len(),"reminder_ids":self.conversation.reminders.keys().collect::<Vec<_>>(),"status":self.conversation.status,"status_line":self.conversation.status_line,"permission_mode":self.conversation.permission_mode,"activity_phase":self.conversation.phase,"last_progress":self.conversation.last_progress,"draft_bytes":self.editor.read(cx).value().len(),"draft_images":self.draft_images,"image_preview_open":self.preview_image.is_some(),"draft_scroll_y":f32::from(self.editor.read(cx).scroll_offset().y),"draft_visible_rows":self.editor.read(cx).visible_row_range(),"scroll_item":self.list.logical_scroll_top().item_ix,"scroll_offset":f32::from(self.list.logical_scroll_top().offset_in_item),"following":self.list.is_following_tail(),"models":self.conversation.models,"model":self.conversation.model.as_deref().unwrap_or(&self.config.model),"effort":self.conversation.effort,"workspace_files":self.conversation.files.len(),"menu":format!("{:?}",self.menu),"menu_index":self.menu_index,"menu_choices":self.choices().iter().map(|c|c.value.clone()).collect::<Vec<_>>()})
    }
    fn install_bridge(&mut self, cx: &mut Context<Self>) {
        if self.installing {
            return;
        }
        let host = self.host.read(cx).value().trim().to_owned();
        if !transport::valid_host(&host) {
            self.notice = "Choose a valid SSH host first.".into();
            cx.notify();
            return;
        }
        if matches!(
            self.conversation.status.as_str(),
            "working" | "interrupting"
        ) || !self.conversation.permissions.is_empty()
        {
            self.notice = "Finish the current turn or approval before updating.".into();
            cx.notify();
            return;
        }
        self.installing = true;
        self.notice = "Updating this chat's execution service…".into();
        let id = self.config.id.clone();
        cx.spawn(async move |view, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { transport::upgrade(&host, &id) })
                .await;
            let _ = view.update(cx, |this, cx| {
                this.installing = false;
                match result {
                    Ok(()) => {
                        this.workspace_requested = false;
                        this.notice = "Service updated.".into();
                        this.connect(cx);
                    }
                    Err(e) => this.notice = e,
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn permission(&mut self, id: &str, decision: &str, cx: &mut Context<Self>) {
        if self.decisions_in_flight.contains(id) {
            return;
        }
        let mut answers = self.answers.get(id).cloned().unwrap_or_default();
        for ((pid, q), field) in &self.free_answers {
            if pid == id {
                let v = field.read(cx).value();
                let visible = self.expanded.contains(&format!("other-answer:{id}:{q}"))
                    || self.conversation.permissions[id]["input"]["questions"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .find(|item| item["question"].as_str() == Some(q))
                        .is_some_and(|item| {
                            item["options"]
                                .as_array()
                                .is_none_or(|options| options.is_empty())
                        });
                if visible && !v.trim().is_empty() {
                    answers.insert(q.clone(), v.to_string());
                }
            }
        }
        if decision == "allow" {
            if let Some(p) = self.conversation.permissions.get(id) {
                if p["name"] == "AskUserQuestion"
                    && p["input"]["questions"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|q| {
                            answers
                                .get(q["question"].as_str().unwrap_or_default())
                                .is_none_or(|a| a.trim().is_empty())
                        })
                {
                    self.notice = "先选择或填写每个问题的答案。".into();
                    cx.notify();
                    return;
                }
            }
        }
        if self.command(
            json!({"action":"permission","id":id,"decision":decision,"answers":answers}),
            cx,
        ) {
            self.decisions_in_flight.insert(id.to_owned());
            self.notice = "已提交，正在等待会话确认…".into();
        }
    }
    fn sync_rows(&mut self) {
        let next = presentation::rows(&self.conversation.messages);
        let dirty = self.conversation.take_dirty_rows();
        sync_presentation_list(&self.list, &self.rows, &next, &dirty);
        self.rows = next;
    }
    fn activity_dots(id: &str) -> AnyElement {
        let mut dots = div().flex().items_center().gap(px(3.)).w(px(20.));
        for index in 0..3 {
            dots = dots.child(
                div()
                    .size(px(3.))
                    .rounded_full()
                    .bg(color("working"))
                    .with_animation(
                        SharedString::from(format!("tool-pulse:{id}:{index}")),
                        Animation::new(Duration::from_millis(1200))
                            .repeat_synced()
                            .with_max_fps(24.),
                        move |el, phase| {
                            el.opacity(
                                0.25 + 0.75
                                    * ((phase * std::f32::consts::TAU - index as f32 * 0.9).sin()
                                        * 0.5
                                        + 0.5),
                            )
                        },
                    ),
            );
        }
        dots.into_any_element()
    }
    fn tool_group(&self, row_index: usize, row: &HistoryRow, cx: &mut Context<Self>) -> AnyElement {
        let messages = &self.conversation.messages[row.start..row.end];
        let calls = messages
            .iter()
            .filter(|m| !m.id.starts_with("result:"))
            .collect::<Vec<_>>();
        let errors = messages.iter().filter(|m| m.role == "Tool error").count();
        let results = messages
            .iter()
            .filter_map(|m| m.id.strip_prefix("result:").map(|id| (id, m)))
            .collect::<HashMap<_, _>>();
        let pending = calls.iter().any(|m| !results.contains_key(m.id.as_str()));
        let key = format!("tools:{}", messages[0].id);
        let opened = self.expanded.contains(&key);
        let toggle = key.clone();
        let waiting = pending
            && self
                .conversation
                .permissions
                .values()
                .any(|p| p["source"] != "codex_worker");
        let active = pending
            && !waiting
            && self.conversation.status == "working"
            && self
                .conversation
                .messages
                .iter()
                .rposition(|m| m.role == "You")
                .is_none_or(|last| row.start > last);
        let current = calls
            .iter()
            .rev()
            .find(|m| !results.contains_key(m.id.as_str()));
        let single = presentation::entries(messages).len() == 1;
        let label = if active || waiting || single {
            current
                .or_else(|| calls.last())
                .map(|m| presentation::summary(m))
                .unwrap_or_else(|| presentation::summary(&messages[0]))
        } else {
            presentation::operations(messages)
        };
        let start = current.map(|m| m.at).unwrap_or(0);
        // Collapsed activity is part of the transcript, not another chat bubble.
        let mut group = div().w_full().child(
            div()
                .id(SharedString::from(key))
                .debug_selector(|| "chat-tool-group".into())
                .flex()
                .items_center()
                .gap_2()
                .py_2()
                .cursor_pointer()
                .text_color(color("dim"))
                .hover(|s| s.text_color(color("text")))
                .when(active, |el| {
                    el.child(Self::activity_dots(&format!("group:{}", messages[0].id)))
                })
                .child(div().min_w_0().truncate().text_sm().child(label))
                .when(waiting, |el| el.child(div().text_xs().child("等待确认")))
                .when(start > 0 && active, |el| {
                    el.child(
                        div().flex_shrink_0().text_xs().with_animation(
                            SharedString::from(format!("tool-elapsed:{}", messages[0].id)),
                            Animation::new(Duration::from_secs(1))
                                .repeat()
                                .with_max_fps(1.),
                            move |el, _| {
                                let seconds = crate::model::now_ms().saturating_sub(start) / 1000;
                                el.child(if seconds < 60 {
                                    format!("{seconds}秒")
                                } else {
                                    format!("{}分{}秒", seconds / 60, seconds % 60)
                                })
                            },
                        ),
                    )
                })
                .when(errors > 0, |el| {
                    el.child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(color("blocked"))
                            .child(format!("{errors} 个失败")),
                    )
                })
                .child(div().text_xs().child(if opened { "⌄" } else { "›" }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.expanded.remove(&toggle) {
                        this.expanded.insert(toggle.clone());
                    }
                    this.list.remeasure_items(row_index..row_index + 1);
                    cx.notify();
                })),
        );
        if opened {
            let mut items = div()
                .id(SharedString::from(format!("tool-items:{}", messages[0].id)))
                .max_h(px(460.))
                .border_1()
                .border_color(color("border").opacity(0.65))
                .rounded_lg()
                .overflow_y_scroll()
                .flex()
                .flex_col();
            let entries = presentation::entries(messages)
                .into_iter()
                .map(|i| &messages[i]);
            for message in entries {
                let result = results.get(message.id.as_str()).copied();
                let failed =
                    result.is_some_and(|r| r.role == "Tool error") || message.role == "Tool error";
                let item_key = format!("tool:{}", message.id);
                let detail = single || self.expanded.contains(&item_key);
                let item_toggle = item_key.clone();
                let mut entry = div().when(!single, |el| {
                    el.child(
                        div()
                            .id(SharedString::from(item_key))
                            .debug_selector(|| "chat-tool-item".into())
                            .flex()
                            .items_center()
                            .gap_3()
                            .px_4()
                            .py_3()
                            .cursor_pointer()
                            .hover(|s| s.bg(color("input").opacity(0.3)))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(color(if failed { "blocked" } else { "dim" }))
                                    .child(if failed {
                                        "!"
                                    } else if result.is_some() {
                                        "✓"
                                    } else {
                                        "·"
                                    }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .truncate()
                                    .child(presentation::summary(message)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(color("dim"))
                                    .child(message.role.clone()),
                            )
                            .when_some(
                                presentation::elapsed(message, result, crate::model::now_ms()),
                                |el, time| {
                                    el.child(div().text_xs().text_color(color("dim")).child(time))
                                },
                            )
                            .child(div().text_color(color("dim")).child(if detail {
                                "⌄"
                            } else {
                                "›"
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.expanded.remove(&item_toggle) {
                                    this.expanded.insert(item_toggle.clone());
                                }
                                this.list.remeasure_items(row_index..row_index + 1);
                                cx.notify();
                            })),
                    )
                });
                if detail {
                    let copy = match result {
                        Some(r) => format!("{}\n\n{}", message.text, r.text),
                        None => message.text.clone(),
                    };
                    let mut body = div()
                        .id(SharedString::from(format!("tool-detail:{}", message.id)))
                        .debug_selector(|| "chat-tool-detail".into())
                        .max_h(px(300.))
                        .overflow_y_scroll()
                        .px_4()
                        .pb_4()
                        .when(single, |el| el.pt_3())
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(div().text_xs().text_color(color("dim")).child("输入"))
                        .child(
                            TextView::markdown(
                                SharedString::from(format!("tool-input:{}", message.id)),
                                presentation::input_detail(message),
                            )
                            .selectable(true),
                        );
                    if let Some(result) = result {
                        body = body
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(color(if failed { "blocked" } else { "dim" }))
                                    .child(if failed { "执行失败" } else { "输出" }),
                            )
                            .child(
                                TextView::markdown(
                                    SharedString::from(format!("tool-output:{}", message.id)),
                                    format!("```text\n{}\n```", result.text),
                                )
                                .selectable(true),
                            );
                    }
                    entry = entry.child(
                        body.child(
                            div()
                                .id(SharedString::from(format!("copy-tool:{}", message.id)))
                                .text_xs()
                                .text_color(color("dim"))
                                .cursor_pointer()
                                .child("复制详情")
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                                }),
                        ),
                    );
                }
                items = items.child(entry);
            }
            group = group.child(items);
        }
        if let Some((id, steps)) = presentation::plan(messages) {
            let completed = steps.iter().filter(|s| s.status == "completed").count();
            let key = format!("plan:{id}");
            let all = self.expanded.contains(&key);
            let mut plan = div()
                .debug_selector(|| "chat-plan".into())
                .mt_2()
                .mb_2()
                .px_3()
                .py_2()
                .border_l_2()
                .border_color(color("border"))
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(color("dim"))
                        .child(format!("任务步骤 · {completed}/{}", steps.len())),
                );
            for step in steps.iter().take(if all { steps.len() } else { 4 }) {
                plan = plan.child(
                    div()
                        .flex()
                        .items_start()
                        .gap_2()
                        .text_sm()
                        .text_color(color(if step.status == "in_progress" {
                            "text"
                        } else {
                            "dim"
                        }))
                        .child(div().flex_shrink_0().child(match step.status.as_str() {
                            "completed" => "✓",
                            "in_progress" => "◉",
                            _ => "○",
                        }))
                        .child(div().min_w_0().child(step.text.clone())),
                );
            }
            if steps.len() > 4 {
                plan = plan.child(
                    div()
                        .id(SharedString::from(key.clone()))
                        .text_xs()
                        .text_color(color("dim"))
                        .cursor_pointer()
                        .child(if all {
                            "收起步骤".into()
                        } else {
                            format!("查看全部 {} 步", steps.len())
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.expanded.remove(&key) {
                                this.expanded.insert(key.clone());
                            }
                            this.list.remeasure_items(row_index..row_index + 1);
                            cx.notify();
                        })),
                );
            }
            group = group.child(plan);
        }
        div()
            .w_full()
            .flex()
            .justify_center()
            .px_6()
            .py_2()
            .child(div().w_full().max_w(px(820.)).child(group))
            .into_any_element()
    }
    fn message(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self.rows.get(index) else {
            return div().into_any_element();
        };
        if row.tools {
            return self.tool_group(index, row, cx);
        }
        let index = row.start;
        let Some(m) = self.conversation.messages.get(index) else {
            return div().into_any_element();
        };
        let id = m.id.clone();
        let copy = m.text.clone();
        let body = if m.role == "You" {
            div().w_full().flex().justify_end().py_3().child(
                div()
                    .id(SharedString::from(format!("user-bubble-{id}")))
                    .max_w(relative(0.85))
                    .px_4()
                    .py_3()
                    .rounded_lg()
                    .bg(color("input"))
                    .child(self.image_strip(&m.images, false, cx))
                    .child(
                        TextView::markdown(
                            SharedString::from(format!("chat-user-{id}")),
                            m.text.clone(),
                        )
                        .selectable(true),
                    ),
            )
        } else {
            div()
                .w_full()
                .py_4()
                .when(m.role == "Notice", |el| {
                    el.text_color(color("dim")).text_sm()
                })
                .child(
                    TextView::markdown(
                        SharedString::from(format!("chat-response-{id}")),
                        m.text.clone(),
                    )
                    .selectable(true),
                )
                .when(m.role == "Claude" || m.role == "Reminder", |el| {
                    el.child(
                        div().flex().justify_end().mt_2().child(
                            div()
                                .id(("copy", index))
                                .text_xs()
                                .text_color(color("dim"))
                                .cursor_pointer()
                                .child("Copy")
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                                }),
                        ),
                    )
                })
        };
        div()
            .id(("message", index))
            .w_full()
            .flex()
            .justify_center()
            .px_6()
            .child(div().w_full().max_w(px(820.)).child(body))
            .into_any_element()
    }
    fn viewport_height(&self, window: &Window) -> f32 {
        let measured = self.pane_height.get();
        let height = f32::from(window.viewport_size().height);
        if measured > 0. {
            measured.min(height)
        } else {
            height
        }
    }
    fn toggle_menu(&mut self, kind: Menu, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = if self.menu == Some(kind) {
            None
        } else {
            Some(kind)
        };
        let current = if kind == Menu::Model {
            self.conversation
                .model
                .as_deref()
                .unwrap_or(&self.config.model)
        } else {
            &self.conversation.effort
        };
        self.menu_index = self
            .choices()
            .iter()
            .position(|c| c.value == current)
            .unwrap_or(0);
        self.menu_scroll.scroll_to_item(self.menu_index + 1);
        self.editor.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }
    fn menu_view(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let kind = self.menu?;
        let choices = self.choices();
        let title = match kind {
            Menu::Slash => "Commands",
            Menu::Files => "Project files",
            Menu::Model => "Model",
            Menu::Effort => "Reasoning effort",
        };
        let mut menu = div()
            .id("chat-menu")
            .debug_selector(|| "chat-menu".into())
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .max_h(px((self.viewport_height(window) - 300.).clamp(60., 260.)))
            .overflow_y_scroll()
            .track_scroll(&self.menu_scroll)
            .max_w(px(if kind == Menu::Files { 480. } else { 400. }))
            .w_full()
            .rounded_lg()
            .border_1()
            .border_color(color("border"))
            .bg(color("surface"))
            .p_2()
            .mb_2()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .px_2()
                    .pb_2()
                    .text_xs()
                    .text_color(color("dim"))
                    .child(title)
                    .child("↑↓ choose · Tab / Enter select")
                    .when(kind == Menu::Files, |el| {
                        el.child(
                            div()
                                .id("chat-refresh-files")
                                .cursor_pointer()
                                .child("Refresh")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.workspace_requested = false;
                                    this.load_workspace(cx);
                                })),
                        )
                    }),
            );
        if choices.is_empty() {
            menu = menu.child(div().p_3().text_sm().text_color(color("dim")).child(
                if kind == Menu::Files && !self.conversation.workspace_error.is_empty() {
                    self.conversation.workspace_error.clone()
                } else if kind == Menu::Files && !self.conversation.workspace_loaded {
                    if self
                        .conversation
                        .capabilities
                        .iter()
                        .any(|c| c == "workspace")
                    {
                        "Loading remote files…".into()
                    } else {
                        "Connect or update the remote service to browse files.".into()
                    }
                } else {
                    "No matches".into()
                },
            ));
        }
        for (i, c) in choices.into_iter().enumerate() {
            let current = match kind {
                Menu::Model => {
                    c.value
                        == self
                            .conversation
                            .model
                            .as_deref()
                            .unwrap_or(&self.config.model)
                }
                Menu::Effort => c.value == self.conversation.effort,
                _ => false,
            };
            menu = menu.child(
                div()
                    .id(("chat-choice", i))
                    .debug_selector(|| "chat-menu-choice".into())
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .cursor_pointer()
                    .when(i == self.menu_index, |el| el.bg(color("input")))
                    .hover(|s| s.bg(color("input")))
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.accept_choice(i, window, cx)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .max_w(relative(0.6))
                            .min_w_0()
                            .truncate()
                            .child(c.label),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_right()
                            .text_xs()
                            .text_color(color("dim"))
                            .child(if current {
                                "✓ 当前使用".to_owned()
                            } else {
                                c.detail
                            }),
                    ),
            );
        }
        Some(menu.into_any_element())
    }
}

impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pane_height = self.viewport_height(window);
        let sidebar = (self.config.id == super::SUPER_ID
            && self.decision_open.unwrap_or(self.pane_width.get() >= 940.))
        .then(|| self.decision_sidebar(window, cx));
        let max_rows = if pane_height < 500. {
            if self.status_expanded
                || self.menu.is_some()
                || !self.draft_images.is_empty()
                || !self.conversation.permissions.is_empty()
            {
                2
            } else {
                4
            }
        } else {
            6
        };
        if self.editor_max_rows != max_rows {
            self.editor_max_rows = max_rows;
            self.editor
                .update(cx, |input, cx| input.set_auto_grow(1, max_rows, cx));
        }
        let view = cx.entity();
        let approvals = self.conversation.permissions.clone();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let status = self.conversation.activity_label(now);
        let mut controls = div()
            .px_5()
            .py_2()
            .border_b_1()
            .border_color(color("border").opacity(0.5))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .text_sm()
                    .child(
                        div()
                            .id("chat-connection-toggle")
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .flex()
                            .gap_2()
                            .items_center()
                            .cursor_pointer()
                            .text_color(color("dim"))
                            .child(format!(
                                "{}  ·  {}  ⌄",
                                if self.config.id==super::SUPER_ID {"Session manager"}else{&self.config.host},
                                self.config.folder.rsplit('/').next().unwrap_or("Project")
                            ))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.connection_open = !this.connection_open;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_4()
                            .items_center()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(color(if self.conversation.status == "working" {
                                        "working"
                                    } else {
                                        "dim"
                                    }))
                                    .child(status),
                            )
                            .when(self.config.id == super::SUPER_ID, |el| el.child(
                                div().id("decision-toggle").debug_selector(|| "decision-toggle".into())
                                    .cursor_pointer().text_sm().child(format!("☷ 待你决策 {}", self.decision_rows.iter().filter(|r| r["status"] != "resolved").count()))
                                    .on_click(cx.listener(|this, _, _, cx| { this.decision_open = Some(!this.decision_open.unwrap_or(this.pane_width.get() >= 940.)); cx.notify(); }))
                            ))
                            .child(
                                div()
                                    .id("chat-new")
                                    .cursor_pointer()
                                    .child(if self.config.id==super::SUPER_ID {"Manage sessions"}else{"＋ New chat"})
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if this.config.id==super::SUPER_ID {this.notice="Tell Super Agent what to do; it can create local or remote workers for you.".into();cx.notify();return;}
                                        cx.emit(ChatEvent::NewConversation {
                                            host: this.config.host.clone(),
                                            folder: this.config.folder.clone(),
                                            model: this
                                                .conversation
                                                .model
                                                .clone()
                                                .unwrap_or_else(|| this.config.model.clone()),
                                        });
                                    })),
                            ),
                    ),
            );
        if self.connection_open {
            controls = controls.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .rounded_lg()
                    .bg(color("input"))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(div().flex_1().min_w_0().child(Input::new(&self.host)))
                            .child(div().flex_1().min_w_0().child(Input::new(&self.folder))),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_4()
                            .text_sm()
                            .child(
                                div()
                                    .id("chat-connect")
                                    .cursor_pointer()
                                    .child("Reconnect")
                                    .on_click(cx.listener(|this, _, _, cx| this.configure(cx))),
                            )
                            .child(
                                div()
                                    .id("chat-install")
                                    .cursor_pointer()
                                    .child(if self.installing {
                                        "Updating…"
                                    } else {
                                        "Update service"
                                    })
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.install_bridge(cx)),
                                    ),
                            ),
                    )
                    .child(div().text_xs().text_color(color("dim")).child(format!(
                        "{} · {} messages · local history",
                        self.conversation.status,
                        self.conversation.messages.len()
                    ))),
            );
        }
        let mut approvals_view = div()
            .id("chat-approvals")
            .debug_selector(|| "chat-approvals".into())
            .max_h(px(if pane_height < 500. {
                (pane_height - 230.).clamp(48., 180.)
            } else {
                (pane_height - 350.).clamp(96., 360.)
            }))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_2();
        let mut footer = div().flex().flex_col().gap_2().px_5().pt_2().pb_3();
        for (id, permission) in approvals {
            let deny = id.clone();
            let allow = id.clone();
            let input = &permission["input"];
            let sending = self.decisions_in_flight.contains(&id);
            let minimized_key = format!("permission-minimized:{id}");
            let minimized = self.expanded.contains(&minimized_key);
            let toggle = minimized_key.clone();
            let mut card = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_4()
                .rounded_lg()
                .border_1()
                .border_color(color("border"))
                .bg(color("surface"))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_xs()
                                .text_color(color("dim"))
                                .truncate()
                                .child(
                                    permission["source_label"]
                                        .as_str()
                                        .unwrap_or(if permission["name"] == "AskUserQuestion" {
                                            "需要你的决定"
                                        } else {
                                            "确认工具执行"
                                        })
                                        .to_owned(),
                                ),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("minimize-{id}")))
                                .debug_selector(|| "chat-confirm-collapse".into())
                                .cursor_pointer()
                                .text_xs()
                                .text_color(color("dim"))
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .hover(|s| s.bg(color("input")))
                                .child(if minimized { "展开" } else { "⌄" })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if !this.expanded.remove(&toggle) {
                                        this.expanded.insert(toggle.clone());
                                    }
                                    cx.notify();
                                })),
                        ),
                );
            if minimized {
                approvals_view = approvals_view.child(card);
                continue;
            }
            if permission["name"] == "AskUserQuestion" {
                for q in input["questions"].as_array().into_iter().flatten() {
                    let question = q["question"].as_str().unwrap_or_default().to_owned();
                    card = card.child(div().text_base().child(question.clone()));
                    let multi = q["multiSelect"] == true;
                    let field = self
                        .free_answers
                        .entry((id.clone(), question.clone()))
                        .or_insert_with(|| {
                            cx.new(|cx| InputState::new(window, cx).placeholder("填写你的回答…"))
                        })
                        .clone();

                    let mut choices = div().flex().flex_wrap().gap_2();
                    let mut descriptions = div().flex().flex_col().gap_1();
                    for option in q["options"].as_array().into_iter().flatten() {
                        let label = option["label"].as_str().unwrap_or_default().to_owned();
                        let pid = id.clone();
                        let q = question.clone();
                        let value = label.clone();
                        let selected = self
                            .answers
                            .get(&id)
                            .and_then(|a| a.get(&question))
                            .is_some_and(|v| v.split(", ").any(|x| x == label));
                        if selected
                            && let Some(description) =
                                option["description"].as_str().filter(|s| !s.is_empty())
                        {
                            descriptions = descriptions.child(
                                div()
                                    .text_sm()
                                    .text_color(color("dim"))
                                    .child(description.to_owned()),
                            );
                        }
                        choices = choices.child(
                            div()
                                .id(SharedString::from(format!(
                                    "answer-{id}-{question}-{label}"
                                )))
                                .debug_selector(|| "chat-confirm-choice".into())
                                .px_3()
                                .py_2()
                                .rounded_md()
                                .max_w_full()
                                .bg(color(if selected { "text" } else { "input" }))
                                .hover(|s| s.opacity(0.85))
                                .cursor_pointer()
                                .text_sm()
                                .text_color(color(if selected { "bg" } else { "text" }))
                                .child(format!(
                                    "{}{label}",
                                    if selected && multi { "✓ " } else { "" }
                                ))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if this.decisions_in_flight.contains(&pid) {
                                        return;
                                    }
                                    this.expanded.remove(&format!("other-answer:{pid}:{q}"));
                                    let answer = this
                                        .answers
                                        .entry(pid.clone())
                                        .or_default()
                                        .entry(q.clone())
                                        .or_default();
                                    if multi {
                                        let mut values: Vec<String> = answer
                                            .split(", ")
                                            .filter(|s| !s.is_empty())
                                            .map(str::to_owned)
                                            .collect();
                                        if values.contains(&value) {
                                            values.retain(|v| v != &value);
                                        } else {
                                            values.push(value.clone());
                                        }
                                        *answer = values.join(", ");
                                    } else {
                                        *answer = value.clone();
                                    }
                                    cx.notify();
                                })),
                        );
                    }
                    let has_selection = self
                        .answers
                        .get(&id)
                        .and_then(|a| a.get(&question))
                        .is_some_and(|s| !s.is_empty());
                    let other_key = format!("other-answer:{id}:{question}");
                    let show_other = q["options"].as_array().is_none_or(|v| v.is_empty())
                        || self.expanded.contains(&other_key);
                    let toggle = other_key.clone();
                    choices = choices.child(
                        div()
                            .id(SharedString::from(other_key))
                            .cursor_pointer()
                            .text_xs()
                            .text_color(color("dim"))
                            .px_2()
                            .py_2()
                            .child(if show_other {
                                "其他回答"
                            } else {
                                "填写其他答案…"
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.expanded.remove(&toggle) {
                                    this.expanded.insert(toggle.clone());
                                }
                                cx.notify();
                            })),
                    );
                    card = card
                        .child(choices)
                        .when(has_selection, |el| el.child(descriptions));
                    if show_other {
                        card = card.child(
                            div()
                                .p_2()
                                .rounded_md()
                                .border_1()
                                .border_color(color("border"))
                                .bg(color("input"))
                                .child(Input::new(&field)),
                        );
                    }
                }
            } else {
                let text = serde_json::to_string_pretty(input).unwrap_or_default();
                let reason = input["reason"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("此操作需要你的确认，允许后会继续当前会话。");
                card = card.child(div().text_sm().child(reason.to_owned()));
                if let Some(command) = input["command"].as_str() {
                    card = card.child(TextView::markdown(
                        SharedString::from(format!("approval-command:{id}")),
                        format!("```sh\n{command}\n```"),
                    ));
                }
                if let Some(path) = input["file_path"]
                    .as_str()
                    .or_else(|| input["path"].as_str())
                {
                    card = card.child(
                        div()
                            .text_sm()
                            .text_color(color("text"))
                            .child(path.to_owned()),
                    );
                }
                if let Some(changes) = input["changes"].as_array() {
                    for change in changes.iter().take(6) {
                        if let Some(path) = change["path"].as_str() {
                            card = card.child(div().text_sm().child(path.to_owned()));
                        }
                    }
                    if changes.len() > 6 {
                        card = card.child(div().text_xs().text_color(color("dim")).child(format!(
                            "共 {} 个文件，展开完整请求查看全部改动",
                            changes.len()
                        )));
                    }
                }
                let detail_key = format!("approval-details:{id}");
                let show = self.expanded.contains(&detail_key);
                let toggle = detail_key.clone();
                card = card.child(
                    div()
                        .id(SharedString::from(detail_key))
                        .cursor_pointer()
                        .text_xs()
                        .text_color(color("dim"))
                        .child(if show {
                            "⌄ 完整请求"
                        } else {
                            "› 查看完整请求"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.expanded.remove(&toggle) {
                                this.expanded.insert(toggle.clone());
                            }
                            cx.notify();
                        })),
                );
                if show {
                    card = card.child(
                        div()
                            .id(SharedString::from(format!("approval-request:{id}")))
                            .max_h(px(220.))
                            .overflow_y_scroll()
                            .child(
                                TextView::markdown(
                                    SharedString::from(format!("permission-{id}")),
                                    format!("```json\n{text}\n```"),
                                )
                                .selectable(true),
                            ),
                    );
                }
            }
            approvals_view = approvals_view.child(
                card.child(
                    div()
                        .flex()
                        .gap_4()
                        .when(
                            !(permission["source"] == "codex_worker"
                                && permission["name"] == "AskUserQuestion"),
                            |el| {
                                el.child(
                                    div()
                                        .id(SharedString::from(format!("deny-{id}")))
                                        .cursor_pointer()
                                        .px_3()
                                        .py_2()
                                        .rounded_md()
                                        .border_1()
                                        .border_color(color("border"))
                                        .hover(|s| s.bg(color("input")))
                                        .child("拒绝")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.permission(&deny, "deny", cx)
                                        })),
                                )
                            },
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("allow-{id}")))
                                .debug_selector(|| "chat-confirm-submit".into())
                                .cursor_pointer()
                                .px_4()
                                .py_2()
                                .rounded_md()
                                .bg(color("text"))
                                .text_color(color("bg"))
                                .hover(|s| s.opacity(0.85))
                                .when(sending, |el| el.opacity(0.5))
                                .child(if sending {
                                    "提交中…"
                                } else if permission["name"] == "AskUserQuestion" {
                                    "确认选择"
                                } else {
                                    "允许这一次"
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.permission(&allow, "allow", cx)
                                })),
                        ),
                ),
            );
        }
        footer = footer.when(!self.conversation.permissions.is_empty(), |el| {
            el.child(approvals_view)
        });
        if !self.conversation.reminders.is_empty() {
            footer = footer.child(
                div()
                    .id("reminder-inbox")
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .text_sm()
                    .child(format!(
                        "⏰ {} 条提醒待确认",
                        self.conversation.reminders.len()
                    ))
                    .child(
                        div()
                            .id("reminder-ack")
                            .cursor_pointer()
                            .text_color(color("accent"))
                            .child("我已看到")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let ids = this
                                    .conversation
                                    .reminders
                                    .keys()
                                    .cloned()
                                    .collect::<Vec<_>>();
                                this.command(json!({"action":"reminder_ack","ids":ids}), cx);
                            })),
                    ),
            );
        }
        let menu = self.menu_view(window, cx);
        if !self.notice.is_empty() {
            footer = footer.child(
                div()
                    .id("chat-notice")
                    .max_h(px(40.))
                    .overflow_y_scroll()
                    .text_xs()
                    .text_color(color("dim"))
                    .px_2()
                    .pb_2()
                    .child(self.notice.clone()),
            );
        }
        let model = self
            .conversation
            .model
            .as_deref()
            .unwrap_or(&self.config.model);
        let model_label = match model {
            "opus" => "Opus",
            "sonnet" => "Sonnet",
            "haiku" => "Haiku",
            other => other,
        };
        let effort = if self.conversation.effort.is_empty() {
            "Default effort"
        } else {
            self.conversation.effort.as_str()
        };
        let busy = matches!(
            self.conversation.status.as_str(),
            "working" | "interrupting"
        );
        let empty = self.editor.read(cx).value().trim().is_empty() && self.draft_images.is_empty();
        // Let the textarea measure its capped auto-grow rows. A percentage-height
        // child in a non-flex frame otherwise paints a larger viewport through a
        // two-line clip, so wheel scrolling can never reveal the hidden lines.
        let composer = div().debug_selector(|| "chat-composer".into()).flex_shrink_0().rounded_lg().border_1().border_color(color("border")).bg(color("surface")).p_3().flex().flex_col().gap_2()
            .child(self.image_strip(&self.draft_images.clone(), true, cx))
            .child(div().id("chat-draft-viewport").px_1().py_2().flex().flex_col().flex_shrink_0().min_h(px(28.)).child(Textarea::new(&self.editor)))
            .child(div().flex().items_center().justify_between().gap_2()
                .child(div().flex().flex_1().min_w_0().items_center().flex_wrap().gap_3().gap_y_1().text_sm().text_color(color("dim"))
                    .child(div().id("chat-add-file").cursor_pointer().text_lg().child("＋").on_click(cx.listener(|this,_,window,cx|{this.editor.update(cx,|s,cx|s.insert("@",window,cx));this.load_workspace(cx);})))
                    .child(div().id("chat-slash").cursor_pointer().child("/").on_click(cx.listener(|this,_,window,cx|{if this.editor.read(cx).value().trim().is_empty(){this.set_draft("/",window,cx);}else{this.notice="Start a new message with / to use commands. Your draft is kept.".into();cx.notify();}})))
                    .when(self.config.id==super::SUPER_ID,|el|el.child(div().id("super-provider").cursor_pointer().child(format!("{} ⇄",if self.conversation.provider=="claude" {"Claude"}else{"Codex"})).on_click(cx.listener(|this,_,_,cx|{let provider=if this.conversation.provider=="claude" {"codex"}else{"claude"};this.command(json!({"action":"provider","provider":provider}),cx);}))))
                    .map(|el|el.child(div().id("chat-model").cursor_pointer().child(format!("{model_label} ⌄")).on_click(cx.listener(|this,_,window,cx|{this.toggle_menu(Menu::Model,window,cx); }))))
                    .map(|el|el.child(div().id("chat-effort").cursor_pointer().child(format!("{effort} ⌄")).on_click(cx.listener(|this,_,window,cx|{this.toggle_menu(Menu::Effort,window,cx); })))))
                .child(div().flex().gap_3().items_center()
                    .when(busy,|el|el.child(div().id("chat-interrupt").w_8().h_8().flex().items_center().justify_center().rounded_full().bg(color("text")).text_color(color("bg")).cursor_pointer().child("■").on_click(cx.listener(|this,_,_,cx|{this.command(json!({"action":"interrupt"}),cx);}))))
                    .when(!busy,|el|el.child(div().id("chat-send").w_8().h_8().flex().items_center().justify_center().rounded_full().bg(color("text")).text_color(color("bg")).when(empty,|el|el.opacity(0.35)).cursor_pointer().child("↑").on_click(cx.listener(|this,_,window,cx|this.send(window,cx)))))))
            ;

        footer = footer.child(
            div()
                .relative()
                .child(composer)
                .when_some(menu, |el, menu| {
                    el.child(deferred(
                        div()
                            .absolute()
                            .bottom(relative(1.))
                            .left_0()
                            .w_full()
                            .child(menu),
                    ))
                }),
        );

        let mut usage = div()
            .id("chat-statusline")
            .px_2()
            .py_1()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x_3()
            .gap_y_1()
            .text_xs();
        for (label, warning) in
            super::status_line::segments(&self.conversation.status_line, model_label, now / 1000)
        {
            usage = usage.child(
                div()
                    .text_color(color(if warning { "working" } else { "dim" }))
                    .child(label),
            );
        }
        usage = usage.child(
            div()
                .id("chat-usage-details")
                .cursor_pointer()
                .text_color(color("dim"))
                .child(if self.status_expanded {
                    "Less ⌃"
                } else {
                    "Details ⌄"
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.status_expanded = !this.status_expanded;
                    cx.notify();
                })),
        );
        if pane_height >= 400. || self.conversation.permissions.is_empty() {
            footer = footer.child(usage);
        }
        if self.status_expanded && (pane_height >= 400. || self.conversation.permissions.is_empty())
        {
            footer = footer.child(
                div()
                    .id("chat-statusline-details")
                    .px_2()
                    .py_1()
                    .max_h(px(72.))
                    .overflow_y_scroll()
                    .text_xs()
                    .text_color(color("dim"))
                    .child(super::status_line::details(
                        &self.conversation.status_line,
                        now / 1000,
                    )),
            );
        }

        footer = footer.when(
            !(pane_height < 500.
                && (!self.conversation.permissions.is_empty()
                    || self.status_expanded && !self.draft_images.is_empty())),
            |footer| {
                footer.child(
                    div()
                        .text_xs()
                        .text_color(color("dim"))
                        .px_2()
                        .pt_1()
                        .child("Shift + Enter for a new line"),
                )
            },
        );

        let history: AnyElement = if self.conversation.messages.is_empty()
            && !self.draft_images.is_empty()
        {
            div().into_any_element()
        } else if self.conversation.messages.is_empty() {
            div().flex_1().size_full().flex().items_center().justify_center().child(div().flex().flex_col().gap_3().px_6().child(div().text_2xl().child(if self.config.id==super::SUPER_ID {"What should I take care of?"}else{"What are we working on?"})).child(div().text_sm().text_color(color("dim")).child(if self.config.id==super::SUPER_ID {"Give me a task, or ask what needs your attention across your local and remote sessions."}else{"Ask Claude to explore your project, make changes, or review your work."}))).into_any_element()
        } else {
            list(self.list.clone(), move |i, _, cx| {
                view.update(cx, |this, cx| this.message(i, cx))
            })
            .size_full()
            .into_any_element()
        };
        let measured = self.pane_height.clone();
        let measured_width = self.pane_width.clone();
        let observer = cx.entity().downgrade();
        let chat = div()
            .h_full()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .capture_action(
                cx.listener(|this, _: &gpui_base::input::Paste, window, cx| {
                    if this.editor.read(cx).focus_handle(cx).is_focused(window)
                        && this.paste_images(cx)
                    {
                        cx.stop_propagation();
                    } else {
                        cx.propagate();
                    }
                }),
            )
            .capture_action(
                cx.listener(|this, _: &gpui_base::input::MovePageUp, window, cx| {
                    if this.editor.read(cx).focus_handle(cx).is_focused(window) {
                        cx.propagate();
                        return;
                    }
                    this.list.scroll_by(px(-480.));
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .capture_action(
                cx.listener(|this, _: &gpui_base::input::MovePageDown, window, cx| {
                    if this.editor.read(cx).focus_handle(cx).is_focused(window) {
                        cx.propagate();
                        return;
                    }
                    this.list.scroll_by(px(480.));
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .capture_action(cx.listener(|this, _: &gpui_base::input::MoveDown, _, cx| {
                let n = this.choices().len();
                if this.menu.is_some() && n > 0 {
                    this.menu_index = (this.menu_index + 1) % n;
                    this.menu_scroll.scroll_to_item(this.menu_index + 1);
                    cx.stop_propagation();
                    cx.notify();
                } else {
                    cx.propagate();
                }
            }))
            .capture_action(cx.listener(|this, _: &gpui_base::input::MoveUp, _, cx| {
                let n = this.choices().len();
                if this.menu.is_some() && n > 0 {
                    this.menu_index = (this.menu_index + n - 1) % n;
                    this.menu_scroll.scroll_to_item(this.menu_index + 1);
                    cx.stop_propagation();
                    cx.notify();
                } else {
                    cx.propagate();
                }
            }))
            .capture_action(
                cx.listener(|this, _: &gpui_base::input::IndentInline, window, cx| {
                    if this.menu.is_some() && !this.choices().is_empty() {
                        this.accept_choice(this.menu_index, window, cx);
                        cx.stop_propagation();
                    } else {
                        cx.propagate();
                    }
                }),
            )
            .capture_action(cx.listener(|this, _: &gpui_base::input::Escape, _, cx| {
                if this.preview_image.take().is_some() {
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                if this.menu.take().is_some() {
                    cx.stop_propagation();
                    cx.notify();
                } else {
                    cx.propagate();
                }
            }))
            .capture_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                if this.preview_image.is_some() {
                    if e.keystroke.key == "escape" {
                        this.preview_image = None;
                        this.editor.read(cx).focus_handle(cx).focus(window, cx);
                        cx.stop_propagation();
                        cx.notify();
                    }
                    return;
                }
                if this.editor.read(cx).focus_handle(cx).is_focused(window) {
                    return;
                }
                match e.keystroke.key.as_str() {
                    "pageup" => this.list.scroll_by(px(-480.)),
                    "pagedown" => this.list.scroll_by(px(480.)),
                    "home" if e.keystroke.modifiers.control => {
                        this.list.scroll_to(ListOffset::default())
                    }
                    "end" if e.keystroke.modifiers.control => {
                        this.list.set_follow_mode(FollowMode::Tail)
                    }
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .bg(color("bg"))
            .text_color(color("text"))
            .text_size(px(14.))
            .overflow_hidden()
            .relative()
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let height = f32::from(bounds.size.height);
                        if (height - measured.get()).abs() > 1. {
                            measured.set(height);
                            let observer = observer.clone();
                            cx.defer(move |cx| {
                                let _ = observer.update(cx, |_, cx| cx.notify());
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(controls)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            if this.menu.is_some() {
                                this.menu = None;
                                cx.notify();
                            }
                        }),
                    )
                    .relative()
                    .debug_selector(|| "chat-history-viewport".into())
                    .child(history)
                    .when(!self.list.is_following_tail(), |el| {
                        el.child(
                            div()
                                .absolute()
                                .bottom_3()
                                .left_0()
                                .w_full()
                                .flex()
                                .justify_center()
                                .child(
                                    div()
                                        .id("chat-bottom")
                                        .debug_selector(|| "chat-jump-latest".into())
                                        .px_3()
                                        .py_2()
                                        .rounded_full()
                                        .border_1()
                                        .border_color(color("border"))
                                        .bg(color("surface"))
                                        .text_sm()
                                        .text_color(color("text"))
                                        .cursor_pointer()
                                        .hover(|s| s.bg(color("input")))
                                        .child("↓ 回到最新")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.list.set_follow_mode(FollowMode::Tail);
                                            cx.notify();
                                        })),
                                ),
                        )
                    }),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(div().w_full().max_w(px(860.)).child(footer)),
            )
            .when_some(self.preview_image.clone(), |el, id| {
                el.child(
                    div()
                        .id("chat-image-preview")
                        .track_focus(&self.preview_focus)
                        .occlude()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(color("bg").opacity(0.98))
                        .p_4()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            div().flex().justify_between().child("Image preview").child(
                                div()
                                    .id("chat-image-close")
                                    .cursor_pointer()
                                    .child("Close ×")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.preview_image = None;
                                        this.editor.read(cx).focus_handle(cx).focus(window, cx);
                                        cx.stop_propagation();
                                        cx.notify();
                                    })),
                            ),
                        )
                        .child(
                            div().flex_1().min_h_0().child(
                                img(attachments::path(
                                    &transport::local_dir(&self.config.id),
                                    &id,
                                ))
                                .size_full()
                                .object_fit(ObjectFit::Contain),
                            ),
                        ),
                )
            });
        let observer = cx.entity().downgrade();
        div()
            .size_full()
            .flex()
            .relative()
            .child(chat)
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let width = f32::from(bounds.size.width);
                        if (measured_width.get() - width).abs() > 1. {
                            measured_width.set(width);
                            let observer = observer.clone();
                            cx.defer(move |cx| {
                                let _ = observer.update(cx, |_, cx| cx.notify());
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .when_some(sidebar, |el, sidebar| el.child(sidebar))
    }
}

#[cfg(test)]
mod scroll_tests {
    use super::{Conversation, sync_history_list};
    use gpui::prelude::*;
    use gpui::{
        Context, FollowMode, ListAlignment, ListState, Render, ScrollDelta, ScrollWheelEvent,
        TestAppContext, Window, div, list, point, px, size,
    };
    use serde_json::json;
    use std::{cell::Cell, rc::Rc};

    #[gpui::test]
    fn reading_anchor_survives_heartbeats_streaming_and_appends(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();
        let state = ListState::new(100, ListAlignment::Bottom, px(1000.));
        let height = Rc::new(Cell::new(50.));
        struct View(ListState, Rc<Cell<f32>>);
        impl Render for View {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let height = self.1.clone();
                list(self.0.clone(), move |i, _, _| {
                    div()
                        .h(px(if i == 99 { height.get() } else { 50. }))
                        .w_full()
                        .into_any_element()
                })
                .size_full()
            }
        }
        let view = cx.update(|_, cx| cx.new(|_| View(state.clone(), height.clone())));
        let draw = |cx: &mut gpui::VisualTestContext| {
            cx.draw(point(px(0.), px(0.)), size(px(400.), px(200.)), |_, _| {
                view.clone().into_any_element()
            });
        };
        state.set_follow_mode(FollowMode::Tail);
        draw(cx);
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(100.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(300.))),
            ..Default::default()
        });
        assert!(!state.is_following_tail());
        // Most of the history is outside overdraw and has unknown height.
        assert!(state.is_scrolled_to_end().is_none());
        let anchor = state.logical_scroll_top();
        let mut conversation = Conversation::default();
        for i in 0..100 {
            conversation.put(i.to_string(), "Claude", "row".into(), false);
        }
        conversation.take_dirty_rows();
        for tick in 0..10 {
            let old = conversation.messages.len();
            conversation.apply(&json!({"type":"activity","last_progress":1000,"phase":"thinking"}));
            assert!(conversation.take_dirty_rows().is_empty());
            height.set(50. + tick as f32 * 20.);
            conversation.put("99".into(), "Claude", " streamed".into(), true);
            conversation.put(format!("new-{tick}"), "Claude", "new row".into(), false);
            sync_history_list(
                &state,
                old,
                conversation.messages.len(),
                conversation.take_dirty_rows(),
            );
            draw(cx);
            assert!(!state.is_following_tail());
            let current = state.logical_scroll_top();
            assert_eq!(current.item_ix, anchor.item_ix);
            assert!((current.offset_in_item - anchor.offset_in_item).abs() < px(1.));
        }
        state.set_follow_mode(FollowMode::Tail);
        draw(cx);
        assert!(state.is_following_tail());
    }
    #[gpui::test]
    fn growing_tool_groups_do_not_add_phantom_rows_or_move_a_reading_anchor(
        cx: &mut TestAppContext,
    ) {
        let cx = cx.add_empty_window();
        let list = ListState::new(100, ListAlignment::Bottom, px(1000.));
        struct View(ListState);
        impl Render for View {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                gpui::list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any_element()
                })
                .size_full()
            }
        }
        let view = cx.update(|_, cx| cx.new(|_| View(list.clone())));
        let draw = |cx: &mut gpui::VisualTestContext| {
            cx.draw(point(px(0.), px(0.)), size(px(400.), px(200.)), |_, _| {
                view.clone().into_any_element()
            });
        };
        list.set_follow_mode(FollowMode::Tail);
        draw(cx);
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(100.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(300.))),
            ..Default::default()
        });
        let anchor = list.logical_scroll_top();
        let mut conversation = Conversation::default();
        for i in 0..99 {
            conversation.put(i.to_string(), "Claude", "row".into(), false);
        }
        conversation.put("tool-0".into(), "Bash", "{}".into(), false);
        conversation.take_dirty_rows();
        let mut rows = super::presentation::rows(&conversation.messages);
        for tick in 0..10 {
            conversation.put(
                format!("result:tool-{tick}"),
                "Tool result",
                "output".into(),
                false,
            );
            conversation.put(format!("tool-{}", tick + 1), "Bash", "{}".into(), false);
            let next = super::presentation::rows(&conversation.messages);
            assert_eq!(next.len(), 100);
            super::sync_presentation_list(&list, &rows, &next, &conversation.take_dirty_rows());
            rows = next;
            draw(cx);
            assert!(!list.is_following_tail());
            let current = list.logical_scroll_top();
            assert_eq!(anchor.item_ix, current.item_ix);
            assert!((anchor.offset_in_item - current.offset_in_item).abs() < px(1.));
        }
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::Menu;
    use super::{ChatConfig, ChatView, Connection, fresh_id};
    use gpui::ListOffset;
    use gpui::{Context, Modifiers, TestAppContext, Window, px, size};
    use serde_json::json;
    struct Home {
        path: std::path::PathBuf,
        old: Option<std::path::PathBuf>,
    }
    impl Home {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("herdr-cards-{}", fresh_id()));
            std::fs::create_dir_all(&path).unwrap();
            let old = crate::mods::test_app_home(Some(path.clone()));
            Self { path, old }
        }
    }
    impl Drop for Home {
        fn drop(&mut self) {
            crate::mods::test_app_home(self.old.take());
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
    fn fixture(window: &mut Window, cx: &mut Context<ChatView>) -> ChatView {
        let mut view = ChatView::new(
            ChatConfig {
                id: fresh_id(),
                host: "invalid host".into(),
                folder: "/qa".into(),
                model: "opus".into(),
                resume: None,
            },
            window,
            cx,
        );
        view.connection_open = false;
        view.notice.clear();
        for e in [
            json!({"type":"tool","id":"qa1","name":"Bash","input":{"description":"检查部署状态","command":"echo qa"},"time":1000}),
            json!({"type":"tool_result","id":"qa1","content":"Ready","time":3000}),
            json!({"type":"tool","id":"qa2","name":"Read","input":{"file_path":"/qa/config.toml"},"time":4000}),
            json!({"type":"tool_result","id":"qa2","is_error":true,"content":"detail\n".repeat(200),"time":6000}),
            json!({"type":"delta","kind":"text_delta","id":"reply","text":"检查已结束，工具详情可以展开查看。"}),
            json!({"type":"state","status":"idle"}),
        ] {
            view.conversation.apply(&e);
        }
        view.sync_rows();
        view
    }
    #[gpui::test]
    fn decision_reply_is_explicit_durable_deduplicated_and_preserves_main_draft(
        cx: &mut TestAppContext,
    ) {
        let home = Home::new();
        crate::chat::decisions::record(&json!({"id":"qa-decision","title":"选择方案","context":"完整问题上下文","address":"t10","options":["方案 A"]})).unwrap();
        cx.update(gpui_shell::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let mut v = fixture(window, cx);
            v.config.id = crate::chat::SUPER_ID.into();
            v.editor
                .update(cx, |s, cx| s.set_value("不能覆盖这个草稿", window, cx));
            v
        });
        cx.simulate_resize(size(px(1200.), px(800.)));
        cx.run_until_parked();
        assert!(cx.debug_bounds("decision-sidebar").is_some());
        assert!(cx.debug_bounds("chat-history-viewport").unwrap().size.width < px(900.));
        let option = cx.debug_bounds("decision-option").unwrap();
        cx.simulate_click(option.center(), Modifiers::default());
        cx.run_until_parked();
        assert_eq!(
            crate::chat::decisions::read().unwrap()[0]["status"],
            "pending"
        );
        // Offline submit keeps the answer and pending state.
        let submit = cx.debug_bounds("decision-submit").unwrap();
        cx.simulate_click(submit.center(), Modifiers::default());
        cx.run_until_parked();
        assert_eq!(
            crate::chat::decisions::read().unwrap()[0]["status"],
            "pending"
        );
        let (connection, receiver) = Connection::test_channel(home.path.join("outbox.json"));
        view.update(cx, |v, cx| {
            v.connection = Some(connection);
            cx.notify();
        });
        cx.run_until_parked();
        let submit = cx.debug_bounds("decision-submit").unwrap();
        cx.simulate_click(submit.center(), Modifiers::default());
        cx.run_until_parked();
        let sent = receiver.try_recv().unwrap();
        assert_eq!(sent["action"], "send");
        assert!(sent["text"].as_str().unwrap().contains("方案 A"));
        assert_eq!(
            crate::chat::decisions::read().unwrap()[0]["status"],
            "answered"
        );
        view.update(cx, |v, cx| v.decision_reply("qa-decision", cx));
        assert!(receiver.try_recv().is_err());
        assert_eq!(
            view.read_with(cx, |v, cx| v.editor.read(cx).value().to_string()),
            "不能覆盖这个草稿"
        );
        let complete = cx.debug_bounds("decision-complete").unwrap();
        cx.simulate_click(complete.center(), Modifiers::default());
        cx.run_until_parked();
        assert_eq!(
            crate::chat::decisions::read().unwrap()[0]["status"],
            "resolved"
        );
        assert!(cx.debug_bounds("decision-card").is_none());
        let tab = cx.debug_bounds("decision-done-tab").unwrap();
        cx.simulate_click(tab.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("decision-card").is_some());
    }
    #[gpui::test]
    fn changed_decision_must_be_reviewed_before_reply(cx: &mut TestAppContext) {
        let home = Home::new();
        let mut card = json!({"id":"qa-changed","title":"选择","context":"原来的问题","address":"t10","options":["A"]});
        crate::chat::decisions::record(&card).unwrap();
        cx.update(gpui_shell::init);
        let (connection, receiver) = Connection::test_channel(home.path.join("outbox.json"));
        let (view, cx) = cx.add_window_view(|window, cx| {
            let mut v = fixture(window, cx);
            v.config.id = crate::chat::SUPER_ID.into();
            v.connection = Some(connection);
            v.decision_choices.insert("qa-changed".into(), "A".into());
            v
        });
        card["context"] = json!("新问题，需要重新读");
        crate::chat::decisions::record(&card).unwrap();
        view.update(cx, |v, cx| v.decision_reply("qa-changed", cx));
        assert!(receiver.try_recv().is_err());
        assert!(view.read_with(cx, |v, _| v.decision_error.contains("事项已更新")));
        assert_eq!(
            crate::chat::decisions::read().unwrap()[0]["status"],
            "pending"
        );
    }
    #[gpui::test]
    fn decision_rail_does_not_cover_narrow_chat_until_opened(cx: &mut TestAppContext) {
        let _home = Home::new();
        cx.update(gpui_shell::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let mut v = fixture(window, cx);
            v.config.id = crate::chat::SUPER_ID.into();
            v
        });
        cx.simulate_resize(size(px(460.), px(600.)));
        cx.run_until_parked();
        assert!(cx.debug_bounds("decision-sidebar").is_none());
        let before = cx.debug_bounds("chat-history-viewport").unwrap();
        let toggle = cx.debug_bounds("decision-toggle").unwrap();
        cx.simulate_click(toggle.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("decision-sidebar").unwrap().size.width <= px(340.));
        assert_eq!(cx.debug_bounds("chat-history-viewport").unwrap(), before);
        assert_eq!(view.read_with(cx, |v, _| v.decision_open), Some(true));
    }
    #[gpui::test]
    fn decision_scroll_does_not_move_conversation(cx: &mut TestAppContext) {
        let _home = Home::new();
        for i in 0..14 {
            crate::chat::decisions::record(&json!({"id":format!("q{i}"),"title":format!("问题 {i}"),"context":"决策上下文", "address":"t10"})).unwrap();
        }
        cx.update(gpui_shell::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let mut v = fixture(window, cx);
            v.config.id = crate::chat::SUPER_ID.into();
            v
        });
        cx.simulate_resize(size(px(1200.), px(700.)));
        cx.run_until_parked();
        let before = view.read_with(cx, |v, _| v.list.logical_scroll_top());
        let bounds = cx.debug_bounds("decision-list").unwrap();
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: bounds.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-500.))),
            ..Default::default()
        });
        cx.run_until_parked();
        assert!(view.read_with(cx, |v, _| v.decision_scroll.offset().y) < px(0.));
        assert_eq!(
            view.read_with(cx, |v, _| v.list.logical_scroll_top().item_ix),
            before.item_ix
        );
    }
    #[gpui::test]
    fn floating_menus_preserve_history_geometry_draft_and_reading_anchor(cx: &mut TestAppContext) {
        let _home = Home::new();
        cx.update(gpui_shell::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let mut v = fixture(window, cx);
            for i in 0..40 {
                v.conversation.put(
                    format!("long-{i}"),
                    "Claude",
                    "历史正文\n\n".repeat(4),
                    false,
                );
            }
            v.sync_rows();
            v.editor
                .update(cx, |s, cx| s.set_value("保留我的草稿", window, cx));
            v
        });
        cx.simulate_resize(size(px(900.), px(800.)));
        cx.run_until_parked();
        view.update(cx, |v, cx| {
            v.list.scroll_to(ListOffset {
                item_ix: 10,
                offset_in_item: px(3.),
            });
            cx.notify();
        });
        cx.run_until_parked();
        let before = view.read_with(cx, |v, _| v.list.logical_scroll_top());
        let geometry = cx.debug_bounds("chat-history-viewport").unwrap();
        assert!(cx.debug_bounds("chat-jump-latest").is_some());
        for kind in [Menu::Model, Menu::Effort, Menu::Files, Menu::Slash] {
            view.update(cx, |v, cx| {
                v.menu = Some(kind);
                cx.notify();
            });
            cx.run_until_parked();
            assert_eq!(cx.debug_bounds("chat-history-viewport").unwrap(), geometry);
            let anchor = view.read_with(cx, |v, _| v.list.logical_scroll_top());
            assert_eq!(anchor.item_ix, before.item_ix);
            assert_eq!(anchor.offset_in_item, before.offset_in_item);
            assert_eq!(
                view.read_with(cx, |v, cx| v.editor.read(cx).value().to_string()),
                "保留我的草稿"
            );
            let popup = cx.debug_bounds("chat-menu").unwrap();
            let composer = cx.debug_bounds("chat-composer").unwrap();
            assert!(popup.origin.y >= px(0.) && popup.bottom() <= composer.origin.y);
        }
        view.update(cx, |v, cx| {
            v.menu = None;
            cx.notify();
        });
        cx.run_until_parked();
        let button = cx.debug_bounds("chat-jump-latest").unwrap();
        cx.simulate_click(button.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(view.read_with(cx, |v, _| v.list.is_following_tail()));
    }
    #[gpui::test]
    fn explicit_task_steps_are_readable_without_opening_raw_tool_json(cx: &mut TestAppContext) {
        let _home = Home::new();
        cx.update(gpui_shell::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let mut v = fixture(window, cx);
            v.conversation.apply(&json!({"type":"tool","id":"plan","name":"TodoWrite","input":{"todos":[{"content":"检查当前配置","status":"completed"},{"content":"运行验证","status":"in_progress"}]}}));
            v.sync_rows(); v
        });
        cx.simulate_resize(size(px(900.), px(800.)));
        cx.run_until_parked();
        assert!(cx.debug_bounds("chat-plan").is_some());
        assert!(view.read_with(cx, |v, _| v.expanded.is_empty()));
    }
    #[gpui::test]
    fn grouped_tools_expand_and_long_results_are_bounded(cx: &mut TestAppContext) {
        let _home = Home::new();
        cx.update(gpui_shell::init);
        let (view, cx) = cx.add_window_view(fixture);
        cx.simulate_resize(size(px(1000.), px(900.)));
        cx.run_until_parked();
        assert_eq!(view.read_with(cx, |v, _| v.rows.len()), 2);
        let group = cx
            .debug_bounds("chat-tool-group")
            .expect("group header visible");
        cx.simulate_click(group.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(view.read_with(cx, |v, _| v.expanded.contains("tools:qa1")));
        let item = cx
            .debug_bounds("chat-tool-item")
            .expect("tool entries visible");
        cx.simulate_click(item.center(), Modifiers::default());
        cx.run_until_parked();
        let detail = cx
            .debug_bounds("chat-tool-detail")
            .expect("tool output expanded");
        assert!(detail.size.height <= px(300.));
        let group = cx.debug_bounds("chat-tool-group").unwrap();
        cx.simulate_click(group.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(!view.read_with(cx, |v, _| v.expanded.contains("tools:qa1")));
    }
    #[gpui::test]
    fn single_tool_opens_directly_without_a_second_disclosure(cx: &mut TestAppContext) {
        let _home = Home::new();
        cx.update(gpui_shell::init);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let mut v = fixture(window, cx);
            v.conversation
                .messages
                .retain(|m| m.id != "qa2" && m.id != "result:qa2");
            v.sync_rows();
            v
        });
        cx.simulate_resize(size(px(900.), px(700.)));
        cx.run_until_parked();
        let header = cx.debug_bounds("chat-tool-group").unwrap();
        assert!(
            header.size.height <= px(40.),
            "Collapsed activity stays one line"
        );
        cx.simulate_click(header.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(cx.debug_bounds("chat-tool-detail").is_some());
        assert!(!view.read_with(cx, |v, _| v.expanded.contains("tool:qa1")));
        let header = cx.debug_bounds("chat-tool-group").unwrap();
        cx.simulate_click(header.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(!view.read_with(cx, |v, _| v.expanded.contains("tools:qa1")));
    }
    #[gpui::test]
    fn confirmation_clicks_preserve_draft_submit_once_and_keep_permission_until_ack(
        cx: &mut TestAppContext,
    ) {
        let home = Home::new();
        cx.update(gpui_shell::init);
        let (connection, wire) = Connection::test_channel(home.path.join("outbox.json"));
        let mut connection = Some(connection);
        let(view,cx)=cx.add_window_view(|window,cx|{let mut v=fixture(window,cx);v.conversation.permissions.insert("q".into(),json!({"name":"AskUserQuestion","input":{"questions":[{"question":"下一步先做什么？","options":[{"label":"先测试","description":"验证当前修复，保留已有任务"},{"label":"继续调查","description":"补充证据后再决定"}]}]}}));v.connection=connection.take();v.editor.update(cx,|s,cx|s.set_value("尚未发送的草稿",window,cx));v});
        cx.simulate_resize(size(px(1000.), px(950.)));
        cx.run_until_parked();
        let collapse = cx.debug_bounds("chat-confirm-collapse").unwrap();
        cx.simulate_click(collapse.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(view.read_with(cx, |v, _| v.conversation.permissions.contains_key("q")));
        let expand = cx.debug_bounds("chat-confirm-collapse").unwrap();
        cx.simulate_click(expand.center(), Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            view.update(cx, |v, cx| {
                v.expanded.insert("other-answer:q:下一步先做什么？".into());
                v.free_answers
                    .get(&("q".into(), "下一步先做什么？".into()))
                    .unwrap()
                    .update(cx, |field, cx| {
                        field.set_value("保留但不提交的其他回答", window, cx)
                    });
                cx.notify();
            })
        });
        cx.run_until_parked();
        let choice = cx.debug_bounds("chat-confirm-choice").unwrap();
        cx.simulate_click(choice.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(
            wire.try_recv().is_err(),
            "Selecting an option is not permission to submit"
        );
        let submit = cx.debug_bounds("chat-confirm-submit").unwrap();
        cx.simulate_click(submit.center(), Modifiers::default());
        cx.run_until_parked();
        let command = wire.try_recv().expect("explicit confirmation sent");
        assert_eq!(command["action"], "permission");
        assert_eq!(command["id"], "q");
        assert_eq!(command["answers"]["下一步先做什么？"], "继续调查");
        cx.simulate_click(submit.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(
            wire.try_recv().is_err(),
            "No duplicate confirmation while awaiting acknowledgment"
        );
        assert_eq!(
            view.read_with(cx, |v, cx| v.editor.read(cx).value().to_string()),
            "尚未发送的草稿"
        );
        assert!(view.read_with(cx, |v, _| v.conversation.permissions.contains_key("q")));
        view.update(cx, |v, cx| {
            v.conversation
                .apply(&json!({"type":"permission_resolved","id":"q"}));
            cx.notify();
        });
        cx.run_until_parked();
        assert!(!view.read_with(cx, |v, _| v.conversation.permissions.contains_key("q")));
    }
    #[gpui::test]
    fn long_questions_remain_scrollable_in_a_small_window(cx: &mut TestAppContext) {
        let _home = Home::new();
        cx.update(gpui_shell::init);
        let(view,cx)=cx.add_window_view(|window,cx|{let mut v=fixture(window,cx);v.conversation.permissions.insert("long".into(),json!({"name":"AskUserQuestion","input":{"questions":[{"question":"需要查看完整问题。".repeat(60),"options":(0..8).map(|i|json!({"label":format!("方案 {i}"),"description":"这里是完整的方案说明，窄窗口下也能滚动阅读。".repeat(3)})).collect::<Vec<_>>()}]}}));v});
        cx.simulate_resize(size(px(540.), px(560.)));
        cx.run_until_parked();
        let panel = cx.debug_bounds("chat-approvals").unwrap();
        assert!(panel.size.height <= px(210.));
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: panel.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-5000.))),
            ..Default::default()
        });
        cx.run_until_parked();
        let button = cx
            .debug_bounds("chat-confirm-submit")
            .expect("confirmation button is reachable by scrolling");
        assert!(button.origin.y >= px(0.) && button.bottom() <= px(560.));
        assert!(view.read_with(cx, |v, _| v.conversation.permissions.contains_key("long")));
    }
    #[gpui::test]
    fn confirmation_uses_split_pane_height_instead_of_the_outer_window(cx: &mut TestAppContext) {
        use gpui::prelude::*;
        use gpui::{Entity, IntoElement, Render, div};
        struct Split {
            chat: Entity<ChatView>,
        }
        impl Render for Split {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    .child(div().w(px(340.)).h(px(320.)).child(self.chat.clone()))
            }
        }
        let _home = Home::new();
        cx.update(gpui_shell::init);
        let(root,cx)=cx.add_window_view(|window,cx|Split{chat:cx.new(|cx|{let mut v=fixture(window,cx);v.conversation.permissions.insert("split".into(),json!({"name":"AskUserQuestion","input":{"questions":[{"question":"完整问题。".repeat(40),"options":[{"label":"继续"},{"label":"等待"}]}]}}));v})});
        cx.simulate_resize(size(px(1100.), px(900.)));
        cx.run_until_parked();
        let chat = root.read_with(cx, |v, _| v.chat.clone());
        assert!(chat.read_with(cx, |v, _| v.pane_height.get()) <= 321.);
        let panel = cx.debug_bounds("chat-approvals").unwrap();
        assert!(panel.size.height <= px(90.));
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: panel.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-5000.))),
            ..Default::default()
        });
        cx.run_until_parked();
        let button = cx.debug_bounds("chat-confirm-submit").unwrap();
        assert!(button.origin.y >= px(0.) && button.bottom() <= px(320.));
    }
}
