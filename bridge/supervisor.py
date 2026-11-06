"""Local stdio MCP adapter. No shell execution; only bounded Herdr controls."""
import json, os, pathlib, sqlite3, subprocess, sys, time, hashlib, re
from session_reads import read_session, inspect_sessions
import schedule_store
import checkins
from codex_workers import control as codex_control, DispatchRefused
HOME=pathlib.Path(os.environ['HERDR_APP_HOME'])
BIN=os.environ['HERDR_APP_BIN']
DB=HOME/'data/super-agent/ledger.sqlite'

from redaction import redact

def db():
    DB.parent.mkdir(parents=True,exist_ok=True,mode=0o700)
    c=sqlite3.connect(DB,timeout=5)
    c.execute('CREATE TABLE IF NOT EXISTS tasks(id TEXT PRIMARY KEY, value TEXT NOT NULL)')
    c.execute('CREATE TABLE IF NOT EXISTS sends(id TEXT PRIMARY KEY, arguments TEXT NOT NULL, result TEXT)')
    os.chmod(DB,0o600)
    return c

class Refused(ValueError):pass

def app(command,args=None):
    r=subprocess.run([BIN,'do',command,json.dumps(args or {},ensure_ascii=False)],capture_output=True,text=True,timeout=2.5 if command=='supervisor.read' else 20)
    if r.returncode:raise ValueError('Herdr control unavailable; inspect connectivity before retrying')
    v=json.loads(r.stdout)
    if isinstance(v,dict) and v.get('error'):raise Refused(v['error'])
    return v

def spec(name,description,properties,required=()):
    return {'name':name,'description':description,'inputSchema':{'type':'object','properties':properties,'required':list(required),'additionalProperties':False}}
