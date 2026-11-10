# herdr-app: drive the person's window

Herdr shows spaces, tabs and panes. herdr-core (`hc`) runs their terminals
and keeps them alive when the window closes. Use `herdr-app` only to change
what the person sees. For starting, reading, messaging, waiting on or cleaning
up agents and programs, read `hc --skill` and use hc.
Your terminal address is `$HC_TERMINAL_ID`; it is not a pane ID.

## Inspect before acting

```sh
herdr-app ls          # spaces > tabs > terminals, including pane IDs
herdr-app status      # current layout, focus, overlays, zoom and file/mod errors
herdr-app commands    # available commands, arguments and descriptions
```

`ls` and `status` talk to the running window, not a new app. `commands`
prints the built-in command list even without an open window. Do not launch bare
`herdr-app` to inspect it: that opens or raises the window. Do not stop,
signal or restart the person's app or core. A window closing does not kill
its terminals.

`app.check_updates` checks for newer releases. Settings reports "Up to date"
only after a successful check; on failure, check the connection and retry.

## Show something when asked

New spaces and tabs are shown immediately: only create them when asked.
Pick an explicit, useful name for every new space and tab; don't rely on
default names. Use IDs from `ls` rather than guessing or acting on the wrong
focused pane.

```sh
herdr-app do space.new --name api --folder /path/to/api
herdr-app do tab.new --name "fix login" --argv '["claude"]' --task "Look at the login bug"
herdr-app do pane.split --direction right --pane p12
herdr-app do agent.focus --terminal t7
```

For a remote space:

```sh
herdr-app do space.new --name "remote api" --machine vps --folder /srv/api
```

The folder is on that machine; tabs and splits in the space stay there.
New-space keys and clicks follow the focused space's machine. Script commands
without `--machine` still use this computer; `--machine ''` explicitly chooses it.
The agent menu checks and starts agents in the remote Unix user's
login/interactive shell (sh, bash, zsh, dash, ksh or fish), including its
configured PATH. Other shells keep the sh launcher.
`terminal.open --folder /path --place down` starts the split in that folder,
including when `--argv` supplies a program. Without `--folder`, it follows the
focused terminal's folder.
`hc machine ls` lists machines. Use `hc run --on vps ...`, `hc exec --on vps ...`
and `hc file ... --on vps ...` for remote work, not your local file tools.

`do` accepts `--key value` arguments (JSON for arrays/objects) or one JSON
object. Unknown argument names fail and list valid names. Target IDs (`id`,
`pane`, `space`, `terminal`) must be strings; other non-null types fail before
the current-target default can apply. Null keeps its omitted-argument meaning.
Use `commands` to confirm the command's exact shape:

```sh
herdr-app do pane.split '{"direction":"right","pane":"p12"}'
```

It prints the created IDs (for example `{ "pane": "...", "terminal": "..." }`)
or `{ "error": "..." }` and exits 1. Read the result, then inspect `ls` or
`status` to confirm. Commands invoked by an agent never open a missing-argument
dialog: supply all required arguments. A successful launch reply means the
terminal was created, not that its program started correctly. If it exits
with a nonzero code within two seconds, the window shows an error toast with
the exit code and last output lines.

A terminal started with hc opts into the window with `herdr.space=<id>`.
`herdr.tab=<id>` chooses its tab; without it, Herdr creates a tab in that space.
Unlabeled workers stay hidden. Do not add labels just to run a background
worker. Use app commands when the
user wants to see a new tab or split, hc for orchestration that need not change
the screen.

## Customize layouts, keys, or mods

First read `~/.config/herdr-app/AGENTS.md` (or `$HERDR_APP_HOME/AGENTS.md` if
set). It is the installed guide to settings, layouts, commands and mods.
Read the generated `.d.ts` files beside it for full API details; do not edit
those files or the generated guide. The app rewrites them on start.

- Home defaults to `~/.config/herdr-app`; `HERDR_APP_HOME` selects another.
- Mods live in `mods/<name>/` with `main.js` and optional `mod.json`.
  They are trusted JavaScript with command and network access.
  `"focusable": true` in `mod.json` lets `pane.go` move onto a copy; it receives
  `"focus"` from `nextCommand()` and focuses its own keyboard handle.
