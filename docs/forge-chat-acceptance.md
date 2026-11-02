# Native chat local acceptance

Validated on macOS Apple Silicon on 2026-11-02. This is the personal fork's
local installed build, not an upstream release or a GitHub CI result.

## Build and checks

Rust 1.95.0, Zig 0.16.0, and Bun 1.4.4 were used.

- `cargo build --release --locked -j 6`: passed.
- `cargo test --locked`: 262 unit tests passed, one existing ignored test;
  command, notices, and skill integration tests passed. Native GPU screenshot
  readback passed at two sizes. The macOS-only colrv1 target contains no tests.
- `bun test scripts/files.test.ts scripts/picker.test.ts scripts/agents.test.ts scripts/spaces.test.ts scripts/docs.test.ts`:
  19 passed.
- `bun scripts/docs.ts check`: passed.
- `node --test bridge/*.test.mjs`: four passed, covering out-of-order streaming,
  durable deduplication/replay, corruption detection, and error visibility.
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
Native chats are separate conversations; existing running terminal sessions
are preserved rather than attached concurrently to the same transcript.
