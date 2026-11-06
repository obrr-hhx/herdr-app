# Super Agent

Super Agent is the fixed first row above the Agents list. Opening it selects one
persistent conversation; it does not create another manager. Ordinary sessions
keep their existing status, ordering, unread markers, panes and manual controls.
The compact sidebar keeps the same entry as a star.

The manager defaults to Codex, using the signed-in local Codex CLI's app-server.
Its composer can switch to Claude. Each backend retains its own model context;
the visible conversation and the task ledger are shared. A changed Codex tool
schema starts a fresh backend context and preserves the old thread reference.
Switching is refused while a turn is active; stop the turn before switching.

The manager discovers native Claude conversations and core terminal agents on
local and remote machines, including sessions made by the person. Discovery is
not authority to change a task. Its tools read bounded history, create Claude
or Codex workers and send assignments to idle workers. Native sends preserve
the person's text/image drafts, focus and scroll position. Terminal reads are
screen/history snapshots, explicitly distinguished from structured messages.
New native workers use an existing SSH host or `@local`; terminal Codex workers
use an existing core machine or the local core. Creation does not select the
new worker or obscure the manager.

A local SQLite ledger keeps task mandates, progress, evidence, pending questions
and results awaiting review. Worker idle, transport reconnection and user
acceptance are different facts. Clearing a pending review requires a newer
explicit user intent; reading a result does not accept it. The manager can relay
a new user reply verbatim to a native AskUserQuestion. It cannot use that path
to approve tools. Existing manual approval controls remain in the worker pane. New local Claude
workers inherit the local permission profile; the established remote full-access
mode is unchanged. The manager itself exposes only bounded management tools.

The supervisor filters heartbeats and repeated states, batches semantic changes,
and only wakes the model for discovery, results, questions or persistent faults.
A transient reconnect is quiet; a persistent outage is reported once. Distinct
question IDs are tracked even while a worker remains blocked. Core turn counters
catch terminal results even when a fast working-to-idle transition was not
visible in the GUI. Source cursors,
observations and task state survive restart. Unfinished observation handling can
be recovered, but earlier user prompts are not automatically resubmitted.
Receipts prevent duplicate sends with the same request ID. Unknown delivery
requires inspecting history; a refusal before sending can be corrected safely.

The GUI publishes the current inventory. Remote workers continue without the
window, while the manager stops processing stale inventory when the GUI is
closed. Reopening reconnects history and reconciles new events. This is not an
always-online server independent of the Mac and GUI.

`local_chat_proxy` optionally configures local service subprocesses; corporate
and loopback destinations stay direct. No global Git/network settings or
credentials are changed. Local services explicitly include common user CLI
locations in PATH so Dock launches can find Codex, Claude, Node and Python.
A short private Unix socket path points at durable conversation storage. Helper
installation is atomic and safe across simultaneous local conversation starts.

## Verification on macOS, 2026-11-05

An isolated app home and core verified the fixed entry, compact sidebar, singleton
opening, restart, state retention and real manager tool calls. Real Codex created
local Claude/Codex workers; local Codex returned the requested marker. A manually
created remote Claude worker was discovered, received a manager assignment and
returned the requested marker. The manager also created a new remote Claude worker,
kept its own pane focused, and verified that worker's requested response. A remote AskUserQuestion was relayed to the
manager, answered there, and completed with the matching answer marker. Pending
results remained awaiting review. The manager accurately reported a local Claude
server error rather than claiming success.

Codex/Claude switching preserved worker identities and the shared ledger. The
local Claude backend initialized, but its existing company Gateway configuration
returned server errors during model calls. Direct personal-account availability
was checked without changing settings and was not logged in. This account/routing
limitation remains separate from the verified Codex-to-remote-Claude workflow.

Eight simultaneous local bridge installs succeeded. A deliberately slow CLI
that ignored SIGTERM verified bounded shutdown and delivery of a message
queued during an idle backend restart. Node protocol tests cover
streaming, bounded management tools and Codex thread resume; Python tests cover
request reuse, uncertain delivery, definite refusal, task-review persistence and
self-delegation prevention. No production user app/core or existing user worker
was stopped for acceptance; only isolated test processes were used.