- Shipped mods belong to Herdr: duplicate with `mod.clone` before editing.
  Shipped layouts likewise use `layout.clone` for a personal copy.
  Discover required arguments with `herdr-app commands`.
- Save to reload. A broken save keeps the old version running and reports
  an error in `status`; a still-working screen is not proof the edit worked.
- Make only the requested change. Don't touch unrelated mods or settings.
- Use theme colors. Do not retain a mod's `cx` across callbacks; each
  callback receives its own. Native animations, not redraw timers, keep
  the window cheap to run. Root `on_size_change((size, cx) => …)` reports
  logical pixels after layout, including resizes without pointer movement.
  Optional `View.close(cx)` gets one bounded final call; start cleanup there,
  not in an earlier task. The installed guide explains its 250 ms quit budget
  and which cleanup commands go out before shutdown.

After every mod change:

```sh
herdr-app check ~/.config/herdr-app/mods/<name>
herdr-app status
herdr-app shot /tmp/mod.png   # then look at the picture
```

`check` loads the mod and renders it once. It exits nonzero with the exact
error. Never call a mod done without a passing check. Then confirm it is
shown correctly in the running window with `shot`; don't claim a visual
check you haven't made.

`herdr-app shot [file.png]` saves the window's last frame and prints its
absolute path (default: `herdr-shot.png` in your current folder). It is the
app's own rendering at full pixel size: terminals, images, overlays and
floats included, even when the window is covered or minimized. No screen
capture or permissions. It needs a running window that has drawn a frame;
`HERDR_APP_HOME` selects the window, as for `status`.

## Problems and safety

If Herdr misbehaves, `herdr-app feedback "<what happened>"` drafts a report.
The person reviews and sends it; it does not silently send feedback.

Never experiment with pointer/key automation on the person's real display.
When developing the app on Linux, its `scripts/sandbox` provides an isolated
home, core and display: `up`, `do`, `status`, `shot`, then `down` when done.
Never stop a server or session you didn't start.

## Native Forge conversations (personal fork)

`chat.status` inspects connection and history; `chat.send` queues a message.
`chat.open` opens a conversation whose input, Markdown history and scrolling
stay local. Claude runs as a normal interactive CLI on the selected SSH host;
the app receives structured hooks rather than terminal frames.

```sh
herdr-app chat-install my-forge-host
herdr-app do chat.open '{"host":"my-forge-host","folder":"/home/forge/project","model":"opus"}'
herdr-app do chat.status --pane p12
herdr-app do chat.send '{"pane":"p12","text":"Explain this project"}'
```

The selected folder is trusted for this chat. Remote prerequisites are Node.js,
Python 3, and Claude Code with MessageDisplay hooks (validated with 2.1.312).
Use the remote user's existing Claude login; do not copy credentials into the
app. Normal user/project settings remain enabled. New chats create separate conversations. To move an existing Claude conversation,
use `chat.open` with `resume` set to its original Claude session UUID after
exiting its idle CLI. History is imported automatically, and the remote bridge
refuses to start a second writer while that conversation is still running.
The host and working folder must match the original conversation.

Choose **Install service**, then **Connect** in the chat header if needed.
Enter sends; Shift+Enter inserts a newline. **Latest** returns to live output.
Tool approvals stay pending until answered, cancelled, or expired after ten
minutes. Closing a tab or losing SSH leaves the remote conversation running.
The app caches history and drafts under `data/forge-chat/` in its own home.
Use a new chat to change hosts or working folders.

Text arrives by completed lines. AskUserQuestion answers are supplied through
a PreToolUse denial reason, since the interactive CLI's question widget is
terminal-only; the actual answer is shown to Claude without running that widget.
Quota savings compared with headless mode have not been measured.

Native Claude chats and local `herdr agent attach` clients appear in Agents
with their live state. Chat rows use `chat:<pane-id>` addresses; click the row
or use `pane.focus` to open the original pane. `agent.next` / `agent.prev`
include chats. Native chat rows do not expose terminal-only pin controls.
`herdr-app ls` includes an `agents` array matching the sidebar. A disconnected
status source is shown as needing attention, never as a cached idle state.

