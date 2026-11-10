# Native chat local acceptance

Validated on macOS Apple Silicon on 2026-11-02. This is the personal fork's
local installed build, not an upstream release or a GitHub CI result.

## Build and checks

Rust 1.95.0, Zig 0.16.0, and Bun 1.4.4 were used.

- `cargo build --release --locked -j 6`: passed.
- `cargo test --locked`: 268 unit tests passed, one existing ignored test;
  command, notices, and skill integration tests passed. Native GPU screenshot
  readback passed at two sizes. The macOS-only colrv1 target contains no tests.
- `bun test scripts/files.test.ts scripts/picker.test.ts scripts/agents.test.ts scripts/spaces.test.ts scripts/docs.test.ts`:
  19 passed.
- `bun scripts/docs.ts check`: passed.
- `node --test bridge/*.test.mjs`: seven passed, covering out-of-order streaming,
  durable deduplication/replay, corruption detection, error visibility, transcript import, and refusal of a live conversation owner, and visible reporting of malformed
  source records without changing the source.
- `rustfmt --edition 2024 --check src/chat/*.rs`, `node --check` for bridge
  modules, `python3 -m py_compile bridge/hook.py bridge/runner.py`,
  `bash -n scripts/install-fork-macos`, and `git diff --check`: passed.
- Notices were regenerated with `bun scripts/notices.ts` and cargo-about 0.9.3.
  Only the Ghostty build-script fingerprint changed; dependencies did not change.

One full-suite run hit an existing lifecycle test's nondeterministic
cross-thread scheduler assertion. The lifecycle test source was unchanged;
the subsequent complete suite passed. Three compiler warnings remain in the
unchanged vendored quickjs-jit-runtime sources.

## Real Opus acceptance

Claude Code 2.1.312, model `claude-opus-5-5`, used the existing remote OAuth-token
login. Test prompts and generated files were confined to scratch directories.
The backend used a normal interactive CLI process, without `-p` or the Agent SDK.

- Incremental line events were received before response completion.
- Disconnecting the subscriber during a reply and reconnecting replayed the
  missing events contiguously. Resending the same request did not execute twice.
- A Write waited for approval; no file existed before approval. The GUI's Allow
  button was also clicked and the expected file content verified on the VM.
- AskUserQuestion answers reached Claude through the hook response.
- Restarting the isolated execution service resumed the original conversation
  and remembered the initial identifier.
- Interrupting an active foreground tool allowed the next turn to complete;
  measured recovery to the next answer was 2.56 seconds on the VM.

## Native window acceptance

- Chinese input and Shift+Enter remained local; Enter sent a prompt.
- A real 45-line reply scrolled both ways with the mouse.
- Switching to another chat and back preserved the reading position.
- Closing/reopening the window restored local history.
- With the composer focused, Ctrl+Home reached the first message. Page Down
  moved from item 0/offset 0 to item 1/offset 407; Page Up returned to the top.
- The installed window was resized from 2560x1642 to approximately 1604x1044
  physical pixels. Connection controls and composer remained visible. Approval
  height is bounded relative to the window height.

The five-hour subscription quota multiplier was not measured. Text streams by
completed lines, not individual tokens. Thinking deltas are not displayed.
New native chats are separate conversations. Imported conversations retain
the original ID after a clean handover; concurrent writers are refused.

## Existing-session handover

- Imported an existing interactive conversation and received SessionStart with
  the same conversation ID and resume source, without submitting a test prompt.
- Completed transcripts are normalized into local message and tool history;
  diagnostic request copies and sidechains are excluded. The original remote
  transcript remains intact. Imports survive retry without duplicating history.
- A live original CLI prevents import/resume. Handover exits only an inspected,
  idle CLI; it preserves its tmux terminal and Forge session record.
- Native tabs can join an existing space with `chat.open --space`, grouping
  migrated conversations beside their original machine's shell.
- Existing Codex processes and plain shells use live terminal attachment.
  They are not restarted or converted to the Claude conversation backend.