STR={'type':'string'}
TOOLS=[
 spec('plan_followup','During a background observation, decide your own NEXT check: after_minutes 1..10080, or null to wait for the human/new evidence. publish=false suppresses this periodic report and its banner. Call before final text. Explain urgency/backoff in reason. This changes only your own follow-up clock, never worker tasks or user preferences.',{'after_minutes':{'type':['integer','null'],'minimum':1,'maximum':10080},'publish':{'type':'boolean'},'reason':STR},['after_minutes','publish','reason']),
 spec('get_checkin_settings','Read proactive progress-report preferences and delivery limits.',{}),
 spec('configure_checkins','Change proactive reporting only at the HUMAN request. idle_minutes is the initial inactivity delay (1..1440). interval_minutes is accepted for compatibility but does not control adaptive follow-ups. This never dispatches worker tasks.',{'enabled':{'type':'boolean'},'idle_minutes':{'type':'integer','minimum':1,'maximum':1440},'interval_minutes':{'type':'integer','minimum':1,'maximum':1440},'desktop_notifications':{'type':'boolean'},'user_intent_id':STR},['user_intent_id']),
 spec('list_sessions','List all automatically tracked local/remote sessions, connectivity, original status, existing machines and task ledger.',{}),
 spec('inspect_sessions','Read all worker sessions concurrently for an overview, including local source terminals and offline native history. Return evidence, connectivity and freshness separately; one failure never blocks the others.',{'addresses':{'type':'array','items':STR,'maxItems':24}}),
 spec('read_session','Read bounded evidence without focusing, scrolling or marking Done seen. Returns provenance, freshness, partial-history limits and offline cached evidence.',{'address':STR,'offset':{'type':'integer','minimum':0},'limit':{'type':'integer','minimum':1,'maximum':40}},['address']),
 spec('send_message','Delegate authorized work to an idle session without touching drafts. Use a unique request_id per assignment; reuse that ID on retry. Local Codex continues its exact existing runtime thread via turn/start, not terminal typing. A dispatch error is never permission to create a replacement worker. Never use this to answer an approval.',{'address':STR,'text':STR,'request_id':STR},['address','text','request_id']),
 spec('close_session','Archive exactly one explicitly user-authorized session from Herdr and its managed list; retain history/files and underlying runtime. Call directly, never delegate shell/terminal commands to a worker. Requires the latest user_intent_id and a stable request_id. Only report closed after verified=true; pending is not completion.',{'address':STR,'request_id':STR,'user_intent_id':STR},['address','request_id','user_intent_id']),
 spec('create_session','Create a new local/remote worker without changing user focus. Choose an existing SSH host for native Claude, or an existing core machine for terminal Codex. Creation is idempotent by request_id; send the assignment separately.',{'host':STR,'folder':STR,'agent':{'enum':['claude','codex']},'title':STR,'request_id':STR},['folder','agent','title','request_id']),
 spec('reply_to_question','Forward the latest HUMAN reply verbatim to an exact live Claude or Codex worker question. Read pending_permissions/pending_questions first, then get latest_user_intent.id from list_sessions. Cannot approve tools.',{'address':STR,'question_id':STR,'user_intent_id':STR},['address','question_id','user_intent_id']),
 spec('schedule_reminder','Persist an explicitly requested reminder. at must be ISO-8601 with offset; timezone is an IANA name. Supports once, daily, weekly. This never runs commands, approves or deploys. Report the returned delivery limits.',{'request_id':STR,'title':STR,'text':STR,'at':STR,'timezone':STR,'repeat':{'enum':['none','daily','weekly']},'user_intent_id':STR},['request_id','title','text','at','timezone','user_intent_id']),
 spec('list_reminders','Get the current local date/time for relative phrases, plus persisted reminders, recurrence and unread deliveries.',{'timezone':STR}),
 spec('cancel_reminder','Cancel a reminder and dismiss its unread deliveries only when the human requests it.',{'id':STR,'user_intent_id':STR},['id','user_intent_id']),
 spec('acknowledge_reminder','Mark a delivered reminder read only after an explicit human response; reading it yourself is not acknowledgment.',{'delivery_id':STR,'user_intent_id':STR},['delivery_id','user_intent_id']),
 spec('list_decisions','Read persistent human decision cards. pending needs a human reply; answered is queued and still needs follow-through; resolved is human-marked complete.',{}),
 spec('record_decision','Hang a concrete HUMAN decision or review in the right sidebar with full question, context and source evidence. Stable id deduplicates repeated observations. Never removes human replies/completion. Observational only: does not dispatch, answer or approve anything. Do not store credentials.',{'id':STR,'title':STR,'context':STR,'address':STR,'options':{'type':'array','items':STR,'maxItems':6},'evidence':STR},['id','title','context','address']),
 spec('record_task','Persist a task and its authority/evidence. awaiting_review survives idle/disconnect; accepted is only for explicit user acceptance.',{'id':STR,'title':STR,'address':STR,'mandate':STR,'stage':{'enum':['working','waiting_user','awaiting_review','accepted','failed','paused']},'summary':STR,'evidence':STR,'next_action':STR,'user_intent_id':STR},['id','title','address','mandate','stage','summary']),
]

def task_card(task):
    ident=task.get('decision_id') or 'task:'+task['id']+':'+task['stage']+':'+str(task.get('awaiting_since',task.get('updated_at',0)))
    return {'id':ident,'title':task['title'],'context':task['summary']+'\n\n'+task.get('next_action',''),'address':task['address'],'evidence':task.get('evidence','')}

