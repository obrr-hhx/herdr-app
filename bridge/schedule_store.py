"""Durable reminders. The clock never executes commands or approves worker work."""
import datetime as dt
import hashlib
import json
import os
import pathlib
import plistlib
import re
import sqlite3
import subprocess
import sys
import tempfile
import time
from zoneinfo import ZoneInfo


def now_ms():
    return int(time.time() * 1000)


def connect(home):
    path = pathlib.Path(home) / 'data/super-agent/ledger.sqlite'
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    c = sqlite3.connect(path, timeout=3)
    c.row_factory = sqlite3.Row
    c.execute('CREATE TABLE IF NOT EXISTS schedules(id TEXT PRIMARY KEY, payload TEXT NOT NULL, next_at INTEGER NOT NULL, state TEXT NOT NULL)')
    c.execute('CREATE TABLE IF NOT EXISTS reminder_deliveries(id TEXT PRIMARY KEY, schedule_id TEXT NOT NULL, due_at INTEGER NOT NULL, payload TEXT NOT NULL, seen_at INTEGER, notify_state TEXT NOT NULL DEFAULT \'pending\', notify_at INTEGER)')
    c.execute('CREATE INDEX IF NOT EXISTS schedule_due ON schedules(state,next_at)')
    os.chmod(path, 0o600)
    return c


def valid_id(value):
    if not isinstance(value, str) or not re.fullmatch(r'[A-Za-z0-9_-]{8,100}', value):
        raise ValueError('Use a stable request_id of 8-100 letters, digits, underscores or hyphens')
    return value


def public(row):
    value = json.loads(row['payload'])
    zone = ZoneInfo(value['timezone'])
    return {**value, 'state': row['state'], 'next_at': row['next_at'],
            'next_local': dt.datetime.fromtimestamp(row['next_at']/1000, zone).isoformat()}


def create(home, args, intent, clock=None):
    clock = now_ms() if clock is None else clock
    ident = valid_id(args['request_id'])
    if not intent.get('id') or args.get('user_intent_id') != intent['id']:
        raise ValueError('Scheduling requires the latest explicit human request from list_sessions')
    text, title = args.get('text', ''), args.get('title', '')
    if not isinstance(text, str) or not text.strip() or len(text) > 12000 or not isinstance(title,str) or not title.strip() or len(title)>160:
        raise ValueError('A title and bounded reminder text are required')
    zone = ZoneInfo(args.get('timezone', 'Asia/Shanghai'))
    at = dt.datetime.fromisoformat(args['at'].replace('Z', '+00:00'))
    if at.tzinfo is None:
        raise ValueError('at must include an explicit timezone offset; never guess local time')
    due = int(at.timestamp()*1000)
    repeat = args.get('repeat', 'none')
    if repeat not in ('none','daily','weekly'):
        raise ValueError('repeat must be none, daily or weekly')
    payload = {'id':ident, 'title':title.strip(), 'text':text.strip(), 'timezone':zone.key,
               'at':at.astimezone(zone).isoformat(), 'repeat':repeat, 'user_intent_id':intent['id']}
    c = connect(home)
    try:
        c.execute('BEGIN IMMEDIATE')
        old = c.execute('SELECT * FROM schedules WHERE id=?',(ident,)).fetchone()
        if old:
            previous = json.loads(old['payload'])
            if any(previous.get(k)!=v for k,v in payload.items()):
                raise ValueError('request_id already identifies a different reminder; cancel it and use a new ID')
            c.commit()
            return public(old)
        if due < clock - 30000:
            raise ValueError('Requested time is in the past; confirm the intended date instead of silently moving it')
        if c.execute("SELECT count(*) FROM schedules WHERE state='pending'").fetchone()[0]>=200:
            raise ValueError('Too many active reminders; cancel unused reminders first')
        payload['created_at']=clock
        c.execute('INSERT INTO schedules VALUES (?,?,?,?)',(ident,json.dumps(payload,ensure_ascii=False),due,'pending'))
        c.commit()
        return public(c.execute('SELECT * FROM schedules WHERE id=?',(ident,)).fetchone())
    finally:
        c.close()


def listing(home, timezone="Asia/Shanghai"):
    c=connect(home)
    try:
        schedules=[public(r) for r in c.execute('SELECT * FROM schedules ORDER BY next_at DESC LIMIT 500')]
        pending=[dict(r) for r in c.execute('SELECT id,schedule_id,due_at FROM reminder_deliveries WHERE seen_at IS NULL ORDER BY due_at')]
        return {'schedules':schedules,'unread_reminders':pending,'now':now_ms(),'now_local':dt.datetime.now(ZoneInfo(timezone)).isoformat(),'timezone':timezone}
    finally:
        c.close()