Personal migration acceptance on the same installed build covered six existing
Claude conversations, four local Codex terminals, and two remote shells. All six
Claude sessions reported idle with the original ID, one live owner each, and no
UserPromptSubmit during migration. Two unsent drafts survived app restart. An
import with one malformed source record completed with an explicit history
notice; its source file was retained. Imported history scrolled from item 0 to
item 17 and kept that position across a space switch.

## Agent list status and navigation

The native sidebar now publishes both interactive Claude chats and local legacy
Herdr attach clients. Codex status is read from the original Herdr agent API,
with one bounded background query per source session per second; terminal
contents are not used to infer status. Claude uses its structured conversation
events, including pending permission requests. Native updates are published
on the existing 400 ms app tick.

An isolated macOS app home and core verified both entries in Working and Needs
you, completion attention, row clicks to both destination panes, and next-agent
navigation across terminal and chat panes. Claude-only state changes, without
a terminal change or tab switch, reached the sidebar in about 0.5 seconds in
the fixture. An unavailable legacy source did not remain idle, recovered after
the source returned, and its entry disappeared when the test terminal exited.
These transition fixtures did not send model prompts.

The installed personal app was then checked against the existing sessions:
seven Claude rows (six migrated plus one prior native chat) and four Codex rows.
Codex foreground PIDs were identical before and after installing/reopening the
app. Native row states matched chat status; local states came from the original
Herdr API. The two plain shells were excluded from the agent count.

## Native composer and layout

The conversation uses a centered reading column, right-aligned user messages,
collapsed tool rows, and a bottom composer. Connection configuration is collapsed.
Command and file menus support keyboard navigation and clicks; menu height is
bounded by the window. Model and effort controls share the composer toolbar.

Real interactive Opus acceptance in an isolated remote workspace verified quoted
Chinese/spaced file references, project command execution, and an effort change
that resumed the original conversation ID. A real `/compact` completed with both
PreCompact and SessionStart (source compact). No SDK or `-p` process was used.
Native window acceptance verified `/` filtering, Enter to execute a discovered
command, Tab replacing an `@` prefix without sending, draft preservation while
using the model selector, and New chat creating a tab in the same space while
keeping the old history and draft. A false failure from old terminal history
was found during acceptance; CLI prompt confirmation now cancels that diagnostic
so a successful command is not reported as rejected.

The real mono-meta file index was bounded to a sub-megabyte response; a subsequent
query found example-project files beyond that initial cache. Linked project skills were
verified separately. Registry tests also cover enabled versus disabled plugins.

The composer was also checked in a 1606 x 1044 physical-pixel window. Full
command menus scroll to keyboard selection and leave the input, send control,
and shortcut hint visible. Busy-runtime refusal and acknowledgement are covered
with a mock interactive process; they do not consume model requests.

## Long draft scrolling

A 25-line Chinese/English draft reproduced the two-line clip: the textarea's
layout viewport was taller than its visible frame. The draft frame now uses a
flex column with intrinsic auto-grow height, capped by the textarea's six-row
mode rather than a second conflicting outer height constraint. Page Up/Down
and document navigation no longer get captured by history while editing.

Native macOS acceptance verified wheel scrolling between lines 1 and 25,
Cmd+Up/Down, Page Up within the draft without moving history, a long soft-wrapped
paragraph, Shift+Enter, and shrinking back to a short draft with the slash menu.
Both 1606x1044 and 1364x866 physical-pixel windows retained the input toolbar and
send button. The installed app was reopened; draft checksums and the live agent
count matched the pre-restart snapshot. No test prompts were sent to user chats.

## Remote tool permissions and activity

New and resumed interactive Claude processes now include
`--dangerously-skip-permissions`, matching the owner's requested full tool access
for these Forge chats. Actual user questions remain interactive. A real isolated
Opus Bash command returned its expected marker without any PermissionRequest;
runner fixtures verify both new and resume paths use this flag and never `-p`.