def call(name,a):
    if name=='list_decisions':return app('supervisor.decisions')
    if name=='record_decision':
        if redact(a)!=a:raise ValueError('Do not store credentials in decision cards')
        snapshot=app('supervisor.list')
        if not any(s['address']==a.get('address') for s in snapshot['sessions']):raise ValueError('Use an exact current session address')
        return app('supervisor.decision',a)
    if name=='plan_followup':
        if redact(a)!=a:raise ValueError('Do not store credentials in follow-up reasons')
        return checkins.plan(HOME,a)
    if name=='get_checkin_settings':return checkins.describe(HOME)
    if name=='configure_checkins':
        intent=app('supervisor.list').get('latest_user_intent') or {}
        if not intent.get('id') or a.get('user_intent_id')!=intent['id']:raise ValueError('Use the latest explicit human request')
        return checkins.configure(HOME,a)
    if name=='list_reminders':return schedule_store.listing(HOME,a.get('timezone','Asia/Shanghai'))
    if name in ['schedule_reminder','cancel_reminder','acknowledge_reminder']:
        intent=app('supervisor.list').get('latest_user_intent') or {}
        if not intent.get('id') or a.get('user_intent_id')!=intent['id']:
            raise ValueError('Use the latest explicit human request from list_sessions')
        if name=='schedule_reminder':
            if redact(a)!=a:raise ValueError('Do not store credentials or authorization links in reminders')
            result=schedule_store.create(HOME,a,intent)
            return {**result,'delivery':schedule_store.install_clock(HOME)}
        if name=='cancel_reminder':return schedule_store.cancel(HOME,a['id'])
        return schedule_store.acknowledge_authorized(HOME,a['delivery_id'],intent)
    if name=='reply_to_question':
        if a['address'].startswith('chat:'):return app('supervisor.answer',a)
        path=HOME/'data/forge-chat/73757065726167656e74000000000001/events.jsonl'
        with path.open('rb') as f:
            size=f.seek(0,2);f.seek(max(0,size-2*1024*1024));lines=f.read().splitlines()
        intent=None
        for line in reversed(lines):
            try:e=json.loads(line)
            except ValueError:continue
            if e.get('type')=='user' and not e.get('id','').startswith('observe_'):intent=e;break
        if not intent or intent.get('id')!=a['user_intent_id']:raise ValueError('Use the latest explicit human reply')
        if redact(intent.get('text',''))!=intent.get('text',''):raise ValueError('Credentials cannot be relayed')
        return codex_control('answer',{'address':a['address'],'question_id':a['question_id'],'intent':intent})
    if name=='close_session':
        ident=a.get('request_id','')
        if not isinstance(ident,str) or not 8<=len(ident)<=100 or not re.fullmatch(r'[A-Za-z0-9_-]+',ident):raise ValueError('invalid request_id')
        snap=app('supervisor.list')
        previous=next((x for x in snap.get('closed_sessions',[]) if x.get('request_id')==ident),None)
        if previous:
            if previous['address']!=a['address']:raise ValueError('request_id reused for different session')
            identity=previous['identity']
        else:
            intent=snap.get('latest_user_intent') or {}
            if not intent.get('id') or a.get('user_intent_id')!=intent['id']:raise ValueError('Use latest explicit human authorization from list_sessions')
            row=next((x for x in snap['sessions'] if x['address']==a['address']),None)
            if not row:raise ValueError('Unknown exact session; nothing closed')
            if row.get('super_agent'):raise ValueError('Cannot close Super Agent')
            identity={'conversation':row['conversation'],'host':row.get('host')} if row.get('conversation') else row.get('read_identity')
            if not identity:raise ValueError('Session identity unavailable; nothing closed')
        result=app('supervisor.close',{**a,'confirmed':True,'expected_identity':identity})
        if result.get('pending'):
            for _ in range(20):
                latest=app('supervisor.list')
                record=next((x for x in latest.get('closed_sessions',[]) if x.get('request_id')==ident and x.get('identity')==identity),None)
                if record and record.get('pane_present') is False and not any(x['address']==a['address'] for x in latest['sessions']):
                    return {**result,'verified':True,'pending':False}
                time.sleep(.15)
        return result
    if name=='record_task':
        a=redact(a)
        if not all(isinstance(a.get(k),str) for k in ['id','title','address','mandate','stage','summary']):raise ValueError('invalid task')
        if a['stage'] not in ['working','waiting_user','awaiting_review','accepted','failed','paused']:raise ValueError('invalid stage')
        c=db();old=c.execute('SELECT value FROM tasks WHERE id=?',(a['id'],)).fetchone()
        previous=json.loads(old[0]) if old else {}
        closes=a['stage']=='accepted'
        resumes=previous.get('stage')=='awaiting_review' and a['stage']!='awaiting_review'
        if closes or resumes:
            intent=app('supervisor.list').get('latest_user_intent') or {}
            if not previous or not a.get('user_intent_id') or a['user_intent_id']!=intent.get('id') or intent.get('time',0)<=previous.get('awaiting_since',previous.get('updated_at',0)):
                raise ValueError('Pending review needs a newer explicit user intent from list_sessions; worker state is not acceptance')
        if a['stage']=='awaiting_review':a={**a,'awaiting_since':previous.get('awaiting_since',int(time.time()*1000))}
        a={**a,'updated_at':int(time.time()*1000)}
        if a['stage'] in ['waiting_user','awaiting_review']:
            a['decision_id']=previous.get('decision_id') if previous.get('stage')==a['stage'] else None
            if not a['decision_id']:a['decision_id']='task:'+a['id']+':'+a['stage']+':'+str(a['updated_at'])
        with c:c.execute('INSERT OR REPLACE INTO tasks VALUES (?,?)',(a['id'],json.dumps(a)))
        c.close()
        if a['stage']=='waiting_user':
            app('supervisor.decision',task_card(a))
        return a
    if name=='list_sessions':
        v=app('supervisor.list');c=db();v['tasks']=[{**json.loads(x[0]),'session_closed':not any(s['address']==json.loads(x[0]).get('address') for s in v['sessions']) and any(s['address']==json.loads(x[0]).get('address') for s in v.get('closed_sessions',[]))} for x in c.execute('SELECT value FROM tasks')];c.close()
        # Old task ledgers can contain superseded snapshots. The manager must
        # verify current questions before hanging them, rather than bulk-importing history.
        return v
    if name=='read_session':
        return read_session(HOME,app('supervisor.list'),a,app,redact)
    if name=='inspect_sessions':
        # Inventory is already published atomically by the GUI. Reading it avoids
        # waiting for the UI thread merely to inspect saved worker evidence.
        try:
            snap=json.loads((HOME/'data/super-agent/sessions.json').read_text())
            if not isinstance(snap.get('sessions'),list):raise ValueError('inventory')
        except (OSError,ValueError):snap=app('supervisor.list')
        return inspect_sessions(HOME,snap,a,app,redact)
    if name not in ['send_message','create_session']:raise ValueError('unknown tool')
    ident=a.get('request_id','')
    if not isinstance(ident,str) or not 8<=len(ident)<=100 or not all(ch.isascii() and (ch.isalnum() or ch in '_-') for ch in ident):raise ValueError('invalid request_id')
    snap=app('supervisor.list')
    if name=='send_message':
        if redact(a.get('text',''))!=a.get('text',''):raise ValueError('Credential-bearing assignments are not forwarded. Use references to configured secrets.')
        target=next((x for x in snap['sessions'] if x['address']==a['address']),None)
        if target is None:raise ValueError('Unknown address; use the exact address returned by list_sessions or create_session. Nothing sent.')
        if target.get('super_agent'):raise ValueError('Cannot delegate to yourself')
    encoded=hashlib.sha256(json.dumps({'tool':name,'args':a},sort_keys=True).encode()).hexdigest()
    c=db()
    with c:
        old=c.execute('SELECT arguments,result FROM sends WHERE id=?',(ident,)).fetchone()
        if old:
            if old[0]!=encoded:raise ValueError('request_id reused for different action')
            if old[1] is None:raise ValueError('Delivery unknown. Read session; do not resend automatically.')
            return json.loads(old[1])
        c.execute('INSERT INTO sends VALUES (?,?,NULL)',(ident,encoded))
    try:
        snap=app('supervisor.list')
        if name=='send_message' and any(x.get('super_agent') and x['address']==a['address'] for x in snap['sessions']):raise ValueError('Cannot delegate to yourself')
        if name=='create_session' and a['agent']=='claude':
            host=a.get('host') or '@local'
            if host!='@local' and not any(x.get('host')==host for x in snap['sessions']):raise ValueError('Unknown SSH host')
            # Only installs helper files; does not stop or restart user sessions.
            r=subprocess.run([BIN,'chat-install',host],capture_output=True,text=True,timeout=30)
            if r.returncode:raise ValueError('Could not install worker bridge')
        if name=='send_message' and not a['address'].startswith('chat:'):
            target=next((x for x in snap['sessions'] if x['address']==a['address']),None)
            if not target:raise Refused('Original session is unavailable; nothing sent')
            local_codex='/' not in a['address'] and target.get('host') in [None,'','@local'] and (target.get('app')=='codex' or target.get('program',{}).get('key')=='codex' or target.get('codex_thread_id') or target.get('read_source',{}).get('kind')=='codex_thread')
            if local_codex:
                try:result=codex_control('send',{**a,'read_identity':target.get('read_identity')})
                except DispatchRefused as error:raise Refused(str(error)) from error
            else:
                if target['status'] not in ['idle','done']:raise Refused('Terminal not idle; no message sent')
                if not a.get('text','').strip() or len(a['text'])>100000 or any(ord(ch)<32 and ch not in '\n\t' for ch in a['text']):raise Refused('Invalid message')
                r=subprocess.run([snap['core'],'send','--json','--if','idle',a['address'],a['text']],capture_output=True,text=True,timeout=15)
                if r.returncode:raise ValueError('Terminal send failed. Delivery is uncertain; inspect and repair this SAME session. Do not create a replacement worker')
                result={'core_receipt':json.loads(r.stdout),'request_id':ident}
        else:
            result=app('supervisor.send' if name=='send_message' else 'supervisor.create',a)
            if name=='create_session':
                result['address']='chat:'+result['pane'] if result.get('pane') else result.get('terminal')
    except Refused:
        if name=='send_message':
            with c:c.execute('DELETE FROM sends WHERE id=?',(ident,))
        c.close();raise
    except Exception:
        c.close();raise
    with c:c.execute('UPDATE sends SET result=? WHERE id=?',(json.dumps(result),ident))
    c.close();return result

