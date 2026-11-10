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
New native workers use `@local`, a host already used by a session, or an explicit
Host alias in the user SSH config (including Include files). A new VM does not
need a previous worker. Unknown-host refusals do not reserve a delivery receipt,
so the same request can be retried after configuring its alias. Terminal Codex workers
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

Local Codex send_message resumes the exact existing runtime thread with structured turn/start, validates live source/process identity and queues while busy, and returns thread/turn IDs. It preserves existing runtime settings and never types into the outer hc/Herdr attach terminal. Request receipts prevent duplicate dispatch; unknown delivery remains unknown and cannot be resent automatically. Pre-send refusals leave the request retryable with the same ID. Dispatch failure never authorizes creating a replacement worker; keep the original session and explain/repair the blocker, or obtain explicit replacement authorization.

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

`record_decision` / `list_decisions` 是管家的清单工具，通过 `supervisor.decision` / `supervisor.decisions` 操作 app 单一持久存储 `data/super-agent/decisions.json`。同一问题复用稳定 ID，重复观察只更新上下文、不抹掉回答与完成标记。`record_task` 只写台账，任何状态都不会自动挂卡；是否需要人类回答或验收，由管家读当前证据和已有清单后判断，再显式调用 record_decision。后台观察允许登记已观察到的决策，但不能代用户回答、完成事项、审批或派发任务。


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
bulk-imported into the inbox. Task state does not itself justify a card; the
manager explicitly judges whether a current human decision or review is needed. Existing transcripts and worker
processes are preserved. Installation instructions for this exact fork are in
the root README; upstream release downloads do not contain these additions.


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


Curation acceptance on 2026-11-06: an isolated native window started with the
original question and a legacy task-derived duplicate. Calling record_task
left the inbox unchanged; the real management adapter's curate_decision merged
them into one active card. The original record remained visible as merged in
已归档. A native option submission preserved the main composer draft and its
answer; curation of the answered card was refused. Closing and reopening the
isolated window retained the canonical answer and archived duplicate. No user
worker received a fixture prompt. The installed manager loaded the new tool
schema after its active turn finished; existing core terminals and the human's
previously answered/completed cards were retained.

Verification used Rust 1.95.0 and Zig 0.16.0: `cargo test --locked --
--test-threads=1` passed 308 unit tests (one ignored) and integration/native
screenshot checks. After adding the final stale-reason submission guard, all
12 native chat interaction tests passed again. `node --test bridge/*.test.mjs`
passed 47 tests; `PYTHONPATH=bridge python3 -m unittest discover -s bridge -p
'*_test.py'` passed 52 tests. The five Bun 1.4.4 suites passed 21 tests, and
generated documentation checks passed in eight pages. Scoped
`rustup run 1.95.0 rustfmt --edition 2024 --check src/chat/decisions.rs
src/chat/view.rs src/chat/view/decision_sidebar.rs` and `git diff --check`
passed. A fresh locked release build was installed and signature-verified.


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

Manual-review acceptance on 2026-11-06: an isolated native window sent one
inspection from the header, displayed the returned report, restored the button
and retained an unsent Chinese composer draft. The daemon integration test
verified queuing behind an ordinary turn, coalescing repeated requests, visible
output without a periodic publication decision, and preserving the original
human mandate. Codex tool tests verified that manual inspections refuse worker
writes. The installed app and idle manager service were updated; all eight
existing core terminal addresses were retained.

Validation: Rust 1.95.0 `cargo test --locked -- --test-threads=1` passed
309 unit tests (one ignored) and the integration/native screenshot checks;
`node --test bridge/*.test.mjs` passed 48 tests; `PYTHONPATH=bridge python3 -m
unittest discover -s bridge -p '*_test.py'` passed 52 tests. Bun 1.4.4 passed
all 21 tests in scripts/{files,picker,agents,spaces,docs}.test.ts. Generated
documentation checks passed in eight pages; scoped Rust formatting and
`git diff --check` passed. `cargo build --release --locked` produced the
installed, signature-verified build. Native report rendering used an isolated
fake model response; no fixture message was sent to a real worker.

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


Follow-up acceptance on 2026-11-06: the isolated native app accepted a second
assignment while the first was Working, with an unsent Chinese draft present.
The first result preceded confirmation/execution of the second assignment;
repeating the same ID yielded exactly one second user record. Both fake-model
results were visible and the draft remained unchanged. No real worker received
fixture messages. Codex adapter tests cover durable FIFO recovery, same-ID
receipts, identity reuse, live-turn steer parameters, pending decisions and
unknown-wire non-replay. Rust 1.95.0 full serial checks passed 310 unit tests
(one ignored) and integration checks; Python passed 60 tests, Node passed 48,
and Bun 1.4.4 passed 21. Scoped Rust formatting, generated documentation checks
and git diff whitespace checks passed.


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