The native Claude composer offers `/` command completion and `@` remote file
references. Tab or Enter accepts a completion; a file choice stays in the draft.
Supported local commands are `/help`, `/model`, `/effort`, `/status`, `/stop`,
and `/new` (`/clear` is an alias that keeps the original chat). `/compact` and
discovered project/user/plugin commands execute in the interactive CLI. Native
model and effort selectors apply to this conversation while it is idle, retaining
its original session ID; they do not start a headless or SDK conversation.
Connection details and a per-chat Update service action are in the header.
Updates refuse active turns or pending approvals and preserve conversation history.
File completion uses Git (including submodules), with bounded local caching and
remote search for paths beyond the initial cache. It lists paths, not file contents.

The native chat composer grows to six lines, then scrolls its draft locally.
When the draft is focused, Page Up/Down and text navigation stay in the draft;
conversation history can be scrolled with the pointer over the messages.

Native remote Claude chats start and resume with full tool access
(`--dangerously-skip-permissions`), as configured for this personal Forge fork.
Questions asking for an actual user answer still require input. The chat header
shows the current activity and warns after 90 seconds without progress; a live
connection alone is not proof that Claude is making progress.

Scrolling up in native chat pauses automatic following. Background status
updates and new messages preserve the reading position; Latest, sending a new
message, or scrolling back to the bottom resumes following.

Native Claude agent attention is reserved for live questions and permission
requests. Reconnecting, stopped, and execution errors use separate neutral badges;
transport loss does not mark a running task as completed.

Unread Done states are saved in agent-attention.json and have no expiry.
Native chat clears Done only when the active window shows its latest result;
reading earlier history does not acknowledge a new completion.

Native Claude chats show a usage line below the composer: model, git branch,
context percentage, five-hour and seven-day quota usage with reset countdowns,
and estimated session cost. Details include token counts, cache reads, duration
and changed lines. Missing metrics stay unknown. These are native Claude CLI
status-line facts; estimated cost is not an additional Pro subscription charge.

Native Claude chat supports pasting screenshots or copied PNG/JPEG/TIFF/BMP files into the editor. Click thumbnails for a full preview or × to remove a draft attachment. Sent images use bounded, aspect-preserving previews above the message text; multiple images wrap in narrow windows. A message can contain up to four images (8 MB per normalized PNG, source at most 64 MB and 8192×8192). Images upload over SSH into that conversation’s private image directory before sending. Image-only messages work. Attachments persist with drafts/outbox across restarts; failed uploads keep the queued message for retry. The SSH client adapts older services without restarting their Claude process. `chat.status` includes `draft_images` and `image_preview_open` for diagnosis.

Super Agent is fixed above the Agents list. `supervisor.open` opens its single persistent native local Claude chat. `supervisor.list` discovers local/remote and manually created sessions without marking them read. `supervisor.read {address,offset?,limit?}` reads native messages without changing focus; `supervisor.send {address,text,request_id,mode?}` defaults to queuing follow-ups behind current native work without touching drafts. `supervisor.create {host?,folder,agent,title,request_id}` creates a worker without selecting it (Claude: a host already used by a session, an explicit SSH config alias (including Include files), or `@local`; Codex: existing core machine or `@local`). Manager tool calls use durable request receipts; uncertain delivery requires inspection, not automatic resend. The manager's task ledger keeps pending reviews separate from worker idle and connectivity.

`local_chat_proxy` in settings.json optionally sets a proxy for local native chat processes only; it does not change system, Git, remote SSH or core settings. The Super Agent backend selector defaults to Codex app-server, with Claude available; task ledger and worker sessions survive switching.

`supervisor.answer {address,question_id,user_intent_id}` forwards the newest explicit user message verbatim to a pending native AskUserQuestion, without focusing the worker or approving its tools. The manager can use reply_to_question; it cannot authorize arbitrary tool approvals. Pending task acceptance/resumption also requires a newer explicit user intent.

`supervisor.provider {provider:"codex"|"claude"}` requests a backend change while the manager is idle; its composer also offers a switch. Workers and the shared task ledger are retained.