A live conversation had no new tool result for over ten minutes after approval,
and no command subprocess was running. Escape did not complete its interruption.
Only that conversation's owned execution service was stopped and resumed with
its original Claude UUID; a continuation asked Claude to inspect prior results
before repeating work. Subsequent Bash results succeeded without approvals.
The other services were updated only when idle, including one deferred busy chat.

An ephemeral activity heartbeat reports the last real progress time and stage;
heartbeats do not count as progress or create conversation messages. The header
identifies tool execution, waiting for Claude, response display, and user input.
After 90 seconds without progress it reports the elapsed time rather than claiming
that a connected process is productive. Rust tests cover this distinction.

## Reading position while updates arrive

The previous update handler treated an unknown bottom position as permission to
jump to the end, and invalidated every message even on a status heartbeat. Native
chat now uses ListState's tail-follow mode, which suspends on user scrolling.
Only changed existing message rows are remeasured; appends add new rows, and
metadata updates leave message geometry intact. Latest and sending explicitly
resume following.

A GPUI regression uses 100 history rows, unknown offscreen heights, user scrolling,
ten rounds of streaming growth and appends, and metadata heartbeats. The reading
anchor and paused follow state remain unchanged; Latest resumes following.
A separate state test verifies that activity/settings events dirty no message rows.

## Hooks after working-directory changes

The session's identity is its UUID on a private per-conversation socket. Hook cwd
is required to be an absolute path, but may change after an interactive Bash cd;
it is no longer compared with the launch directory. The control regression sends
PreToolUse and Stop from a subdirectory and rejects a different session UUID.

A real affected session had completed its turn in Claude's transcript while its
bridge was still Working. Its transcript cwd had changed to code/backend/example-project.
After verifying the final turn-duration record and absence of command subprocesses,
only its owned idle execution service was stopped. Sanitized missing transcript
records were appended with existing tool IDs deduplicated; 23 records, including
the final response, were restored under the same UUID. No prompt was resubmitted.

## Attention without false requests

Native Claude's execution state is kept separately from transport health. Only
live permission/question records produce Needs you; connection failures, stopped
sessions and execution errors do not. Connected snapshots reconcile the local
pending IDs with the server's authoritative list. Reconnection and errors have
neutral badges, including a compact reconnect glyph.

Legacy Codex source-query failures no longer generate a blocked/error agent.
They retain the previous confirmed source status while that source is still
listed, and do not invent either a request or a completed turn. A successful
source report of blocked still passes through normally.

State regressions cover transport loss during work, stale versus live requests,
execution faults, and source-query failures. Sidebar tests cover neutral badges.
After installation, all eight native rows with zero pending requests were checked
to be outside Needs you; drafts and the live agent count were preserved.

## Durable unread completions

Attention changes now write an independent atomic metadata snapshot rather than
relying on a later layout save. Startup does not prune native attention before
chat views have loaded. Done has no time expiry. Restoring attention preserves
unread state and ordering; a later idle refresh cannot acknowledge it. Native
completion sequence numbers detect turns missed while the app was closed.

A focused Claude chat counts as read only while following its latest result;
reading earlier history leaves a new completion unread. On the first upgrade,
completed chats not being read and idle attached agents are conservatively
restored to Done once, because older lost read markers cannot be reconstructed.

Regressions cover serialization/restart, arbitrarily old Done states, idle/done
refreshes, explicit read acknowledgement, and a completion that occurs entirely
while the app is closed. Conversation tests ensure reconnects and heartbeats do
not advance the completion stamp.

Native window acceptance used isolated structured history/completion fixtures,
without model prompts. Both unread rows survived quitting/reopening the QA app;
a current chat reading old history retained Done, and Latest in the active window
acknowledged it. In the installed user app, eight completed rows were restored to
Done; all eight remained Done after a second app restart. The 12 agent sessions
and all draft checksums were preserved.

## Native Claude usage status line

The composer footer now shows the native model, Git branch, context percentage,
five-hour and seven-day usage with reset countdowns, and estimated session cost.
Details expands token/cache counts, session duration, changed lines and sample
age. Missing native values remain unknown rather than becoming zero. Session
cost is a list-price estimate, not an additional Pro subscription charge.