def cancel(home, ident):
    c=connect(home)
    try:
        with c:
            row=c.execute('SELECT * FROM schedules WHERE id=?',(ident,)).fetchone()
            if not row:
                raise ValueError('No such schedule')
            c.execute("UPDATE schedules SET state='cancelled' WHERE id=?",(ident,))
            c.execute('UPDATE reminder_deliveries SET seen_at=coalesce(seen_at,?) WHERE schedule_id=?',(now_ms(),ident))
        return {'id':ident,'state':'cancelled','note':'Future occurrences cancelled and existing unread reminders dismissed'}
    finally:
        c.close()


def acknowledge(home, ids, clock=None):
    if not isinstance(ids,list) or len(ids)>128 or not all(isinstance(x,str) and len(x)<180 for x in ids):
        raise ValueError('Invalid reminder IDs')
    c=connect(home)
    try:
        with c:
            for ident in ids:
                c.execute('UPDATE reminder_deliveries SET seen_at=coalesce(seen_at,?) WHERE id=?',(now_ms() if clock is None else clock,ident))
        return {'acknowledged':ids}
    finally:
        c.close()



def acknowledge_authorized(home, ident, intent):
    c=connect(home)
    try:
        row=c.execute('SELECT payload FROM reminder_deliveries WHERE id=?',(ident,)).fetchone()
        if not row:raise ValueError('No such delivered reminder')
        delivery=json.loads(row['payload'])
        if intent.get('id')==delivery.get('user_intent_id') or intent.get('time',0)<=delivery['triggered_at']:
            raise ValueError('A newer explicit human acknowledgment is required; reading the reminder is not acceptance')
    finally:c.close()
    return acknowledge(home,[ident])