Local Claude can reuse an existing SSH host subscription via settings.json `local_claude_auth_host`; credentials remain in subprocess memory. Codex Super Agent model/effort choices come from the local app-server catalog and apply to subsequent turns.

Super Agent inspection uses `inspect_sessions` to batch worker reads. `read_session` includes provenance, content age and connectivity; legacy attach clients read their original Herdr source without scrolling, and native journals remain available offline.

Super Agent supports durable reminder tools (create/list/cancel/acknowledge), offset-bearing due times and once/daily/weekly recurrence. The local clock never executes reminder text. macOS background delivery requires the user to be logged in and awake; overdue reminders catch up and remain in Needs you until explicitly acknowledged.

`reminder-notify <json>` sends through Herdr App's native notification identity; `supervisor.reminder_notify` is its running-window command. Notification activation opens Super Agent, without acknowledging a reminder or executing its text.

Local Codex workers use their existing app-server Unix WebSocket for structured messages, tool results, runtime status, and complete pending questions. Super Agent displays question cards with full option descriptions and common command/file approvals. A human can answer those cards here, or give a newer text reply for `reply_to_question {address,question_id,user_intent_id}`. Text is forwarded verbatim to the exact pending request; approvals require explicit buttons. Source thread/process identity is revalidated before every reply. Ambiguous older TUI bindings, disconnected runtimes, secret input and unsupported approvals cannot be answered by guessing. No worker restart or terminal scrolling is involved.

Super Agent uses adaptive follow-ups: after the initial idle_minutes (default 15), the manager calls plan_followup {after_minutes:1..10080|null,publish:boolean,reason} during each observation to choose its next check, or wait for the human/new evidence. It backs off when ignored and avoids repeating unchanged questions. publish=false keeps the periodic inspection silent (no chat output or banner). Decisions persist in the manager journal; new human messages or changed session evidence invalidate them. Worker tasks remain read-only; only this own follow-up clock can be changed autonomously. Existing event alerts and explicit reminders remain independent. interval_minutes is retained for compatibility and no longer sets a fixed repeat cadence.

Local Codex send_message resumes the exact existing runtime thread with structured turn/start, validates live source/process identity and queues while busy, and returns thread/turn IDs. It preserves existing runtime settings and never types into the outer hc/Herdr attach terminal. Request receipts prevent duplicate dispatch; unknown delivery remains unknown and cannot be resent automatically. Pre-send refusals leave the request retryable with the same ID. Dispatch failure never authorizes creating a replacement worker; keep the original session and explain/repair the blocker, or obtain explicit replacement authorization.

Native chat shows tools as quiet, borderless activity lines. A single tool opens its details in one click; multiple consecutive calls expand into a shared list. Shell inputs show the command while copying details retains the full request; failures remain visible on the collapsed group. Active tools show an existing description/command/path, a reduced-motion-aware pulse and elapsed wall time. Completed timings freeze; imported records without reliable timings remain unknown. No model call generates these labels. Questions/approvals use compact confirmation cards with wrapping choice buttons, selected-option explanations, optional custom answers, a full-request disclosure and an explicit one-time submit. Minimizing a card does not answer/dismiss it; drafts and Needs you remain. Duplicate submits are blocked until acknowledgment, and selecting an option never submits automatically. Tool details and long questions scroll within bounded panels; grouping changes presentation only, not saved transcript/tool IDs or history-read APIs. chat.status also reports history_rows/tool_groups/open_tool_groups/sending_decisions.

Chat completion menus (model, effort, `/`, `@`) float above the composer without resizing history. Current model/effort is marked. Scrolling away from the live tail reveals a contextual “回到最新” button in the history area; selecting it resumes following. Grouped activity counts file reads, commands, edits, and searches separately. Explicit TodoWrite/update_plan steps are shown with their reported statuses; no plan or completion is inferred for other tool schemas.

