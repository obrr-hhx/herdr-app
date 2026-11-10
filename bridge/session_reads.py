"""Bounded, read-only session inspection, independent of terminal rendering."""
import hashlib
import json
import pathlib
import re
import subprocess
import time
from concurrent.futures import ThreadPoolExecutor
from codex_workers import control as codex_control


def now():
    return int(time.time() * 1000)


def command(args):
    r = subprocess.run(args, capture_output=True, text=True, timeout=2.5)
    if r.returncode:
        raise ValueError("source_read_failed")
    return r.stdout


def attach_source(record):
    a = record.get('foreground', {}).get('argv') or record.get('argv', [])
    if len(a) > 4 and a[2] == '"$@"; exec "$0" -li':
        a = a[4:]
    if not a or pathlib.Path(a[0]).name != 'herdr':
        return None
    session, i = (a[2], 3) if len(a) > 2 and a[1] == '--session' else ('default', 1)
    if a[i:i+2] != ['agent', 'attach'] or len(a) <= i+2:
        return None
    return {'kind': 'herdr_attach', 'binary': a[0], 'session': session, 'pane': a[i+2]}


def identity(row):
    return row.get('conversation') or row.get('read_identity') or row.get('read_source')


def cache_path(home, row):
    key = hashlib.sha256(row['address'].encode()).hexdigest()
    return home / 'data/super-agent/reads' / (key + '.json')


def cached(home, row):
    try:
        v = json.loads(cache_path(home, row).read_text())
        # Never reuse another occupant's history when an address is reused.
        return v['read'] if identity(row) and v['identity'] == identity(row) else None
    except (OSError, ValueError, KeyError):
        return None


def save(home, row, value, redact):
    if not identity(row):
        return
    p = cache_path(home, row)
    p.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    # Parallel inspections use separate temporary files and atomic replacement.
    import tempfile
    with tempfile.NamedTemporaryFile(mode='w', dir=p.parent, delete=False) as f:
        json.dump({'identity': identity(row), 'read': redact(value)}, f, ensure_ascii=False)
        temp = pathlib.Path(f.name)
    temp.replace(p)


def local_native(home, row, offset, limit):
    ident = row.get('conversation', '')
    if not re.fullmatch('[a-f0-9]{16,64}', ident):
        raise ValueError('native_history_not_located')
    p = home / 'data/forge-chat' / ident / 'events.jsonl'
    with p.open('rb') as f:
        size = f.seek(0, 2)
        start = max(0, size - 2 * 1024 * 1024)
        f.seek(start)
        if start:
            f.readline()
        data = f.read()
    messages, pending, updated, malformed = {}, {}, 0, 0
    dispatches, active_turn = {}, None
    for line in data.splitlines(keepends=True):
        if not line.endswith(b'\n'):
            continue  # A concurrent append is not a malformed source record.
        try:
            e = json.loads(line)
        except ValueError:
            malformed += 1
            continue
        kind, key = e.get('type'), e.get('id')
        if 'active_turn' in e:active_turn=e['active_turn']
        if kind in ('dispatch_receipt','prompt_confirmed') and isinstance(key,str):
            dispatches.pop(key,None);dispatches[key]={k:e[k] for k in ['id','mode','stage','reason','time'] if k in e}
            if kind=='prompt_confirmed':dispatches[key]['stage']='confirmed'
            if len(dispatches)>32:dispatches.pop(next(iter(dispatches)))
        if kind == 'permission':
            pending[key] = e
        elif kind == 'permission_resolved':
            pending.pop(key, None)
        if kind not in ('user', 'delta', 'assistant', 'tool', 'tool_result') or not key:
            continue
        if not isinstance(key, str):
            continue
        if kind == 'user' and key.startswith('observe_'):
            continue
        updated = max(updated, e.get('time', 0))
        text = e.get('text', '')
        role = 'You' if kind == 'user' else 'Assistant'
        if kind == 'assistant':
            text = ''.join(b.get('text', '') for b in e.get('content', []) if b.get('type') == 'text')
        elif kind in ('tool', 'tool_result'):
            role = e.get('name', 'Tool result')
            text = json.dumps(e.get('input', e.get('content', '')), ensure_ascii=False)[:3000]
        if kind == 'delta' and key in messages:
            messages[key]['text'] = (messages[key]['text'] + text)[-24000:]
        else:
            messages[key] = {'id': key, 'role': role, 'text': str(text)[-24000:]}
    rows = list(messages.values())
    end = max(0, len(rows) - offset)
    begin = max(0, end - limit)
    return {'source': 'native_journal', 'messages': rows[begin:end],
            'pending_permissions': pending, 'dispatches':dispatches, 'active_turn':active_turn, 'content_updated_at': updated,
            'next_offset': offset + end - begin, 'has_more': begin > 0 or start > 0,
            'partial': start > 0 or malformed > 0, 'malformed_records_skipped': malformed}