A private collector accepts only allowlisted status-line metadata for the owning
session. Existing interactive CLI processes hot-load a dispatcher through their
user status-line configuration; it preserves the original terminal formatter
and unrelated settings. New conversations also receive their own collector
configuration. Collection makes no model or usage API requests. Idle sessions
can omit quota fields until Claude itself supplies another native sample.

Acceptance used one dedicated ordinary interactive Opus turn, including a Bash
tool result, and observed real context, quota/reset and cost fields in the GUI.
The 1364-by-866 native QA window retained all footer controls with a 20-line
draft; opening Details reduced visible editor rows and internal scrolling reached
the last draft line. Tests cover split transport frames, missing versus zero
usage, session ownership, metadata filtering, original formatter passthrough and
idempotent installation. Rust tests passed (278 passed, one existing ignored),
as did all 18 Node bridge tests and chat Rust formatting checks.

Both existing remote hosts received the collector without restarting their
Claude or daemon processes. After installing and reopening the final local app,
remote process IDs, draft checksums and previously unread Done markers remained
preserved. Existing non-empty conversations supplied real native telemetry;
quota fields unavailable from individual idle CLIs remained unknown.

## Clipboard images and remote vision (2026-11-04)

Screenshot paste and copied image files now create private, content-addressed
PNG attachments in the native draft. Thumbnails open a full preview and have
individual remove controls. Preview owns keyboard focus, Escape returns to the
editor, and its controls do not click through to the conversation underneath.
Four thumbnails fit a narrow composer; long drafts keep internal scrolling.

Images upload over SSH before their command is forwarded. Local outbox
records contain image identifiers, not binary payloads or local file paths.
Upload failure preserves the same queued request across app restart. The remote
receiver checks size, dimensions, content hash, the destination
path and symlinks, and writes images with private permissions. Native interactive
Claude consumes the remote image references, including image-only messages.

The SSH client adapts older running services without restarting Claude. Private
request metadata restores clean GUI text and attachments from legacy history;
retry keeps the same wire representation even if the service is upgraded.

Acceptance used an isolated native macOS window at 1364×866 physical pixels.
System clipboard image paste, preview, Escape/Close, removal, four-image limit,
20-line draft scrolling, draft restoration and narrow-window controls were
checked directly. Preview hit testing and narrow attachment sizing defects
found during acceptance were repaired and retested.

Real interactive Opus correctly identified three colored squares, a yellow
circle and a purple triangle from the attached image. A second, image-only send
was deliberately blocked at upload, survived app restart with the same request,
then recovered with exactly one server delivery and a completed answer. A copy
of the previous running service also accepted a complete Chinese message plus
the image and returned the correct visual description with its original Claude
PID unchanged. No SDK or print-mode model process was used.

Verification includes 279 passing Rust tests (one existing ignored), integration
and real GPU pixel tests, 22 passing Node bridge tests, and 16 Bun tests. Chat
Rust formatting, Node syntax, Python compilation and diff whitespace checks
passed. Third-party notices were regenerated for the JPEG feature declaration.

The installed user app retained its seven native conversation panes, current
draft checksums and unread native Done markers. Helpers initially installed on
both active Claude hosts without restarting their user CLI processes. During
the final refresh, `claude-hello-xl-1027` became unreachable over its configured
SSH route, its VM Tailscale peer reported offline, and Forge exec timed out.
The last helper refresh and live process observation on that host remain
unverified. The other host's CLI PIDs were preserved. Test-only services and the
isolated core were stopped after acceptance.

### 2026-11-04: hello XL connectivity recovery and final helper sync