## Local verification commands

The local build used Rust 1.97.1, Codex CLI 0.166.0 and Zig 0.16.0. The repository
CI pins Rust 1.95.0; no GitHub CI or publication is claimed by this local acceptance.
The default parallel Cargo run once hit three existing filesystem-watch timeouts;
the complete suite was then run serially without changing those tests.

```sh
ZIG=/path/to/zig cargo test --locked -- --test-threads=1
node --test bridge/*.test.mjs
python3 bridge/supervisor_test.py
bun test scripts/files.test.ts scripts/picker.test.ts scripts/agents.test.ts scripts/spaces.test.ts scripts/docs.test.ts
rustfmt --edition 2024 --check src/supervisor.rs src/chat/mod.rs src/chat/state.rs src/chat/transport.rs src/chat/view.rs
python3 -m py_compile bridge/*.py
bun scripts/docs.ts check target/release/herdr-app
git diff --check
```

History passed to the manager and task notes redact recognized credential formats
and authorization URLs; credential-bearing assignments are refused. Request
receipts store argument hashes instead of plaintext assignment bodies.

Final local acceptance: 281 Rust unit tests passed (1 ignored), plus integration,
GPU and CLI manual tests; 30 Node tests, 7 Python management tests and 21 Bun tests
passed. After switching back to Codex, a message sent during startup produced
`SWITCH_QUEUE_OK` on the same backend thread. A fast terminal completion was
subsequently discovered and recorded as awaiting review without resending it.
The signed release binary was installed into the existing Herdr App bundle,
without restarting the person's running app. Reopen the app to activate it.

Codex model and effort menus now use the app-server catalog, validate advertised
efforts per model, and apply selections to subsequent turns on the same thread.
Each provider retains its own last selected model and effort. `local_claude_auth_host`
selects the existing SSH source for a local Claude subscription token; no token is
stored locally or passed in command arguments. Startup requires the source host
to be reachable. Inherited `CLAUDE_CODE_OAUTH_TOKEN` takes precedence.

Follow-up model/auth acceptance: 282 Rust unit tests passed (1 ignored), plus
integration checks; all 30 Node, 21 Bun, 7 supervisor Python and 3 local-auth
Python tests passed. A selected Codex model with high effort returned the marker,
and its selection/catalog survived app restart. Real local interactive Claude
and the Claude manager both returned their requested markers using the existing
Team subscription token, without print mode. Test socket paths were shortened
on macOS to respect its Unix socket path limit.

## Evidence reads and overview follow-up

`inspect_sessions` reads up to 24 workers concurrently (8 bounded readers) from
an atomically published inventory, without waiting for the GUI thread. Individual
reads are bounded at 2.5 seconds per source call. Legacy Herdr attach terminals
are recognized from exact argv and read through their original pane's visible
source; this does not scroll, focus or send input. Visible output is explicitly
partial and is not claimed to be full Codex conversation history.

Native conversations read the locally synced journal even when SSH is unavailable
or a pane is not instantiated. Malformed lines are skipped with a count, partial
appends are ignored, and the original file is unchanged. Reads carry content time,
source, partial limits and connectivity separately. An offline or stale inventory
never establishes current idle/working state. Redacted successful reads are cached
privately; a later failure can reuse evidence only for the same source identity.
A bad session does not hide the other results. Bounded excerpts preserve recent
message IDs without overflowing the management protocol.

On the person's real inventory, all 10 workers returned evidence in about 70 ms:
three original Herdr visible sources and seven native local journals. A separate
real Codex overview used only `inspect_sessions`, prioritized concrete questions,
reviewable results and monitoring gaps, and did not create or message workers.
Six Python regression cases cover source routing, disconnected journals,
malformed/partial records, cache identity, concurrent failures and UTF-8 bounds.
Forge control-plane running is separate from SSH reachability; the environment
with an independent SSH handshake failure remains last-known history only.

## Durable reminders