def decorate(row, value, snapshot_at):
    detail = row.get('detail', '') or ''
    offline = bool(re.search('connect|SSH|unavailable', detail, re.I))
    stale = not snapshot_at or now() - snapshot_at > 30000
    uncertain = offline or stale or detail == 'starting' or value.get('cached', False)
    return {**value, 'address': row['address'], 'title': value.get('source_title') or row.get('title') or row.get('tab_name'),
            'host': row.get('host'), 'reported_status': row.get('status'),
            'execution_state': 'unknown' if uncertain else value.get('execution_state', row.get('execution_state', row.get('status'))),
            'connectivity': 'offline' if offline else ('starting' if detail == 'starting' else 'online'),
            'status_observed_at': snapshot_at, 'inventory_stale': stale, 'last_known_only': uncertain}


def read_session(home, snap, args, app, redact):
    row = next((r for r in snap['sessions'] if r['address'] == args['address']), None)
    if row is None:
        raise ValueError('No such managed session')
    offset, limit = max(0, int(args.get('offset', 0))), min(40, max(1, int(args.get('limit', 12))))
    began = time.monotonic()
    source, failure = row.get('read_source'), None
    try:
        if row.get('app')=='codex' or row.get('program',{}).get('key')=='codex' or row.get('read_source',{}).get('kind')=='codex_thread':
            state=json.loads((home/'data/super-agent/codex-workers.json').read_text())
            source_state=state.get('sessions',{}).get(row['address'],{})
            try:source_state=codex_control('read',{'address':row['address'],'offset':offset,'limit':limit})
            except (OSError,ValueError):pass
            if source_state.get('read_identity')!=row.get('read_identity'):raise ValueError('Codex source identity changed')
            messages=source_state.get('messages',[]);end=max(0,len(messages)-offset);begin=max(0,end-limit)
            live=bool(source_state.get('binding_live') and source_state.get('connected') and now()-source_state.get('at',0)<15000)
            result={**source_state,'messages':messages if source_state.get('history_paginated') else messages[begin:end],'pending_questions':source_state.get('pending_questions',{}),'source':'codex_app_server','cached':not live,'available':bool(source_state.get('available')),'partial':True,'next_offset':source_state.get('next_offset',offset+end-begin),'has_more':source_state.get('has_more',begin>0),'history_excerpt_limited':not source_state.get('history_paginated',False),'content_updated_at':source_state.get('at'),'captured_at':source_state.get('at'),'last_known_only':not live}
            result=decorate(row,result,snap.get('at'))
            if not live:result.update(execution_state='unknown',last_known_only=True)
            result['read_ms']=int((time.monotonic()-began)*1000)
            return redact(bounded(result,64000))
        if row['address'].startswith('chat:'):
            # The local event journal remains readable while SSH is down and
            # while a pane is not instantiated. It never marks a result seen.
            try:
                result = local_native(home, row, offset, limit)
            except (OSError, ValueError):
                result = {**app('supervisor.read', {**args, 'limit': limit}), 'source': 'native_history', 'content_updated_at': None}
        else:
            if not source:
                record = json.loads(command([snap['core'], 'get', '--json', row['address']]))
                row = {**row, 'read_identity': {'pid': record.get('pid'), 'created_ms': record.get('created_ms')}}
                source = attach_source(record)
            if source and source.get('kind') == 'herdr_attach':
                base = [source['binary'], '--session', source['session']]
                raw = command(base + ['agent', 'read', source['pane'], '--source', 'visible', '--format', 'text'])
                # Only visible output: asking for alternate-screen history can
                # scroll the person's terminal or block on a working agent.
                result = {'source': 'herdr_visible', 'text': raw[-24000:], 'partial': True,
                          'warning': 'Visible source terminal only; earlier history is not included. No scrolling or input was performed.'}
                try:
                    agent = json.loads(command(base + ['agent', 'get', source['pane']]))['result']['agent']
                    result['execution_state'] = agent.get('agent_status', 'unknown')
                    result['source_title'] = agent.get('terminal_title_stripped')
                except (OSError, ValueError, KeyError, subprocess.TimeoutExpired):
                    pass
            else:
                raw = json.loads(command([snap['core'], 'read', '--json', '--lines', '100', row['address']]))
                result = {'source': 'terminal_screen', 'text': str(raw.get('text', ''))[-24000:],
                          'partial': not raw.get('complete', False),
                          'warning': 'Terminal snapshot, not structured message history'}
        result.update(captured_at=now(), cached=False, available=True)
        save(home, row, result, redact)
    except (OSError, ValueError, KeyError, subprocess.TimeoutExpired):
        if row.get('app')=='codex' or row.get('program',{}).get('key')=='codex':
            return decorate(row,{'source':'codex_app_server_unavailable','available':False,'cached':True,'execution_state':'unknown','last_known_only':True,'warning':'Structured Codex source is synchronizing or disconnected. Do not infer question content from a terminal title.'},snap.get('at'))
        failure = 'source_timeout_or_unavailable'
        result = cached(home, row)
        if result:
            result = {**result, 'cached': True, 'available': True, 'read_error': failure}
        else:
            result = {'source': 'unavailable', 'available': False, 'cached': False, 'read_error': failure,
                      'warning': 'No readable evidence. Do not infer progress from the title.'}
    result = decorate(row, result, snap.get('at'))
    result['read_ms'] = int((time.monotonic() - began) * 1000)
    content_at = result.get('content_updated_at', result.get('captured_at'))
    result['age_ms'] = max(0, now() - content_at) if content_at else None
    result = bounded(result, 64000)
    return redact(result)


