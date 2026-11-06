# Interactive Claude backend probe

Status: integrated into this local fork on 2026-11-02; local acceptance completed below.

The native Forge conversation backend should use a normal interactive Claude
process, with a remote PTY owner and hooks that journal structured events. The
local app renders these events; it does not synchronize the terminal screen.
The Agent SDK dependency has been removed.

## Measured behavior

An isolated scratch directory on the existing Forge test machine was used.
Claude Code 2.1.312 ran without `-p`, with model `opus`, effort `low`, normal
permission mode, and the existing remote OAuth-token authentication. The
transcript confirmed `claude-opus-5-5`. User/project settings and MCP servers
were excluded for isolation. No existing user session was interrupted.

Four small model turns verified:

- A 35-line answer produced 35 `MessageDisplay` batches, with the first batch
  about 1.99 seconds after submission and subsequent batches over 5.92 seconds.
- A `PermissionRequest` command hook delayed its answer by 1.5 seconds, then
  allowed only the exact scratch-file Write requested by the test. The file
  was created and `PostToolUse` was received. This was a scripted test approval,
  not a human approval in the app.
- The next turn recalled the marker from the first turn.
- A new CLI process resumed the same conversation ID and recalled the marker.
  A multiline bracketed paste was received as one user submission.

Reassembling display batches by `message_id` and `index` matched the persisted
assistant text. Indices were contiguous, but arrival order was not: index 31
arrived before index 30. `Stop` arrived before the last display batch.

Local measurement files are under
`/private/tmp/claude-interactive-probe-20261102/`; the scratch CLI processes
exited after their tests. Remote credentials were not copied.

## Required bridge behavior

- Keep one interactive process alive remotely independently of SSH subscribers.
  Read and discard its PTY output so it cannot block; forward structured events.
- Journal hook events durably before acknowledging them. Keep hooks fast;
  remote journaling must not wait for a network round trip to the local GUI.
- Group display batches by conversation, message, and index; reorder and
  deduplicate them. A final batch can be empty. Do not mark all display work
  complete solely on `Stop`.
- Reconcile completed messages with the transcript for recovery. Display hook
  IDs differ from API message IDs, so do not assume equality when reconciling.
- Use a permission request/reply channel scoped to the current request, with
  explicit expiry/cancellation and no automatic approval on disconnect.
- Treat PTY submission separately from the application's durable outbox ACK.
  Sending keystrokes alone does not prove Claude accepted a prompt.

## Boundaries still requiring acceptance

This proves line-level incremental output, not token-level streaming. Long
single-line text can wait for a completed line. Thinking deltas are not currently exposed. Follow-up acceptance verified real
Opus tool approvals and question answers, subscriber disconnect/replay with
contiguous sequence numbers, duplicate-send suppression, service restart/resume,
and interruption followed by a new turn (2.56 seconds in the isolated probe).
Native app acceptance covered Chinese input, Enter submission, 45-line history
scrolling, preserved reading position across chat switches, local history after
window restart, and a tool approval clicked in the app with its file verified
on the VM. This is a local build, not a published release.

The probe did not measure five-hour quota weighting. Using a normal interactive
process avoids choosing the headless execution path, but does not establish a
specific allowance multiplier or savings. No billing labels were spoofed.

Official interfaces:

- [CLI reference](https://code.claude.com/docs/en/cli-reference)
- [MessageDisplay](https://code.claude.com/docs/en/hooks#messagedisplay)
- [PermissionRequest](https://code.claude.com/docs/en/hooks#permissionrequest)