The network gap above is now closed. Forge reported the environment running
while the guest was unreachable from both the Mac and another healthy Forge VM.
The real VM Tailscale peer was offline; SSH, Forge exec and the SSH gateway could
not reach the guest. Forge logs recorded agent loss at 11:18 China time. A nearby
Proxmox allocation failed because the host had only 5 GB available memory; this
is supporting evidence of host pressure, not proof that this guest was OOM-killed.
The guest's previous journal stops abruptly at 11:17 without a clean shutdown.
Previous-boot kernel records include rust-lld OOM kills on October 28; those
older events do not establish the cause of the November 4 outage.

A disk-preserving Forge suspend/resume restored SSH at the same private and
Tailscale addresses. The environment returned to running, and fileagent and
tailscaled were active. All 15 installed bridge assets matched local SHA-256
hashes. All five original conversation/session ID pairs connected successfully
with image capability and no pending approval. Four resumed automatically; the
remaining lazy service was restarted with its original Claude session ID after
guarded idle/queue checks. No old prompt was resubmitted. This VM recovery
restarted the guest: it does not preserve pre-reboot processes or prove that an
in-flight task completed.

All 15 recorded local draft/outbox files retained their checksums. A separate
test-only conversation successfully received a PNG with mode 0600 and passed
the native Node image-path validation; its files were removed afterwards.
The previously completed real interactive vision acceptance applies to the same
helper versions. No additional model request was sent to user conversations.

Native chat groups consecutive tool calls/results into a compact expandable activity card. Open the group, then a tool row to inspect/copy its full input and output; failures remain visible on the collapsed group. Active tools show an existing description/command/path, a reduced-motion-aware pulse and elapsed wall time. Completed timings freeze; imported records without reliable timings remain unknown. No model call generates these labels. Questions/approvals use inline confirmation cards with selectable choices, optional custom answers, a full-request disclosure and an explicit one-time submit. Minimizing a card does not answer/dismiss it; drafts and Needs you remain. Duplicate submits are blocked until acknowledgment, and selecting an option never submits automatically. Tool details and long questions scroll within bounded panels; grouping changes presentation only, not saved transcript/tool IDs or history-read APIs. chat.status also reports history_rows/tool_groups/open_tool_groups/sending_decisions.

The tool-card refresh was verified in isolated native macOS windows in dark and
light themes. Native screenshots show collapsed execution/error summaries,
inline choice cards, and a running tool description/timer. Two captured running
frames differed at the pulse dots, proving the indicator animates. GPUI pointer
acceptance tests expand/collapse groups and tool results, bound long output,
choose/submit an answer once, retain a pending card until acknowledgment, and
preserve both composer drafts and hidden alternative-answer drafts. Scroll tests
cover long questions and a 340x320 chat pane inside an 1100x900 outer window;
confirmation bounds use the pane's measured height. Group growth preserves a
reader's scroll anchor and does not create extra blank history rows. Test homes
use a thread-local override rather than changing the process environment.

### Native Claude interaction comparison (2026-11-06)

Read-only computer use in the installed Claude desktop app covered history
scrolling, opening a command group, opening/collapsing an individual command,
and the contextual confirmation presentation. No task was approved or sent.
The resulting Herdr refinement uses a borderless collapsed activity line,
one-click details for a single call, wrapping choice buttons with the selected
explanation, and a smaller empty composer that still grows and scrolls.

Computer use in a separate Herdr UI QA app/home/core exercised the actual
native renderer: group/detail expansion, changing a choice, resizing to an
808 × 548 window, and pasting 24 Chinese draft lines. Scrolling inside the
composer reached both lines 1–2 and lines 23–24 without submitting the draft.
The fixture used a fake model, not user workers. GPUI interaction tests also
cover single-call disclosure, explicit/duplicate confirmation, long-question
scrolling, a 340 × 320 split pane, and history anchors during new tool events.

Validation: `rustfmt --check --edition 2024 src/chat/view.rs
src/chat/presentation.rs`, `git diff --check`, the five Bun script suites
(21 tests), CLI documentation checks (8 pages), and
`cargo test --locked -- --test-threads=1` passed (295 unit tests, 1 ignored,
plus integration suites). The initial parallel run hit the pre-existing file
watcher timeout; the full serial run passed. Local Rust was 1.97.1 and Bun
1.4.2; this is macOS local evidence, not the pinned multi-platform CI result.
The signed personal app was updated and reopened through computer use; its
existing manager backend PID was unchanged and the session list remained.

