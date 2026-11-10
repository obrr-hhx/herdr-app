"""Own an ordinary interactive Claude PTY. Only control metadata leaves this process."""
import fcntl
import json
import os
import pathlib
import pty
import select
import signal
import struct
import sys
import termios
import time
import subprocess
import re

def shutdown(signum, frame):
    raise SystemExit(0)

signal.signal(signal.SIGTERM, shutdown)
signal.signal(signal.SIGINT, shutdown)
folder, cwd, model, session = sys.argv[1:5]
effort = sys.argv[5] if len(sys.argv) > 5 else ""
root = pathlib.Path(folder)
settings = root / 'settings.json'
args = [os.environ.get('HERDR_CHAT_CLAUDE', 'claude'), '--model', model,
        '--settings', str(settings), '--ax-screen-reader']
if os.environ.get('HERDR_CHAT_LOCAL') != '1' or os.environ.get('HERDR_SUPER_AGENT') == '1':
    args += ['--dangerously-skip-permissions']
if os.environ.get('HERDR_SUPER_AGENT') == '1':
    bridge = pathlib.Path(__file__).resolve().parent
    mcp = {'mcpServers': {'herdr': {'command': 'python3', 'args': [str(bridge/'supervisor.py')]}}}
    args += ['--append-system-prompt', (bridge/'supervisor-prompt.md').read_text(), '--tools', '', '--strict-mcp-config', '--mcp-config', json.dumps(mcp)]
if effort:
    args += ['--effort', effort]
args += ['--resume', session] if (root / 'has-session').exists() else ['--session-id', session]
if os.environ.get('HERDR_CHAT_TEST_ISOLATED') == '1' and os.environ.get('HERDR_SUPER_AGENT') != '1':
    args += ['--strict-mcp-config', '--mcp-config', '{"mcpServers":{}}']
    if not effort:
        args += ['--effort', 'low']
# Only this local CLI receives the selected subscription credential. Never put it
# in argv, settings files, journals or tool output.
if os.environ.get('HERDR_CHAT_LOCAL') == '1':
    from local_auth import subscription_environment
    try:
        subscription_env, subscription = subscription_environment(os.environ, os.environ.get('HERDR_LOCAL_CLAUDE_AUTH_HOST', ''))
    except Exception:
        print(json.dumps({'type':'authentication_failed'}), flush=True)
        raise SystemExit(2)
    if subscription:
        os.environ.update(subscription_env)
        override = json.loads(settings.read_text())
        override['env'] = {**override.get('env', {}), **{k: subscription_env[k] for k in ['ANTHROPIC_AUTH_TOKEN','ANTHROPIC_API_KEY','ANTHROPIC_BASE_URL','CLAUDE_CODE_USE_BEDROCK','CLAUDE_CODE_USE_VERTEX','CLAUDE_CODE_USE_FOUNDRY']}}
        args[args.index('--settings')+1] = json.dumps(override)
# Probe the CLI without creating another model conversation or exposing credentials.
try:
    version = subprocess.run([args[0], '--version'], capture_output=True, text=True, timeout=3)
    match = re.search(r'(\d+)\.(\d+)\.(\d+)', version.stdout)
    supports_steer = bool(version.returncode == 0 and match and tuple(map(int, match.groups())) >= (2, 1, 275))
except (OSError, subprocess.TimeoutExpired):
    supports_steer = False
print(json.dumps({'type': 'capabilities', 'steer': supports_steer}), flush=True)
pid, fd = pty.fork()
if pid == 0:
    os.chdir(cwd)
    os.environ['TERM'] = 'xterm-256color'
    os.execvp(args[0], args)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', 45, 160, 0, 0))
os.set_blocking(fd, False)
buffer = b''
commands = b''
trusted = False
bypass_accepted = False
setup_pending = True
compacting = False
slash_pending = False
slash_error_seen = False
command_started = 0.0

def report(kind, **fields):
    print(json.dumps({'type': kind, **fields}), flush=True)

try:
    while True:
        ready, _, _ = select.select([fd, sys.stdin.buffer], [], [], .1)
        if fd in ready:
            try:
                data = os.read(fd, 65536)
            except (OSError, BlockingIOError):
                break
            if not data:
                break
            # Drain terminal output locally; it is never sent to the app or logged.
            buffer = (buffer + data)[-12000:]
            if compacting and any(x in buffer.lower() for x in [b'not enough messages to compact', b'compaction failed', b'error during compaction']):
                slash_error_seen = True
            if slash_pending and any(x in buffer.lower() for x in [b'unknown skill', b'unknown slash command', b'unknown command', b'skill not found']):
                slash_error_seen = True
            # Connecting a chat grants access to its selected working directory.
            if setup_pending and not trusted and b'Enter y/n:' in buffer and b'trust this folder' in buffer:
                os.write(fd, b'y\r')
                trusted = True
                buffer = b''
            if setup_pending and not bypass_accepted and b'Enter y/n:' in buffer and b'Yes, I accept' in buffer and b'No, exit' in buffer:
                os.write(fd, b'y\r')
                bypass_accepted = True
                buffer = b''
        if slash_error_seen and (slash_pending or compacting) and time.monotonic() - command_started > 8:
            report('command_failed', reason='slash_unavailable' if slash_pending else 'compact_unavailable')
            slash_pending = False
            compacting = False
            slash_error_seen = False
            buffer = b''
        if sys.stdin.buffer in ready:
            chunk = os.read(sys.stdin.fileno(), 65536)
            if not chunk:
                break
            commands += chunk
            while b'\n' in commands:
                line, commands = commands.split(b'\n', 1)
                c = json.loads(line)
                if c['action'] == 'prompt_confirmed':
                    slash_pending = False
                    compacting = False
                    slash_error_seen = False
                elif c['action'] == 'interrupt':
                    os.write(fd, b'\x1b')
                elif c['action'] == 'send':
                    setup_pending = False
                    text = c['text']
                    paths = c.get('image_paths', [])
                    if paths:
                        text += '\n\nAttached images (use Read to view these image files):\n' + '\n'.join(paths)
                    compacting = text.strip().startswith('/compact')
                    slash_pending = text.strip().startswith('/') and not compacting
                    slash_error_seen = False
                    command_started = time.monotonic()
                    buffer = b''
                    # Never let user text escape bracketed paste or act as terminal controls.
                    if any(ord(ch) < 32 and ch not in '\n\t' for ch in text):
                        report('rejected', request_id=c['request_id'])
                        continue
                    raw = ('\x1b[200~' + text + '\x1b[201~').encode()
                    # Handle partial/nonblocking writes without dropping long pastes.
                    while raw:
                        readable, writable, _ = select.select([fd], [fd], [], 1)
                        if readable:
                            os.read(fd, 65536)
                        if writable:
                            raw = raw[os.write(fd, raw):]
                    time.sleep(.1)
                    os.write(fd, b'\x18\x13' if c.get('mode') == 'steer' else b'\r')
                    report('submitted', request_id=c['request_id'])
finally:
    try:
        os.kill(pid, signal.SIGTERM)
        for _ in range(50):
            if os.waitpid(pid, os.WNOHANG)[0]:
                break
            time.sleep(.1)
        else:
            os.kill(pid, signal.SIGKILL)
            # macOS can leave an exiting CLI unreapable; never block the next backend.
            os.waitpid(pid, os.WNOHANG)
    except ProcessLookupError:
        pass