The manager exposes schedule_reminder, list_reminders, cancel_reminder and
acknowledge_reminder. Creation requires the latest human request, a stable ID,
an explicit offset-bearing date and an IANA timezone; once, daily and weekly
recurrence are supported. Relative dates use list_reminders.now_local rather
than a remembered date. Reusing a request ID does not duplicate or overwrite a
reminder. Acknowledgment requires a later human response or the UI button;
merely reading the reminder is not acknowledgment.

SQLite commits a due occurrence before any notification. Both clocks share the
same occurrence key. Overdue recurring jobs coalesce into one catch-up delivery
and advance to the next wall-clock occurrence, avoiding a wake-up notification
flood. Delivery survives process restart, network loss and GUI closure. A
per-user macOS LaunchAgent checks every 30 seconds while signed in and awake;
the manager service also polls independently of model turns. Sleep, power-off
and logout cannot deliver a banner; due reminders catch up on wake/login/reopen.
Other platforms retain local-service polling and restart catch-up.

System notifications are sent by the actual installed Herdr App bundle through
GPUI's UNUserNotificationCenter path, preserving Herdr's name and icon. When the
GUI is closed, reminder-notify runs a windowless native sender: it starts no
core, model or workspace. Notification activation opens Super Agent. The app
inbox remains Needs you until explicit acknowledgment; acknowledgment removes
its native notification. macOS controls whether requested banners are visible.
The clock never interprets or executes the reminder text, approves PRs, runs
/apply or dispatches a worker.

Acceptance used a real Codex tool call to create a one-shot reminder, a standalone
clock firing while the GUI was closed, and native GUI catch-up showing Needs you.
A busy-model test delivered and acknowledged a reminder without another model
turn. Store tests cover due boundaries, UTC/local conversion, recurrence/DST,
restart, concurrent clocks, cancellation, later-human acknowledgment, notification
failure and isolated homes. Test homes never install a real login item or send OS
notifications. The built-binary asset test imports the installed management tools
and scheduler so omitted helper files cannot silently break startup.

Local Codex workers use their existing app-server Unix WebSocket for structured messages, tool results, runtime status, and complete pending questions. Super Agent displays question cards with full option descriptions and common command/file approvals. A human can answer those cards here, or give a newer text reply for `reply_to_question {address,question_id,user_intent_id}`. Text is forwarded verbatim to the exact pending request; approvals require explicit buttons. Source thread/process identity is revalidated before every reply. Ambiguous older TUI bindings, disconnected runtimes, secret input and unsupported approvals cannot be answered by guessing. No worker restart or terminal scrolling is involved.

### Local Codex decision relay (2026-11-05)

The local adapter now observes the same app-server runtime as existing Codex
TUIs over its user-owned Unix WebSocket. It reads paginated structured items
and pending server requests; it never scrolls or types into a worker terminal.
For older TUIs without an explicit session ID, the initial binding requires one
exact live name/cwd match. Replies also revalidate the original terminal/process
and thread, and refuse ambiguous, reused or disconnected sources.

Validation used isolated homes and real Codex workers. A question created before
the observer connected was replayed with its complete body and option descriptions.
Both the `reply_to_question` tool and the manager's permission protocol submitted
B to that exact request; the original worker continued with `QUESTION_ANSWER_B`.
Reconnection retained the question, resolution removed it, and receipt replay did
not submit twice. An isolated native window showed the source label, full question,
option descriptions, submit button and Needs you state. Its Chinese question was
an interface fixture, not a decision sent to the user's real t2.

Checks: `cargo test --locked -- --test-threads=1`, `node --test bridge/*.test.mjs`,
all `bridge/*_test.py`, the five Bun script test files, Rust formatting of the
changed formatted modules, Python compilation, Node syntax checks, CLI docs checks
and `git diff --check`. Native validation was on macOS; other platforms were not
run. The local Rust toolchain was 1.97.1, rather than CI's pinned 1.95.0.

Super Agent uses adaptive follow-ups: after the initial idle_minutes (default 15), the manager calls plan_followup {after_minutes:1..10080|null,publish:boolean,reason} during each observation to choose its next check, or wait for the human/new evidence. It backs off when ignored and avoids repeating unchanged questions. publish=false keeps the periodic inspection silent (no chat output or banner). Decisions persist in the manager journal; new human messages or changed session evidence invalidate them. Worker tasks remain read-only; only this own follow-up clock can be changed autonomously. Existing event alerts and explicit reminders remain independent. interval_minutes is retained for compatibility and no longer sets a fixed repeat cadence.

