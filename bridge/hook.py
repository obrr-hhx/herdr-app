"""Fast local hook RPC; no model requests, credentials, or terminal parsing."""
import json
import socket
import sys

folder = sys.argv[1]
event = json.load(sys.stdin)
s = socket.socket(socket.AF_UNIX)
s.settimeout(605 if event['hook_event_name'] in ('PermissionRequest', 'PreToolUse') else 5)
try:
    s.connect(folder + '/run.sock')
    s.sendall((json.dumps({'action': 'hook', 'event': event}) + '\n').encode())
    with s.makefile('r') as f:
        line = f.readline(1048577)
        if not line or len(line) > 1048576:
            raise RuntimeError('bridge unavailable')
        reply = json.loads(line)
        if 'output' in reply:
            print(json.dumps(reply['output']))
except Exception:
    # Fail closed when a decision channel is lost. Display-only hooks fall back
    # to Claude's normal display and the saved transcript remains recoverable.
    name = event['hook_event_name']
    if name == 'PermissionRequest':
        print(json.dumps({'hookSpecificOutput': {'hookEventName': name, 'decision': {'behavior': 'deny', 'message': 'Local approval connection unavailable'}}}))
    elif name == 'PreToolUse' and event.get('tool_name') == 'AskUserQuestion':
        print(json.dumps({'hookSpecificOutput': {'hookEventName': name, 'permissionDecision': 'deny', 'permissionDecisionReason': 'Local question connection unavailable'}}))
finally:
    s.close()