`supervisor.close {address, request_id, expected_identity, confirmed:true}` archives one exact app session; `expected_identity` is `{conversation,host}` for native chat or its current `read_identity` for terminal sessions. Optional `user_intent_id` must match the latest human message. The Super Agent tool `close_session` requires that intent and resolves identity itself; it must run directly rather than delegate to a sandboxed worker. A receipt with `pending:true` is unfinished; report closed only after `verified:true`. Histories, work files and the underlying execution runtime are preserved. The closed entry and original pane content are retained in `data/super-agent/closed-sessions.json`; this is not process termination. `supervisor.list` includes `closed_sessions` with `pane_present`, and closed identities are not automatically rediscovered as managed sessions. Super Agent cannot close itself.


### Persistent decision sidebar
`supervisor.decisions` reads the Super Agent decision inbox. `supervisor.decision {"id":"question:stable-id","title":"Choose the next step","context":"Complete question and relevant context","address":"t10","options":["A","B"],"evidence":"message-id","decision_key":"choose-next-step","reason":"The owner must choose between distinct scopes","source_ids":["msg-example-question"]}` upserts an observational card without erasing the human's response/completion. No worker action is dispatched. Manager tools `list_decisions` / `record_decision` use the same canonical store. Never include credentials. Reuse the same ID for repeated observations of one question.
The right rail preserves pending/answered decisions; the agent may withdraw an unanswered obsolete card with an explicit reason, while answered cards stay until the human handles them. An answer is explicitly submitted into the manager's durable queue, independent of the main editor draft; it does not approve worker tools or prove action completion. Open the original session from its card. Completed cards can be reopened. In narrow panes use 待你决策 to show/hide the overlay; the rail scrolls independently of chat history.


### Agent-selected decision cards
The task ledger and decision inbox are independent: `record_task` never creates cards.
Super Agent reads current questions, existing cards and prior human replies, then
chooses whether a concrete human choice is necessary within the existing mandate.
`record_decision` requires decision_key, reason and exact source_ids. Renaming a
question preserves its canonical card; shared evidence triggers a collision that
requires updating/merging the existing card or explicitly explaining separate
questions with distinct_from. Human answers/completion are never overwritten.
`curate_decision` uses `supervisor.decision_curate` with action merge/withdraw,
id, reason and optional duplicate_ids. It can consolidate duplicate pending
cards or withdraw an obsolete pending card; originals remain archived with an
explanation. It cannot change answered/completed cards, accept work, or act on a
worker. These observational tools are available during background check-ins.
The sidebar's 已归档 tab includes human-completed, merged and withdrawn records,
with separate status labels. Merged cards cannot be reopened as duplicates;
withdrawn cards can be reopened explicitly by the human.


The Super Agent header's **检查全部会话** button queues one manual read-only
review behind any active manager turn. Repeated clicks while queued/running
coalesce; the composer draft and attachments are kept. Inspect every local and
remote session with list_sessions and inspect_sessions in batches of at most 24,
using read_session for missing evidence. Report actionable questions, reviewable
results, blockers and remaining progress; explicitly identify stale/unavailable
sources. Unlike periodic check-ins, [Manual session review] always produces a
visible report even when nothing changed. Inspect existing decisions and human
answers before curating cards. The click does not authorize worker dispatch,
question replies, acceptance, approvals, merges or deployment, and does not
replace the latest typed human mandate. Interrupted reviews are marked ended,
not automatically replayed; the button allows an explicit retry.

Native worker dispatch preserves explicit pre-send refusal reasons even when
the CLI exits 1. Invalid input or missing session is a refusal, not a transport
failure: nothing was sent and the same request may be retried after repair.
Retries must retain the original address, text and request_id exactly. Window
timeouts/transport failures remain delivery-unknown and cannot be auto-replayed.