### Adaptive proactive progress reports

After initial human inactivity (default 15 minutes), Super Agent inspects all
sessions and uses plan_followup to decide whether a report warrants interruption
and when to look again. after_minutes is 1..10080, or null to wait for a human or
new evidence; publish=false keeps the periodic turn out of chat and notifications.
There is no fixed repeating 30-minute loop. A missing decision falls back to wait.
The model is instructed to back off when ignored, distinguish ongoing work from
human-blocked review, and record a concrete reason. Decisions are journaled and
survive provider/app restarts. New human messages or changed source evidence
invalidate old plans; existing semantic event alerts still work independently.

Only the manager's own follow-up clock may be changed in these periodic turns;
worker tasks, approvals, task acceptance and reminders remain protected. The clock
waits for a free manager and a fresh inventory from the running app. Hidden
inspections buffer and discard chat/tool output unless publish=true, and do not
send a banner or mark a new visible completion. Explicit reminders are independent.
User preferences remain in data/super-agent/checkins.json; interval_minutes is
read for older clients but no longer controls adaptive follow-ups. Each inspection
uses the selected manager model's normal quota. Herdr supplies native banners.

Adaptive follow-up verification: fake-clock tests cover model-selected due times,
one-time consumption, indefinite waiting, user/evidence invalidation, restart
persistence, and migration from the previous fixed-interval state. Daemon tests
verify publish=false discards output both before and after the planning tool,
sends no visible completion, and falls back to waiting if no decision is made.
A real Codex manager used list_sessions, inspect_sessions and plan_followup,
reported all three fixture sources, then explicitly chose null with the reason
that it was waiting for the human/new evidence and should not repeat the question.
The initial human inactivity delay remains configurable; the former repeat
interval is no longer used for scheduling. Existing native reminders are unchanged.

Local Codex send_message resumes the exact existing runtime thread with structured turn/start, validates live source/process identity and idle state, and returns thread/turn IDs. It preserves existing runtime settings and never types into the outer hc/Herdr attach terminal. Request receipts prevent duplicate dispatch; unknown delivery remains unknown and cannot be resent automatically. Pre-send refusals leave the request retryable with the same ID. Dispatch failure never authorizes creating a replacement worker; keep the original session and explain/repair the blocker, or obtain explicit replacement authorization.

### Continue an existing local Codex worker

The t4 failure exposed a routing mismatch: its outer hc terminal was a legacy
Herdr attach client, while the real Codex worker was inside that pane. Reading and
question replies used the Codex runtime, but task dispatch still used hc send
with the outer terminal's idle guard. Local Codex dispatch now uses the validated
original runtime thread and turn/start, returning the acknowledged thread/turn
IDs without overriding the worker's model, permissions, cwd or other settings.

Pre-send refusals are explicitly retryable with the same request ID. A missing
wire acknowledgment keeps a durable unknown receipt, preventing automatic retries
or dispatch to a new worker. A send failure never authorizes replacing a session.
Existing unknown receipts from the old path remain unknown; absence of a new
transcript entry cannot prove the old send was never delivered.

Validation: an isolated real Codex thread remembered context from its first turn
when continued through the production supervisor send_message tool. Its attached
real TUI showed the reply and retained an unsent draft. Replaying the request ID
returned the same receipt, with exactly two total turns. Unit tests also cover
outer-terminal bypass, busy/reused source refusal and unknown-delivery protection.
The user's t4/t10 tasks were neither replayed nor interrupted for this test.

## Closing an app session

The manager owns `close_session`: it archives the exact entry through the app, retaining transcript, pane metadata, files and underlying runtime. It does not ask a coding worker to run Herdr CLI inside its sandbox, and never relaxes sandbox permissions. Native identities use conversation and host; terminal identities use PID and creation time, so address reuse does not hide a new worker. A pending pane change is verified before reporting completion. This operation is unavailable to periodic read-only check-ins and cannot target the manager itself. Closed entries are retained in `data/super-agent/closed-sessions.json`.