Goal-management acceptance on 2026-11-06: 54 existing task snapshots were linked
to 12 canonical goals without changing any original ledger row; a SQLite backup
was retained outside the checkout. Actual human message IDs ground the enabled
mandates and scoped continuation plans. A subsequently created, independently
authorized goal was discovered normally. After installation a real read-only
review refreshed all 13 then-current goals with no tool errors; the manager
also called list_goals/record_goal_progress in its own background turn. No
production completion was inferred from this feature's tests or installation.

The daemon end-to-end fixture drove actual management tool processes through
a fake app control endpoint: verified evidence released a cited step, the
confirmation was reconciled, duplicate requests and a daemon restart produced
one worker assignment, arbitrary background send was blocked and attempted
mandate expansion was refused. Python tests cover pause, true human gates,
stale evidence/revision rejection, worker identity changes, manual inspection,
unknown delivery non-replay, same-step refusal backoff and projection recovery.
The isolated native app showed three goal cards, expanded/contracted all 12,
scrolled the remaining goals, expanded scope/exclusions, switched goals/inbox,
kept an unsent Chinese draft and refreshed a changed summary live. Native
interaction checks also covered a narrow window. All eight original terminal
addresses were retained during the installed app/idle manager update.

Validation: Rust 1.95.0 with Zig 0.16.0, `cargo test --locked --
--test-threads=1`, passed 311 unit tests (one ignored) and integration checks;
`PYTHONPATH=bridge python3 -m unittest discover -s bridge -p '*_test.py'` passed
73; `node --test --test-concurrency=1 bridge/*.test.mjs` passed all 51; Bun
1.4.4 passed all 21 scripts/{files,picker,agents,spaces,docs}.test.ts checks.
The parallel Node run hit the existing status formatter's 3-second subprocess
timeout; its isolated suite and then the complete serial suite passed. One
Rust run hit the existing mods lifecycle scheduler nondeterminism; the complete
serial rerun passed. Scoped `rustup run 1.95.0 rustfmt --edition 2024 --check
src/chat/view.rs src/chat/view/decision_sidebar.rs src/chat/transport.rs
tests/bridge_assets.rs`, generated documentation validation in eight pages
and `git diff --check` passed. The locked release build was installed and
signature-verified. UI and dispatch fixtures never sent prompts to real workers.

The real native queue contract fixture also exercised the actual CLI, ChatView,
Unix socket, durable outbox, daemon and model acknowledgement in an isolated
home. Explicit queue mode was adapted before the native command boundary; two
identical requests produced one accepted input and one confirmation. The fake
control endpoint now rejects unexpected native arguments. Three live requests
rejected by that exact pre-send argument validation were backed up and audited
before being classified as refused; unrelated unknown deliveries were retained.
After loading the updated local client and idle manager, a bounded background
observation queued those same three immutable planned steps on their original
workers. Each then reconciled to an actual native prompt confirmation. These
are delivery receipts, not claims that the worker tasks finished.


Runtime release (2026-11-07): `close_session` remains archive-only and returns
`runtime_stopped:false`. New `release_session` requires the latest explicit
human intent, works with live or previously archived exact identities, first
verifies removal from the active app/management list, then terminates the
owned native conversation process tree or the exact Core terminal. Native
ownership is established from executable/folder arguments, not `daemon.pid`.
PID start identity is checked before each signal; descendant groups are
tracked so session-owned tools that detach do not escape unnoticed. Graceful
shutdown precedes escalation. Shared SSH, Core and Codex servers are retained.
A native reconnect tombstone prevents automatic resurrection. Missing ownership
or an uncertain exit never becomes success; a prior unknown release is not
blindly replayed. History/work files remain. `list_sessions.runtime_releases`
and `closed_sessions[].runtime_release` expose durable current receipts while
the historical archive record stays unchanged. Only `released:true`,
`runtime_stopped:true` and `verified:true` establishes resource release.

Acceptance used isolated homes and disposable workers. The real native daemon
started its ordinary PTY runner, a fixture CLI and a detached tool process;
release verified all four processes exited while a neighboring daemon/CLI/tool
continued. A real isolated Core released one sleep terminal, retained its
neighbor and returned the same durable receipt on retry. Unit coverage includes
stale pidfile isolation, manager/invalid-address rejection, missing-owner
non-success, reconnect prevention, archive confirmation, archived-worker
release, human/background guards, identity changes and unknown non-replay.
No real user session was released as part of acceptance.

Validation: `PYTHONPATH=bridge python3 -m unittest discover -s bridge -p
'*_test.py'` passed 83 tests; `node --test --test-concurrency=1
bridge/*.test.mjs` passed 51. `cargo test --locked --test bridge_assets`
(Rust 1.95.0, Zig 0.16.0) checked that the built binary installs both release
modules and exposes the updated tool schema. Scoped rustfmt and
`git diff --check` passed.

