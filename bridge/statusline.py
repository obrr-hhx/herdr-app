"""Receive native Claude status-line data. Persist only allowlisted usage metadata."""
import json
import math
import os
import pathlib
import re
import subprocess
import sys
import time


def number(value, maximum=None):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0:
        return None
    return min(value, maximum) if maximum is not None else value


def text(value, limit=100):
    if not isinstance(value, str):
        return ''
    value = re.sub(r'\x1b\][^\x07]*(?:\x07|\x1b\\)|\x1b\[[0-?]*[ -/]*[@-~]|[\x00-\x1f\x7f]', '', value)
    value = re.sub(r'(?:sk-|ghp_|github_pat_|xoxb-)[A-Za-z0-9_-]{12,}|https?://\S+', '[redacted]', value)
    return value[:limit]


def extract(data):
    model = data.get('model') or {}
    context = data.get('context_window') or {}
    cost = data.get('cost') or {}
    limits = data.get('rate_limits') or {}
    usage = context.get('current_usage') or {}
    out = {
        'updated_at': int(time.time() * 1000),
        'model': text(model.get('display_name')),
        'context_pct': number(context.get('used_percentage'), 100),
        'context_size': number(context.get('context_window_size')),
        'input_tokens': number(context.get('total_input_tokens')),
        'output_tokens': number(context.get('total_output_tokens')),
        'cache_read_tokens': number(usage.get('cache_read_input_tokens')),
        'cost_usd': number(cost.get('total_cost_usd')),
        'duration_ms': number(cost.get('total_duration_ms')),
        'api_duration_ms': number(cost.get('total_api_duration_ms')),
        'lines_added': number(cost.get('total_lines_added')),
        'lines_removed': number(cost.get('total_lines_removed')),
        'branch': '',
    }
    for source, key in [('five_hour', 'five_hour'), ('seven_day', 'seven_day')]:
        limit = limits.get(source) or {}
        out[key] = {'used_pct': number(limit.get('used_percentage'), 100), 'resets_at': number(limit.get('resets_at'))}
    return out


def collect(root, data):
    root = pathlib.Path(root)
    with (root / 'events.jsonl').open() as f:
        first = json.loads(f.readline(1048576))
    if data.get('session_id') != first.get('session_id') or first.get('type') != 'session':
        return None
    out = extract(data)
    cwd = (data.get('workspace') or {}).get('current_dir') or data.get('cwd')
    if isinstance(cwd, str) and os.path.isabs(cwd):
        try:
            branch = subprocess.run(['git', '--no-optional-locks', '-C', cwd, 'symbolic-ref', '--quiet', '--short', 'HEAD'], capture_output=True, text=True, timeout=.5)
            if branch.returncode == 0:
                out['branch'] = text(branch.stdout.strip())
        except (OSError, subprocess.TimeoutExpired):
            pass
    temp = root / ('statusline.' + str(os.getpid()) + '.tmp')
    try:
        fd = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, 'w') as f:
            json.dump(out, f, separators=(',', ':'))
        os.replace(temp, root / 'statusline.json')
    finally:
        temp.unlink(missing_ok=True)
    return out


def main():
    raw = sys.stdin.buffer.read(1048577)
    if len(raw) > 1048576:
        return
    data = json.loads(raw)
    base = pathlib.Path(__file__).resolve().parent
    out = None
    if sys.argv[1] == '--dispatch':
        sid = data.get('session_id', '')
        if isinstance(sid, str) and re.fullmatch(r'[a-fA-F0-9-]{36}', sid):
            try:
                record = json.loads((base / 'statusline-sessions' / (sid + '.json')).read_text())
                root = pathlib.Path(record['folder']).resolve()
                if root.is_relative_to(base / 'conversations'):
                    out = collect(root, data)
            except (OSError, ValueError, KeyError):
                pass
    else:
        out = collect(pathlib.Path(sys.argv[1]), data)
    # Preserve the existing native terminal formatter, including its colours.
    original = base / 'statusline-original.json'
    if original.exists():
        try:
            config = json.loads(original.read_text())
            command = config.get('command') if isinstance(config, dict) else None
            if command:
                result = subprocess.run(command, shell=True, input=raw, capture_output=True, timeout=3)
                if result.returncode == 0:
                    sys.stdout.buffer.write(result.stdout)
                return
        except (OSError, ValueError, subprocess.TimeoutExpired):
            pass
    if sys.argv[1] != '--dispatch' or out is not None:
        print((out or {}).get('model') or 'Claude')


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, TypeError, AttributeError):
        # Status display must never block the interactive agent.
        pass