### Fresh Claude conversation dogfooding (2026-11-06)

Created a new Claude desktop Code conversation, `Herdr UI Playground`, scoped
to `/private/tmp/herdr-claude-playground`. Real interaction covered reading two
fixture files, running Node, answering an AskUserQuestion, editing the fixture,
adding/running standalone tests, file mention search and Tab selection, slash
and model menus, long structured replies, and scrolling old history while a
turn ran. A harmless 20-second print command completed. Cmd+Enter and Stop
were exercised, but the replies completed too quickly to establish queued or
interrupted states; neither is claimed as a successful acceptance scenario.
No real repository or external service was operated on by this test chat.

The resulting UI changes classify observed tool operations, show explicit
TodoWrite/update_plan steps, render completion menus as bounded overlays with
current-setting marks, and show a contextual jump-to-latest button while
reading history. Task statuses are worker-reported, not independent task
verification; unsupported task schemas are left as ordinary tool details.

A separate light-theme native Herdr fixture verified the operation summary,
explicit steps, model/slash overlays, window resizing, scrolling away from the
tail and clicking back to it. GPUI interaction tests verify all four menus
preserve the history viewport, exact reading anchor and draft; explicit task
steps are visible without opening tool JSON. Existing confirmation and narrow
split-pane coverage remains in the suite.

Final local verification for the dogfooding refinement: 298 unit tests passed
(1 ignored), all integration suites passed, and 21 Bun tests passed. Exact
commands were `cargo test --locked -- --test-threads=1`, `bun test
scripts/files.test.ts scripts/picker.test.ts scripts/agents.test.ts
scripts/spaces.test.ts scripts/docs.test.ts`, `bun scripts/docs.ts check
target/release/herdr-app`, the scoped rustfmt check
above, and `git diff --check`. The signed local app was reopened; its prior
manager backend PID was preserved and the existing 18-agent list reappeared.
The disposable Herdr fixture's daemon/core were stopped after acceptance.


## Bounded image-message layout (2026-11-06)

Sent images now reserve a 160×144 logical-pixel preview and contain the image
independently of intrinsic dimensions. The bubble stacks images above text;
up to four previews wrap in narrow windows. Image-only messages omit the
empty text block. Draft thumbnails remain compact and removable.

`cargo test --locked chat::view::interaction_tests -- --test-threads=1`
passed all 16 interaction tests with Rust 1.95.0 and Zig 0.16.0. The new case
loads portrait (40×1600), landscape (1600×40), square and another portrait
image; at 1200/540 window widths it asserts bounded thumbnails, text below
images, no bubble overflow, image-only layout and preview activation.
A separate native app home rendered four real PNG fixtures with wrapping
and a visibly separate caption. No prompts went to real workers. Scoped
rustfmt, generated documentation checks and `git diff --check` passed.
The signed local app was updated and reopened with worker sessions retained.


## Text-only bubble regression (2026-11-06)

The image-layout change incorrectly put text-only Markdown in an auto-width
vertical flex container. The reported Chinese message reproduced as a 32px
wide, 609px tall bubble. Text-only bubbles now use intrinsic block measurement;
attachment bubbles keep an explicit bounded width and vertical image/text flow.
The regression test covers the original sentence at 1200/720 widths with the
decision sidebar open and 540 without it, plus a short reply and long English
text. The portrait/landscape multi-image test remains in the same suite.
`cargo test --locked chat::view::interaction_tests -- --test-threads=1`
passed all 17 tests (Rust 1.95.0, Zig 0.16.0).
The isolated native window showed the exact reported Chinese sentence in a
normal horizontal bubble, a compact single-character reply and unchanged image
layout with the sidebar open. After updating the signed installed client, the
real existing message was also visibly horizontal; all eight core sessions
were retained. Generated docs checks and scoped rustfmt passed.