def handle(r):
    method=r.get('method')
    if method=='initialize':return {'protocolVersion':'2024-11-05','capabilities':{'tools':{}},'serverInfo':{'name':'herdr-super-agent','version':'0.1.0'}}
    if method=='ping':return {}
    if method=='tools/list':return {'tools':TOOLS}
    if method=='tools/call':
        try:v=redact(call(r['params']['name'],r['params'].get('arguments',{})));return {'content':[{'type':'text','text':json.dumps(v,ensure_ascii=False)}]}
        except Exception as e:return {'content':[{'type':'text','text':redact(str(e))[:500]}],'isError':True}
    raise ValueError('unknown method')

if __name__=='__main__':
    if sys.argv[1:]==['--tools']:
        print(json.dumps(TOOLS));raise SystemExit(0)
    if len(sys.argv)==4 and sys.argv[1]=='--call':
        try:print(json.dumps(redact(call(sys.argv[2],json.loads(sys.argv[3]))),ensure_ascii=False))
        except Exception as e:print(json.dumps({'error':redact(str(e))[:500]}));raise SystemExit(1)
        raise SystemExit(0)
    for line in sys.stdin:
        if len(line)>300000:break
        try:
            r=json.loads(line)
            if 'id' not in r:continue
            response={'jsonrpc':'2.0','id':r['id'],'result':handle(r)}
        except Exception:response={'jsonrpc':'2.0','id':None,'error':{'code':-32603,'message':'Invalid request'}}
        print(json.dumps(response,ensure_ascii=False),flush=True)