def next_occurrence(payload, due, clock):
    zone=ZoneInfo(payload['timezone'])
    previous=dt.datetime.fromtimestamp(due/1000,zone)
    today=dt.datetime.fromtimestamp(clock/1000,zone)
    step=1 if payload['repeat']=='daily' else 7
    days=max(step, ((today.date()-previous.date()).days//step)*step)
    original=dt.datetime.fromisoformat(payload['at']).astimezone(zone)
    target=previous.date()+dt.timedelta(days=days)
    following=dt.datetime.combine(target,original.time(),zone)
    while following.timestamp()*1000<=clock:
        target+=dt.timedelta(days=step)
        following=dt.datetime.combine(target,original.time(),zone)
    return int(following.timestamp()*1000)


def desktop_notification(title,text,ident,home=None):
    if sys.platform!='darwin':return False
    home=pathlib.Path(home or os.environ.get('HERDR_APP_HOME',pathlib.Path.home()/'.config/herdr-app'))
    try:
        config=json.loads((home/'data/super-agent/scheduler/native-executable.json').read_text())
        exe=config['executable']
        payload=json.dumps({'id':ident,'title':title,'text':text[:220]},ensure_ascii=False)
        r=subprocess.run([exe,'reminder-notify',payload],capture_output=True,timeout=12,env={**os.environ,'HERDR_APP_HOME':str(home)})
        return r.returncode==0
    except (OSError,ValueError,KeyError,subprocess.TimeoutExpired):return False


def poll(home, known=(), clock=None, notify=desktop_notification):
    clock=now_ms() if clock is None else clock
    c=connect(home)
    try:
        # Due delivery is committed BEFORE any external notification. Multiple
        # clocks (launchd and the app) cannot create the same occurrence twice.
        with c:
            c.execute('BEGIN IMMEDIATE')
            for row in c.execute("SELECT * FROM schedules WHERE state='pending' AND next_at<=? ORDER BY next_at LIMIT 128",(clock,)).fetchall():
                p=json.loads(row['payload']);ident=row['id']+'@'+str(row['next_at'])
                delivery={**p,'at':dt.datetime.fromtimestamp(row['next_at']/1000,ZoneInfo(p['timezone'])).isoformat(),'delivery_id':ident,'due_at':row['next_at'],'triggered_at':clock,'overdue_ms':max(0,clock-row['next_at'])}
                c.execute('INSERT OR IGNORE INTO reminder_deliveries(id,schedule_id,due_at,payload) VALUES (?,?,?,?)',(ident,row['id'],row['next_at'],json.dumps(delivery,ensure_ascii=False)))
                if p['repeat']=='none':
                    c.execute("UPDATE schedules SET state='fired' WHERE id=?",(row['id'],))
                else:
                    c.execute('UPDATE schedules SET next_at=? WHERE id=?',(next_occurrence(p,row['next_at'],clock),row['id']))
        # A lease retries a failed/crashed desktop request, while unread in-app
        # delivery remains durable regardless of notification permission/network.
        for row in c.execute("SELECT id,payload FROM reminder_deliveries WHERE seen_at IS NULL AND (notify_state='pending' OR notify_state IN ('sending','failed') AND notify_at<?) ORDER BY due_at LIMIT 32",(clock-300000,)).fetchall():
            with c:
                claimed=c.execute("UPDATE reminder_deliveries SET notify_state='sending',notify_at=? WHERE id=? AND seen_at IS NULL AND (notify_state='pending' OR notify_at<?)",(clock,row['id'],clock-300000)).rowcount
            if claimed:
                p=json.loads(row['payload']);ok=notify(p['title'],p['text'],row['id'])
                with c:
                    c.execute('UPDATE reminder_deliveries SET notify_state=? WHERE id=?',('requested' if ok else 'failed',row['id']))
        deliveries=[]
        known_set=set(known)
        for r in c.execute('SELECT id,payload FROM reminder_deliveries WHERE seen_at IS NULL ORDER BY due_at'):
            if r['id'] not in known_set:deliveries.append(json.loads(r['payload']))
            if len(deliveries)>=32:break
        acknowledged=[]
        for ident in known:
            row=c.execute('SELECT seen_at FROM reminder_deliveries WHERE id=?',(ident,)).fetchone()
            if row and row['seen_at'] is not None:
                acknowledged.append(ident)
        return {'deliveries':deliveries,'acknowledged':acknowledged}
    finally:
        c.close()


def atomic(path, data):
    path.parent.mkdir(parents=True,exist_ok=True,mode=0o700)
    with tempfile.NamedTemporaryFile(dir=path.parent,delete=False) as f:
        f.write(data);name=f.name
    os.replace(name,path)


def install_clock(home):
    home=pathlib.Path(home).resolve()
    # Isolated homes never install a login item or emit desktop notifications.
    if sys.platform!='darwin' or home!=(pathlib.Path.home()/'.config/herdr-app').resolve():
        return {'mode':'app_service','background_login_clock':False,'note':'Clock runs with the local service; overdue reminders return on app restart'}
    native=os.environ.get('HERDR_APP_BIN')
    if native:atomic(home/'data/super-agent/scheduler/native-executable.json',json.dumps({'executable':native}).encode())
    worker=home/'data/super-agent/scheduler/schedule_store.py'
    atomic(worker,pathlib.Path(__file__).read_bytes())
    label='dev.herdr.app.reminders.'+hashlib.sha256(str(home).encode()).hexdigest()[:12]
    plist=pathlib.Path.home()/'Library/LaunchAgents'/f'{label}.plist'
    python=next((p for p in ['/opt/homebrew/bin/python3','/usr/local/bin/python3','/usr/bin/python3'] if pathlib.Path(p).exists()),sys.executable)
    payload=plistlib.dumps({'Label':label,'ProgramArguments':[python,str(worker),'--tick',str(home)],'RunAtLoad':True,'StartInterval':30,'ProcessType':'Background'})
    domain=f'gui/{os.getuid()}'
    try:
        loaded=subprocess.run(['launchctl','print',domain+'/'+label],capture_output=True,timeout=5).returncode==0
        changed=not plist.exists() or plist.read_bytes()!=payload
        if changed:
            atomic(plist,payload)
        if loaded and changed:
            subprocess.run(['launchctl','bootout',domain+'/'+label],capture_output=True,timeout=5)
            loaded=False
        if not loaded:
            subprocess.run(['launchctl','bootstrap',domain,str(plist)],capture_output=True,timeout=8,check=True)
        return {'mode':'macos_login_clock','background_login_clock':True,'resolution_seconds':30,'notification_sender':'Herdr App','note':'Runs while signed in and awake; overdue reminders catch up after wake/login. macOS controls desktop banner permission.'}
    except (OSError,subprocess.SubprocessError):
        return {'mode':'app_service','background_login_clock':False,'note':'Login clock unavailable; reminders still persist and run with the local service'}


if __name__=='__main__':
    mode,home=sys.argv[1:3]
    args=json.loads(sys.stdin.read() or '{}') if mode!='--tick' else {}
    if mode in ('--poll','--tick'):
        # Acceptance homes do not send OS notifications to the person's desktop.
        notify=(lambda *args:desktop_notification(*args,home=home)) if pathlib.Path(home).resolve()==(pathlib.Path.home()/'.config/herdr-app').resolve() else lambda *_:False
        result=poll(home,args.get('known',[]),notify=notify)
        if mode=='--poll':
            print(json.dumps(result,ensure_ascii=False))
    elif mode=='--ack':
        print(json.dumps(acknowledge(home,args.get('ids',[]))))
