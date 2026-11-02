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

def shutdown(signum, frame):
    raise SystemExit(0)

signal.signal(signal.SIGTERM, shutdown)
signal.signal(signal.SIGINT, shutdown)
folder, cwd, model, session = sys.argv[1:]
root = pathlib.Path(folder)
settings = root / 'settings.json'
args = [os.environ.get('HERDR_CHAT_CLAUDE', 'claude'), '--model', model,
        '--settings', str(settings), '--permission-mode', 'default', '--ax-screen-reader']
args += ['--resume', session] if (root / 'has-session').exists() else ['--session-id', session]
if os.environ.get('HERDR_CHAT_TEST_ISOLATED') == '1':
    args += ['--setting-sources', '', '--strict-mcp-config', '--mcp-config', '{"mcpServers":{}}', '--effort', 'low']
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
            # Connecting a chat grants access to its selected working directory.
            if not trusted and b'Enter y/n:' in buffer and b'trust this folder' in buffer:
                os.write(fd, b'y\r')
                trusted = True
                buffer = b''
        if sys.stdin.buffer in ready:
            chunk = os.read(sys.stdin.fileno(), 65536)
            if not chunk:
                break
            commands += chunk
            while b'\n' in commands:
                line, commands = commands.split(b'\n', 1)
                c = json.loads(line)
                if c['action'] == 'interrupt':
                    os.write(fd, b'\x1b')
                elif c['action'] == 'send':
                    text = c['text']
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
                    os.write(fd, b'\r')
                    report('submitted', request_id=c['request_id'])
finally:
    try:
        os.kill(pid, signal.SIGTERM)
        os.waitpid(pid, 0)
    except ProcessLookupError:
        pass