Follow-ups no longer require an idle native worker. send_message defaults to
mode=queue: native Claude queues behind its active turn while preserving the
composer draft/attachments and pending approvals. Local Codex queues durably in
the original runtime binding, waits for active work and decisions to finish,
and sends the next turn once idle. Queue receipts mean accepted, not started.
Read dispatches in read_session for updated queued/confirmed/unknown delivery.
A queued Codex follow-up survives observer restarts; identity changes refuse it
rather than targeting a reused pane. Timeout after submission is never replayed.
For an authorized immediate correction to LOCAL Codex, explicitly choose
mode=steer: use the verified current turn/steer with expectedTurnId; do not
interrupt or override worker settings. If already idle, it starts the next turn.
Steer never answers pending questions or tool approvals. Capable native Claude chats support interactive steer with a verified active
turn; legacy terminal adapters do not support steer: report that clearly,
never silently turn steer into queue. The legacy remote terminal path remains
idle-only; native remote Claude uses its daemon FIFO instead of terminal input.
Continue the exact original session and retain the original request ID/text on
retry. Never automatically replay historical delivery-unknown assignments.


Super Agent manages canonical goals in `data/super-agent/goals.sqlite`, with
a generated atomic `goals.json` projection for the right sidebar's 目标 tab.
It shows the three highest-priority goals first; 其余目标 expands the remainder.
Cards show current progress/next action and expandable mandate/exclusions.
The decision inbox and archived human answers remain separate.

Management tools: list_goals reads goals and reconciled receipts; record_goal
links legacy task IDs to a stable goal and stores its outcome, cited HUMAN
mandate, allowed action classes, exclusions and exact-worker continuation plan.
Only a current human turn can change a mandate, plan or enabled/pause state;
background discovery can record an unenabled goal with no mandate. Historical
worker messages cannot grant authority. Attach legacy snapshots instead of
creating a new goal per report. Original ledger rows remain unchanged and are
returned with historical_snapshot/not_a_human_gate and their goal_id.

record_goal_progress updates current facts using source_ids, observed_at and
expected_revision. Older or concurrent updates are rejected. named milestones
need evidence and achieved=true, not merely a queued/merged/deployed claim.
needs_user=true requires why_user explaining the actual unresolved choice.
observed_complete is an observed postcondition; it does not accept work for the
human. Updating objective facts does not need another “继续” or a new grant.

advance_goal {goal_id,step_id} queues only a stored, cited, authorized step on
the same worker identity after verified milestone dependencies. It accepts no
arbitrary new instruction/action/address. Step receipts are durable across
restarts; confirmed/unknown steps never replay. Definite pre-send refusals may
retry the same immutable step after a backoff. Receipt reconciliation reads
actual native prompt_confirmed or Codex dispatches; accepted input is not task
completion. Pending tool approvals are never answered by this mechanism.

Periodic/session-event observations may record goal progress and advance a
ready authorized stored step. Raw send/create/reply/approval tools remain
blocked there. Manual 检查全部会话 stays observational and cannot advance goals.
The active-observation context also guards plan/mandate edits in Python. An
explicit pause stops future goal dispatches and does not claim to retract
already accepted worker input. Plan, verified prerequisite or human-gate changes wake the manager once;
ordinary summary, timestamp and projection writes do not create a self-triggering
observation loop. A manual inspection that verifies a prerequisite can therefore
be followed by a separate bounded background continuation without another user
message.

The Super Agent tool `release_session {address,request_id,user_intent_id}` is
separate from `close_session`: use release when the human requests stopping
and freeing a session, and close for archive-only requests. Release requires
the latest explicit human authorization and may target `closed_sessions` too.
It first verifies archival, then stops the exact original runtime. Native
Claude release checks conversation-owned daemon/runner arguments, captures
their process trees, checks process start identity before signalling and
verifies exit; it never kills a shared SSH connection or shared Codex server.
Core terminals use the core's exact-address kill and verify matching identity
and finished state. Transcript and work files remain. Reconnects cannot
automatically recreate a released native conversation. Only report resources
released when `released`, `runtime_stopped` and `verified` are all true.
Partial/unknown outcomes are not success and are not blindly retried.

Native Claude steering: while a capable worker is working, Enter or “调整 ↑”
uses the existing interactive CLI sendNow action (Ctrl+X Ctrl+S). “排队”
retains the after-turn FIFO behavior. Stop remains separate. Native
`supervisor.send {address,text,request_id,mode:"steer"}` / Super Agent
`send_message` validates the active turn, availability and approval state;
`mode:"queue"` remains the default for delegated work. `supervisor.list`
exposes `steer_available`; `supervisor.read` includes per-request `dispatches`
and `active_turn`. A queued transport receipt is not delivery: only the exact
UserPromptSubmit hook confirms steering. Do not automatically resend unknown
steering input. Pending approvals/questions remain unanswered. Slash/shell
commands use the normal queue. Older running bridges need an idle reconnect
before advertising this capability; never interrupt a worker merely to upgrade.