def bounded(value, budget):
    """Keep batched tool output below the bridge frame limit, with honest limits."""
    remaining, clipped = budget, False
    def clip(text):
        nonlocal remaining, clipped
        if not isinstance(text, str):
            return text
        raw = text.encode('utf-8')
        if len(raw) > remaining:
            clipped = True
        out = raw[:remaining].decode('utf-8', errors='ignore')
        remaining = max(0, remaining - len(out.encode('utf-8')))
        return out
    value = dict(value)
    if 'text' in value:
        value['text'] = clip(value['text'])
    messages = []
    for m in reversed(value.get('messages', [])):
        messages.append({**m, 'text': clip(m.get('text', ''))})
    if 'messages' in value:
        value['messages'] = list(reversed(messages))
    if 'pending_permissions' in value:
        value['pending_permissions'] = {k: {f: p.get(f) for f in ('id','name','time')} | {'input_excerpt':clip(json.dumps(p.get('input',p.get('input_excerpt',{})),ensure_ascii=False))} for k,p in value['pending_permissions'].items()}
    if clipped:
        value.update(excerpt_truncated=True, partial=True)
    return value

def inspect_sessions(home, snap, args, app, redact):
    addresses = args.get('addresses')
    rows = [s for s in snap['sessions'] if not s.get('super_agent') and (addresses is None or s['address'] in addresses)]
    total = len(rows)
    rows = rows[:24]
    def inspect(row):
        try:
            return bounded(read_session(home, snap, {'address': row['address'], 'limit': 4}, app, redact), 12000)
        except Exception:
            return decorate(row, {'source': 'unavailable', 'available': False, 'read_error': 'inspection_failed'}, snap.get('at'))
    with ThreadPoolExecutor(max_workers=8) as pool:
        results = list(pool.map(inspect, rows))
    return {'sessions': results, 'inspected_at': now(), 'truncated': total > 24,
            'available': sum(bool(r.get('available')) for r in results),
            'unavailable': sum(not r.get('available') for r in results)}