Local acceptance for `close_session` (2026-11-06): an isolated app/core archived
a native fixture and a reported terminal fixture through `supervisor.close`.
Both pane removal and inventory absence were observed; retry returned the same
verified outcome. A sibling native worker and Super Agent stayed present,
self-close and mismatched identity were rejected, the native transcript file
remained, and the terminal runtime remained alive after parking. Unit coverage
includes reused addresses, corrupt registries, exact-identity checks and retry
receipts. Adapter tests cover stale human intent, self-targeting and pending
verification. No sandbox permissions were expanded.

The owner's three requested entries were already absent at the first live close
attempt after the app update. Their original inventory identities were adopted
into durable archives with `observed_already_closed:true`; the new API verified
pane absence. The two native history files remained readable, and t3/t10 kept
their original identities. This live observation is distinct from the isolated
end-to-end test of removing attached panes.


## 待你决策
Super Agent 右侧保存需要你回答、选择或验收的事项。宽面板默认显示，窄面板通过「待你决策」展开。每张卡片有完整上下文、来源会话、证据和选项，可以补充答案后明确提交，也可以打开原会话检查。

- 待你决策：尚未回答。
- 已答复 · 待推进：回答已进入管家的持久发送队列，尚不等于远端执行或任务完成。
- 已完成：由你标为已处理；历史保留，可重新打开。

卡片不会因为会话空闲、完成、断连或时间经过消失。提交不会覆盖主输入框草稿。工具执行审批仍使用专门的一次性确认控件；标记处理只是整理清单。

`record_decision` / `list_decisions` 是管家的清单工具，通过 `supervisor.decision` / `supervisor.decisions` 操作 app 单一持久存储 `data/super-agent/decisions.json`。同一问题复用稳定 ID，重复观察只更新上下文、不抹掉回答与完成标记。`record_task` 的 waiting_user 会生成卡片；awaiting_review 的一般进展不自动堆入，由管家核实存在具体人类验收或决策后登记。后台观察允许登记已观察到的决策，但不能代用户回答、完成事项、审批或派发任务。


## Fork publication checks, 2026-11-06

The complete maintained fork was checked with Rust 1.95.0, Zig 0.16.0,
Node.js 24.19.0, Python 3.14.7 and Bun 1.4.4. Local macOS results:
304 Rust unit tests passed (one ignored), integration and native screenshot
checks passed, 47 bridge Node tests passed, 51 bridge Python tests passed,
21 Bun tests passed, and 14 distribution-script tests passed. Documentation
references/examples passed for all eight pages. The same bridge suites are now
required by CI in addition to the existing platform matrix; local results do
not establish Linux/Windows runtime acceptance or a completed remote CI run.

Exact commands (Zig must be available on PATH or through `ZIG`):

```sh
rustup run 1.95.0 cargo test --locked -- --test-threads=1
node --test bridge/*.test.mjs
PYTHONPATH=bridge python3 -m unittest discover -s bridge -p '*_test.py'
python3 -m unittest discover -s scripts -p test_distribution.py
npm exec --yes --package=bun@1.4.4 -- bun test scripts/files.test.ts scripts/picker.test.ts scripts/agents.test.ts scripts/spaces.test.ts scripts/docs.test.ts
npm exec --yes --package=bun@1.4.4 -- bun scripts/docs.ts check target/debug/herdr-app
rustup run 1.95.0 rustfmt --edition 2024 --check src/chat/*.rs src/chat/view/*.rs src/supervisor.rs src/reminders.rs src/legacy_agents.rs tests/bridge_assets.rs
bash -n scripts/install-fork-macos
git diff --check
```

Real native-window acceptance covered explicit decision submission, independent
long-list scrolling, preservation of the main composer draft, completion and
answer-history retention. Unverified historical ledger snapshots are not
bulk-imported into the inbox. `waiting_user` task updates create cards; ordinary
`awaiting_review` results are captured only after the manager identifies a
concrete current human decision or review. Existing transcripts and worker
processes are preserved. Installation instructions for this exact fork are in
the root README; upstream release downloads do not contain these additions.
