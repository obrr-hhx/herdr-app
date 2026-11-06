"""Restart only this conversation's idle execution service; keep its journal."""
import json
import os
import pathlib
import signal
import sys
import time

identity = sys.argv[1]
if not 16 <= len(identity) <= 64 or any(c not in '0123456789abcdef' for c in identity):
    raise SystemExit(2)
root = pathlib.Path.home() / '.local/share/herdr-chat/conversations' / identity
if not (root / 'daemon.pid').exists():
    raise SystemExit(0)
if not pathlib.Path('/proc').exists():
    raise SystemExit(4)
try:
    pid = int((root / 'daemon.pid').read_text())
    proc = pathlib.Path('/proc') / str(pid)
    args = (proc / 'cmdline').read_bytes().split(b'\0')
    if str(root).encode() not in args or not any(a.endswith(b'/daemon.mjs') for a in args) or proc.stat().st_uid != os.getuid():
        raise SystemExit(4)
except FileNotFoundError:
    raise SystemExit(0)
events = [json.loads(line) for line in (root / 'events.jsonl').read_text().split('\n') if line]
state = next((e.get('status') for e in reversed(events) if e['type'] == 'state'), 'starting')
pending = set()
for e in events:
    if e['type'] == 'permission':
        pending.add(e['id'])
    elif e['type'] == 'permission_resolved':
        pending.discard(e['id'])
safe_seq = next((e.get('seq', 0) for e in reversed(events) if e['type'] == 'state' and e.get('status') in ('idle', 'interrupted', 'error')), 0)
confirmed = {e['id'] for e in events if e['type'] == 'prompt_confirmed'}
queued = any(e['type'] == 'accepted' and e.get('action') == 'send' and e.get('seq', 0) > safe_seq and e.get('request_id') not in confirmed for e in events)
if pending or queued or state not in ('idle', 'starting', 'interrupted', 'error'):
    raise SystemExit(3)
os.kill(pid, signal.SIGTERM)
for _ in range(100):
    try:
        fields = (proc / 'stat').read_text().rsplit(')', 1)[1].split()
        if fields[0] == 'Z':
            break
    except FileNotFoundError:
        break
    time.sleep(.1)
else:
    raise SystemExit(4)