The same native process-tree E2E also passed on Forge `env-rc411q92` via SSH
in disposable `/tmp` roots: four owned processes exited, zero remained, and
the neighboring CLI/tool stayed alive. Both fixture service and package roots
were removed. This did not stop any real Forge worker. The local app was
installed/signature-verified, reopened and its idle manager reloaded. Installed
release modules and reconnect guard matched source; original Core sessions
were retained.

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


Native steering acceptance (2026-11-07): remote Claude Code was verified as
2.1.319 on both Forge hosts. Steering uses ordinary interactive CLI sendNow
(Ctrl+X Ctrl+S), with active-turn validation and exact UserPromptSubmit-text
confirmation; it does not use print mode or another model conversation.
[Claude's documented sendNow behavior](https://code.claude.com/docs/en/interactive-mode#when-claude-code-sends-what-you-queued)
backgrounds movable work or interrupts non-movable generation. Slash/shell
commands keep normal queuing; pending questions and approvals are untouched.
Transport queuing, CLI confirmation, refusal and unknown delivery are distinct.
After a bridge restart, unconfirmed steering is marked unknown and not replayed.
Local native-journal reads expose bounded per-request receipts to the manager.

A disposable Forge Opus session ran a prepared Unix-socket fixture in Bash.
Before that tool finished, steering was confirmed in 0.20 seconds and the
model followed the new instruction in 5.24 seconds, in the same daemon and
conversation, with exactly one confirmation. A foreground sleep fixture was
refused by the environment's existing hook; no hook/security settings were
changed. The replacement fixture used an actual pending local IO operation.
All fixture processes and temporary roots were removed afterwards.

The native GUI fixture showed separate Stop, 排队 and 调整 controls while busy,
kept queued work pending, submitted a steer through the same ChatView send
path and displayed 已送达 Claude plus the changed reply. GPUI interaction
checks clicked both send modes and verified that an approval blocks steering
while preserving the draft. The old text-width and image-layout tests remain.

The client upgrades an old bridge through a guarded idle reconnect: original
conversation ID/history retained, pending input held in the durable outbox.
It requires this to be the sole client and verifies idle state twice, no
pending approval, confirmed historical input and no background process tree.
Unverified background agents defer upgrading. The legacy-upgrade fixture passed
on both macOS and Forge Linux without a model call; active/approval/busy
background cases refused signalling. No active user task is forcibly upgraded.

Validation: Python discovery passed 85 tests; serial Node passed 54; Bun 1.4.4
passed 21. Rust 1.95.0/Zig 0.16.0 `cargo test --locked -- --test-threads=1`
passed 315 unit tests (one ignored) and the integration checks. Canonical
commands: `PYTHONPATH=bridge python3 -m unittest discover -s bridge -p
'*_test.py'`; `node --test --test-concurrency=1 bridge/*.test.mjs`;
`bun test scripts/files.test.ts scripts/picker.test.ts scripts/agents.test.ts
scripts/spaces.test.ts scripts/docs.test.ts`. Generated command references
were refreshed for the optional native `mode` argument.

## Collaboration memory

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

The redesigned memory sidebar separates the effective **当前摘要** from
**详细记忆**. It displays compiled versus immediately effective fallback state,
refreshes the summary even when source records are unchanged, and preserves
editor drafts during refresh. Add/edit opens a dedicated form, with a bounded
title viewport and growing multiline body; source IDs stay out of the main UI.
Source quotes expand inside their own cards. The `chat.memory` navigation command
opens summary/records/new/edit views without mutating data or discarding drafts.

## Periodic goal tracking
Unfinished goals receive a mandatory published review every `goal_review_minutes`
(default 60). This backstop runs independently of adaptive wait decisions, human
silence and unchanged terminal status. The manager reads canonical goals, attempts
fresh source evidence, updates verified progress and advances only stored authorized
steps. Reports explain completed outcomes, blockers, actual advancement and next
steps. Missing evidence is explicit; skipped final review produces an incomplete
check notice. Busy turns defer, missed intervals coalesce, and reporting disabled
or paused goals stop the backstop. Earlier adaptive follow-ups remain available.

Goal worker follow-up: a bound original worker quiet for 15 minutes generates a
`goal_session_quiet` observation. `ask_goal_session` verifies fresh identity,
idle state/history and absence of human gates, then sends a fixed progress question
once per quiet episode. It never replaces a worker or expands authority. Busy,
blocked, paused, completed and disconnected cases are excluded; unknown delivery
is not replayed. Replies feed the existing goal-progress/continuation workflow.

目标卡片的“删除目标”需要再次确认，可取消。删除后停止该目标的后续跟进与派发，保留会话、历史、审计及已发送回执；已派发的工作不会被终止。旧后台更新不能恢复已删除的目标。