The updated native client automatically upgrades a legacy bridge only while
its exact worker is idle, all prior input is confirmed, no approvals are
pending, no extra background processes are present and this is its only
client. Input is held in the app outbox during that reconnect. The original
Claude session ID and transcript are retained. Ambiguous background agents or
another client defer upgrade; active work is never stopped for this update.

Super Agent uses layered collaboration memory. Private source records remain in
`data/super-agent/memories.sqlite`. A stable, query-independent summary contains
user preferences and a detailed-memory index; full records and quotes stay behind
`list_memories`, `search_memories`, and `read_memory`. Saying “继续” does not filter
out preferences. Codex receives the summary through a keyed application-context
snapshot; interactive Claude receives it through UserPromptSubmit hook context
on first use, version change, or a new session context. Human message text is
never modified. A changed/deleted note invalidates generated summaries immediately;
all adapters read the authoritative store, so stale projection files cannot revive
old preferences. Current human instructions still win; memory never grants
operational authority or replaces fresh task evidence.

Quiet idle maintenance uses `review_memories` to inspect a bounded batch of real
human feedback, learns source-cited durable lessons with `record_memory`, then
`consolidate_memories` builds a deduplicated profile/preferences/index overview.
The summary must cover all current note IDs and preserve manually edited wording.
Background extraction cannot overwrite manual/newer notes; transient task commands
and worker claims do not become preferences or permissions. Maintenance has its
own cooldown, sends no routine chat report, does not change progress-report timing,
and cannot dispatch workers, answer questions, change goals, approve, or deploy.
The deterministic current-note overview is available while consolidation is pending;
large overviews are explicitly marked shortened and route to the full paginated
index. Exact quotes remain in the source records, not in the default summary.

Direct memory management: click **记忆** in the Super Agent header to open the
**协作记忆** sidebar. **＋ 新增记忆** opens a title and multiline content editor;
**编辑** prefills the existing note; **保存** writes directly to the same local
SQLite store without a model turn or chat connection. **删除** asks for
confirmation, then removes the note directly. Sources distinguish manual edits
from learned human feedback. Saves and deletes check the displayed revision;
conflicts/errors retain the editor draft. Your chat composer remains untouched.
Conversation-based memory tools remain available, with their existing source
validation; direct UI editing is not exposed as an additional manager tool.

Memory UI has **当前摘要** (effective summary, consolidation state) and
**详细记忆** (editable records with collapsed source quotes). Add/edit replaces
the list with a separate bounded form; it does not overlay other cards.
`chat.memory {view:"summary"|"records"|"new"|"edit", id?, pane?}` opens a native
memory view only, never saves or deletes. It refuses to discard unsaved memory
edits. `chat.status` includes `memory_records` and `memory_summary_compiled`.

Goal progress reviews: `configure_checkins {goal_review_minutes:60,...}` controls
a mandatory published goal review independent of adaptive wait/terminal changes.
Fresh goal/worker evidence is required; failed/incomplete checking is reported as
a gap, never completion. Only existing authorized goal steps may advance. Busy
turns defer and missed intervals coalesce; disabled reporting/paused goals stop it.

Goal worker follow-up: a bound original worker quiet for 15 minutes generates a
`goal_session_quiet` observation. `ask_goal_session` verifies fresh identity,
idle state/history and absence of human gates, then sends a fixed progress question
once per quiet episode. It never replaces a worker or expands authority. Busy,
blocked, paused, completed and disconnected cases are excluded; unknown delivery
is not replayed. Replies feed the existing goal-progress/continuation workflow.

目标卡片的“删除目标”需要再次确认，可取消。删除后停止该目标的后续跟进与派发，保留会话、历史、审计及已发送回执；已派发的工作不会被终止。旧后台更新不能恢复已删除的目标。
