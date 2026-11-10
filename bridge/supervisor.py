"""Local stdio MCP adapter. No shell execution; only bounded Herdr controls."""
import json, os, pathlib, sqlite3, subprocess, sys, time, hashlib, re, shlex, glob, fnmatch
from session_reads import read_session, inspect_sessions
import session_release
import goal_watch
import goal_store
import memory_store
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

def configured_ssh_host(host, config=None):
    """Accept explicit user SSH aliases, including aliases in Include files.

    A wildcard Host block supplies defaults; it does not register every name
    as a machine the manager may create workers on. Never run Match exec.
    """
    if not isinstance(host,str) or not re.fullmatch(r'[A-Za-z0-9_][A-Za-z0-9_.@:-]{0,199}',host):return False
    config=pathlib.Path(config) if config is not None else pathlib.Path.home()/'.ssh/config'
    base=config.parent;pending=[config];seen=set()
    while pending and len(seen)<128:
        path=pending.pop().resolve()
        if path in seen:continue
        seen.add(path)
        try:lines=path.read_text().splitlines()
        except (OSError,UnicodeError):continue
        for line in lines:
            try:words=shlex.split(line,comments=True)
            except ValueError:continue
            if not words:continue
            key=words.pop(0)
            if '=' in key:
                key,value=key.split('=',1);words.insert(0,value)
            elif words and words[0]=='=':words.pop(0)
            key=key.lower()
            if key=='host':
                excluded=any(fnmatch.fnmatchcase(host.lower(),alias[1:].lower()) for alias in words if alias.startswith('!'))
                if not excluded and any(alias.lower()==host.lower() for alias in words if not any(ch in alias for ch in '*?!')):return True
            if key=='include':
                for value in words:
                    include=pathlib.Path(value).expanduser()
                    if not include.is_absolute():include=base/include
                    pending.extend(pathlib.Path(p) for p in glob.glob(str(include))[:128])
    return False

def app(command,args=None):
    r=subprocess.run([BIN,'do',command,json.dumps(args or {},ensure_ascii=False)],capture_output=True,text=True,timeout=2.5 if command=='supervisor.read' else 20)
    # The CLI exits 1 for BOTH application refusals and transport errors.
    # Only explicit pre-send refusals establish that no delivery occurred.
    try:v=json.loads(r.stdout)
    except (ValueError,TypeError):v=None
    if isinstance(v,dict) and v.get('error'):
        error=v['error']
        refused={'User draft in progress; no message sent','Session is not idle; no message sent',
                 'Could not queue message','Invalid message','Invalid request ID',
                 'No such native session','Use the core adapter for terminal sessions',
                 'supervisor.send needs text','supervisor.send needs address','supervisor.send needs request_id',
                 'supervisor.send: unknown argument(s): mode; valid arguments: address, text, request_id',
                 'Native steering unavailable; update this runtime when idle; nothing sent','No verified active turn; nothing sent',
                 'Pending approval blocks steering; nothing sent','Steering accepts conversation text only; nothing sent',
                 'Unsupported follow-up mode; nothing sent'}
        if command=='supervisor.send' and error in refused:raise Refused(error)
        # Do not reflect arbitrary error strings (they may contain credentials),
        # or treat a window timeout as proof that a command was never applied.
        raise ValueError('Herdr window did not confirm the command; delivery may be unknown. Inspect this same session before retrying')
    if r.returncode or not isinstance(v,dict):raise ValueError('Herdr control unavailable; inspect connectivity before retrying')
    return v

def native_steer_receipt(args,result,wait=False):
    if args.get('mode')!='steer' or not args['address'].startswith('chat:'):return result
    for i in range(12 if wait else 1):
        try:receipt=app('supervisor.read',{'address':args['address'],'limit':1}).get('dispatches',{}).get(args['request_id'])
        except ValueError:break
        if receipt and receipt.get('stage') in ['confirmed','refused','unknown','cancelled']:
            return {**result,'queued':False,'confirmed':receipt['stage']=='confirmed','refused':receipt['stage']=='refused','delivery':receipt['stage'],'receipt':receipt}
        if wait:time.sleep(.15)
    return {**result,'mode':'steer','confirmed':False,'delivery':'awaiting_claude_confirmation'}

def spec(name,description,properties,required=()):
    return {'name':name,'description':description,'inputSchema':{'type':'object','properties':properties,'required':list(required),'additionalProperties':False}}
STR={'type':'string'}
TOOLS=[
 spec('plan_followup','During a background observation, decide your own NEXT check: after_minutes 1..10080, or null to wait for the human/new evidence. publish=false suppresses this periodic report and its banner. Call before final text. Explain urgency/backoff in reason. This changes only your own follow-up clock, never worker tasks or user preferences.',{'after_minutes':{'type':['integer','null'],'minimum':1,'maximum':10080},'publish':{'type':'boolean'},'reason':STR},['after_minutes','publish','reason']),
 spec('get_checkin_settings','Read proactive progress-report preferences and delivery limits.',{}),
 spec('configure_checkins','Change proactive reporting only at the HUMAN request. goal_review_minutes controls mandatory goal progress reviews (default 60), independent of adaptive wait. idle_minutes is the initial inactivity delay (1..1440). interval_minutes is accepted for compatibility but does not control adaptive follow-ups. This never dispatches worker tasks.',{'enabled':{'type':'boolean'},'idle_minutes':{'type':'integer','minimum':1,'maximum':1440},'interval_minutes':{'type':'integer','minimum':1,'maximum':1440},'goal_review_minutes':{'type':'integer','minimum':1,'maximum':1440},'desktop_notifications':{'type':'boolean'},'user_intent_id':STR},['user_intent_id']),
 spec('list_sessions','List all automatically tracked local/remote sessions, connectivity, original status, existing machines and task ledger.',{}),
 spec('inspect_sessions','Read all worker sessions concurrently for an overview, including local source terminals and offline native history. Return evidence, connectivity and freshness separately; one failure never blocks the others.',{'addresses':{'type':'array','items':STR,'maxItems':24}}),
 spec('read_session','Read bounded evidence without focusing, scrolling or marking Done seen. Returns provenance, freshness, partial-history limits and offline cached evidence.',{'address':STR,'offset':{'type':'integer','minimum':0},'limit':{'type':'integer','minimum':1,'maximum':40}},['address']),
 spec('send_message','Send an authorized follow-up without touching drafts. Default mode=queue waits for current work; local Codex and capable native Claude support mode=steer to adjust the verified active turn. Native Claude uses ordinary interactive sendNow, preserves approval gates and returns confirmed only after its prompt hook acknowledges the exact text. Native read_session exposes per-request dispatches; never call transport queuing completed steering. Unsupported/changed runtimes are refused, never silently downgraded. Use a unique request_id per assignment; reuse that ID on retry. Local Codex continues its exact existing runtime thread via turn/start, not terminal typing. A dispatch error is never permission to create a replacement worker. Never use this to answer an approval.',{'address':STR,'text':STR,'request_id':STR,'mode':{'enum':['queue','steer']}},['address','text','request_id']),
 spec('close_session','Archive exactly one explicitly user-authorized session from Herdr and its managed list; retain history/files and underlying runtime. Call directly, never delegate shell/terminal commands to a worker. Requires the latest user_intent_id and a stable request_id. Only report closed after verified=true; pending is not completion.',{'address':STR,'request_id':STR,'user_intent_id':STR},['address','request_id','user_intent_id']),
 spec('release_session','Terminate and verify the exact user-authorized worker runtime, then report released only when runtime_stopped and verified are true. Archives the app entry first; preserves history/files. Can release previously archived workers. Never use for mere archive or in background turns. Requires latest explicit human authorization; never delegate to another worker.',{'address':STR,'request_id':STR,'user_intent_id':STR},['address','request_id','user_intent_id']),
 spec('create_session','Create a new local/remote worker without changing user focus. Claude accepts @local, hosts already used by sessions, and explicit aliases in the user SSH config (including Include files); no prior worker is required. Codex uses an existing core machine. Creation is idempotent by request_id; send the assignment separately.',{'host':STR,'folder':STR,'agent':{'enum':['claude','codex']},'title':STR,'request_id':STR},['folder','agent','title','request_id']),
 spec('reply_to_question','Forward the latest HUMAN reply verbatim to an exact live Claude or Codex worker question. Read pending_permissions/pending_questions first, then get latest_user_intent.id from list_sessions. Cannot approve tools.',{'address':STR,'question_id':STR,'user_intent_id':STR},['address','question_id','user_intent_id']),
 spec('schedule_reminder','Persist an explicitly requested reminder. at must be ISO-8601 with offset; timezone is an IANA name. Supports once, daily, weekly. This never runs commands, approves or deploys. Report the returned delivery limits.',{'request_id':STR,'title':STR,'text':STR,'at':STR,'timezone':STR,'repeat':{'enum':['none','daily','weekly']},'user_intent_id':STR},['request_id','title','text','at','timezone','user_intent_id']),
 spec('list_reminders','Get the current local date/time for relative phrases, plus persisted reminders, recurrence and unread deliveries.',{'timezone':STR}),
 spec('cancel_reminder','Cancel a reminder and dismiss its unread deliveries only when the human requests it.',{'id':STR,'user_intent_id':STR},['id','user_intent_id']),
 spec('acknowledge_reminder','Mark a delivered reminder read only after an explicit human response; reading it yourself is not acknowledgment.',{'delivery_id':STR,'user_intent_id':STR},['delivery_id','user_intent_id']),
 spec('list_decisions','Read persistent human decision cards. pending needs a human reply; answered is queued and still needs follow-through; resolved is human-marked complete.',{}),
 spec('record_decision','Only after judging that current HUMAN input is necessary: read list_decisions and fresh source evidence, then update an existing card or explicitly create one. Explain why existing authority is insufficient in reason. decision_key identifies the same decision across titles/snapshots, never use timestamps/status as its identity. source_ids are exact message/question IDs. Shared-source collisions require updating/merging the existing card, or distinct_from plus a reason explaining truly separate questions. No worker writes, acceptance or approvals.',{'id':STR,'title':STR,'context':STR,'address':STR,'decision_key':STR,'reason':STR,'source_ids':{'type':'array','items':STR,'minItems':1,'maxItems':12},'distinct_from':{'type':'array','items':STR,'maxItems':12},'options':{'type':'array','items':STR,'maxItems':6},'evidence':STR},['id','title','context','address','decision_key','reason','source_ids']),
 spec('curate_decision','After inspecting the board/evidence, merge genuinely duplicate pending cards into keeper id, or withdraw a pending card that is obsolete/not a human decision. Requires an explanatory reason; retains original records as archived, never marks work accepted/completed, never changes worker tasks. Cannot modify answered/completed cards.',{'action':{'enum':['merge','withdraw']},'id':STR,'duplicate_ids':{'type':'array','items':STR,'maxItems':12},'reason':STR},['action','id','reason']),
 spec('record_task','Ledger only: persist task authority/evidence; NO stage automatically creates a decision card. Separately judge whether a concrete human decision is needed before using record_decision. awaiting_review survives idle/disconnect; accepted requires explicit user acceptance.',{'id':STR,'title':STR,'address':STR,'mandate':STR,'stage':{'enum':['working','waiting_user','awaiting_review','accepted','failed','paused']},'summary':STR,'evidence':STR,'next_action':STR,'user_intent_id':STR},['id','title','address','mandate','stage','summary']),
]

TOOLS += [
 spec('list_goals','Read canonical goals, cited mandates, verified milestones, next steps and reconciled delivery receipts. Legacy tasks are historical snapshots, not new human decisions.',{}),
 spec('record_goal','Create/update a canonical goal and attach legacy task IDs without changing historical records. Only a current human turn can change mandate/plan/enabled. Keep a stable goal ID across sessions and titles. Plans contain exact existing workers, action, instruction, evidence dependencies and postcondition; background advance_goal executes these stored steps only.',{'id':STR,'expected_revision':{'type':'integer'},'title':STR,'outcome':STR,'summary':STR,'next_action':STR,'priority':{'type':'integer','minimum':1,'maximum':9},'task_ids':{'type':'array','items':STR},'enabled':{'type':'boolean'},'mandate':{'type':'object','properties':{'source_ids':{'type':'array','items':STR},'actions':{'type':'array','items':{'enum':['inspect','implement','test','open_pr','merge','deploy_stg','deploy_prd']}},'scope':STR,'exclusions':STR},'required':['source_ids','actions','scope','exclusions'],'additionalProperties':False},'plan':{'type':'array','items':{'type':'object','properties':{'id':STR,'address':STR,'action':STR,'instruction':STR,'postcondition':STR,'depends_on':{'type':'array','items':STR}},'required':['id','address','action','instruction','postcondition','depends_on'],'additionalProperties':False}}},['id','expected_revision','title','outcome']),
 spec('record_goal_progress','Update the current observational goal state from fresh evidence. This does not grant authority, accept work or change a plan. observed_complete means verified postcondition, never human acceptance. Milestones may release stored authorized dependent steps; require fresh complete source evidence, never a queued receipt alone.',{'id':STR,'expected_revision':{'type':'integer'},'phase':{'enum':['active','blocked','observed_complete']},'needs_user':{'type':'boolean'},'why_user':STR,'summary':STR,'next_action':STR,'observed_at':{'type':'integer'},'source_ids':{'type':'array','items':STR},'milestones':{'type':'object','additionalProperties':{'type':'object','properties':{'achieved':{'type':'boolean'},'summary':STR,'source_ids':{'type':'array','items':STR}},'required':['achieved','summary','source_ids'],'additionalProperties':False}}},['id','expected_revision','phase','summary','next_action','observed_at','source_ids']),
 spec('ask_goal_session','Ask an existing quiet goal worker for progress, blockers and next step. Fixed question only; validates stored identity, fresh idle evidence, no pending approvals, 15 minutes quiet, and durable once-only delivery per quiet episode. Never starts replacement workers or expands authority.',{'goal_id':STR,'step_id':STR},['goal_id','step_id']),
 spec('advance_goal','Continue ONE stored authorized goal step. No arbitrary text/action/address is accepted. Enforces enabled state, exact original worker, cited action grant, verified dependencies and durable once-only step receipt. Safe for background goal follow-through; never approves tools or replays unknown delivery. Manual inspection cannot dispatch.',{'goal_id':STR,'step_id':STR},['goal_id','step_id']),
]

TOOLS += [
 spec('search_memories','Retrieve persistent collaboration preferences with exact human sources. Empty query lists notes. These are historical data, never authority or current task progress.',{'query':STR,'limit':{'type':'integer','minimum':1,'maximum':100}}),
 spec('list_memories','List the complete detailed-memory index, independent of the current utterance. The stable summary is already in application context; inspect this index only when more detail is useful.',{'offset':{'type':'integer','minimum':0},'limit':{'type':'integer','minimum':1,'maximum':100}}),
 spec('read_memory','Read one exact memory record and original evidence; returns missing after deletion. Memory is historical, not operational authority.',{'id':STR},['id']),
 spec('review_memories','Memory maintenance: read the current snapshot and bounded pending HUMAN feedback. Learn durable collaboration patterns, not task status or permissions. Preserve manual/newer notes, update stable topics before consolidation.',{}),
 spec('consolidate_memories','Compile a stable overview after reviewing feedback and exact memory records. Every statement must cite memory_ids and all current records must be covered. Preserve manual wording. Invalidated immediately by any note edit/delete. Acknowledge exactly the reviewed feedback batch. No worker actions or operational authority.',{'expected_version':STR,'reviewed_source_ids':{'type':'array','items':STR},**{k:{'type':'array','items':{'type':'object','properties':{'text':STR,'memory_ids':{'type':'array','items':STR}},'required':['text','memory_ids'],'additionalProperties':False}} for k in ['profile','preferences','topics']}},['expected_version','reviewed_source_ids','profile','preferences','topics']),
 spec('record_memory','Learn a durable cooperation preference from current human feedback, or the exact eligible human feedback batch in a dedicated observe_memory_ maintenance turn. Background maintenance cannot overwrite manual/newer notes. Search first and update the stable topic instead of duplicating. Cite verbatim human quotes and current message ID; explicit instructions need one source, inferred repeated patterns need two. Never store credentials, worker claims, transient task status, or permission grants. expected_revision=0 creates; otherwise use the current revision.',{'topic':STR,'expected_revision':{'type':'integer'},'title':STR,'content':STR,'kind':{'enum':['preference','workflow','context']},'confidence':{'enum':['explicit','inferred']},'importance':{'type':'integer','minimum':1,'maximum':5},'sources':{'type':'array','items':{'type':'object','properties':{'id':STR,'quote':STR},'required':['id','quote'],'additionalProperties':False},'minItems':1,'maxItems':6}},['topic','expected_revision','title','content','kind','confidence','importance','sources']),
 spec('forget_memory','Delete a collaboration note after an explicit CURRENT human request naming its ID. Removes stored text and sources; original chat history remains. Older evidence cannot silently recreate the note.',{'id':STR,'expected_revision':{'type':'integer'},'user_intent_id':STR,'quote':STR},['id','expected_revision','user_intent_id','quote']),
]

def call(name,a):
    if name=='list_memories':return memory_store.list_notes(HOME,a)
    if name=='read_memory':return memory_store.read_note(HOME,a)
    if name=='review_memories':return memory_store.review(HOME,a)
    if name=='consolidate_memories':return memory_store.consolidate(HOME,a)
    if name=='search_memories':return memory_store.search(HOME,a)
    if name=='record_memory':return memory_store.save(HOME,a)
    if name=='forget_memory':return memory_store.forget(HOME,a)
    if name=='list_goals':return goal_store.reconcile(HOME,app('supervisor.list')['sessions'])
    if name=='record_goal':return goal_store.save(HOME,a,app('supervisor.list')['sessions'])
    if name=='record_goal_progress':return goal_store.progress(HOME,a)
    if name=='ask_goal_session':return goal_watch.ask(HOME,a,app('supervisor.list')['sessions'],lambda args:call('read_session',args),lambda args:call('send_message',args),lambda:app('supervisor.list')['sessions'])
    if name=='advance_goal':return goal_store.advance(HOME,a,app('supervisor.list')['sessions'],lambda args:call('send_message',args),refused=(Refused,))
    if name=='list_decisions':return app('supervisor.decisions')
    if name=='curate_decision':
        if redact(a)!=a:raise ValueError('Do not store credentials in curation notes')
        if a.get('action') not in ['merge','withdraw'] or not isinstance(a.get('reason'),str) or not a['reason'].strip():raise ValueError('Explain the observational curation action')
        return app('supervisor.decision_curate',a)
    if name=='record_decision':
        if not all(isinstance(a.get(k),str) and a[k].strip() for k in ['decision_key','reason']) or not isinstance(a.get('source_ids'),list) or not a['source_ids'] or not all(isinstance(x,str) and x.strip() for x in a['source_ids']):raise ValueError('First judge whether human input is needed; supply decision_key, reason and exact source_ids')
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
    if name=='release_session':return session_release.release(HOME,BIN,a,app,lambda args:call('close_session',args))
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
        if previous.get('decision_id'):a.setdefault('decision_id',previous['decision_id'])
        with c:c.execute('INSERT OR REPLACE INTO tasks VALUES (?,?)',(a['id'],json.dumps(a)))
        c.close()
        return a
    if name=='list_sessions':
        v=app('supervisor.list');c=db();v['tasks']=[{**json.loads(x[0]),'session_closed':not any(s['address']==json.loads(x[0]).get('address') for s in v['sessions']) and any(s['address']==json.loads(x[0]).get('address') for s in v.get('closed_sessions',[]))} for x in c.execute('SELECT value FROM tasks')];c.close()
        goals=goal_store.reconcile(HOME,v['sessions']);v['goals']=goals['goals']
        for task in v['tasks']:
            if task['id'] in goals['task_links']:task.update(goal_id=goals['task_links'][task['id']],historical_snapshot=True,not_a_human_gate=True)
        # Old task ledgers can contain superseded snapshots. The manager must
        # verify current questions before hanging them, rather than bulk-importing history.
        return session_release.decorate(HOME,v)
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
        if a.get('mode','queue') not in ['queue','steer']:raise Refused('Unsupported follow-up mode; nothing sent')
        if a.get('mode')=='steer' and a['address'].startswith('chat:') and not target.get('steer_available'):raise Refused('Native steering unavailable; update this runtime when idle; nothing sent')
    encoded=hashlib.sha256(json.dumps({'tool':name,'args':a},sort_keys=True).encode()).hexdigest()
    c=db()
    with c:
        old=c.execute('SELECT arguments,result FROM sends WHERE id=?',(ident,)).fetchone()
        if old:
            if old[0]!=encoded:raise ValueError('request_id reused for different action')
            if old[1] is None:raise ValueError('Delivery unknown. Read session; do not resend automatically.')
            result=json.loads(old[1])
            if name=='send_message' and result.get('queued') and result.get('source')=='codex_app_server':
                try:
                    result=codex_control('send',{**a,'read_identity':target.get('read_identity')})
                    c.execute('UPDATE sends SET result=? WHERE id=?',(json.dumps(result),ident))
                except (OSError,ValueError):pass  # Existing queue receipt remains valid; not a new send.
            if name=='send_message' and a['address'].startswith('chat:') and a.get('mode')=='steer' and not result.get('confirmed'):
                result=native_steer_receipt(a,result)
                c.execute('UPDATE sends SET result=? WHERE id=?',(json.dumps(result),ident))
            return result
        c.execute('INSERT INTO sends VALUES (?,?,NULL)',(ident,encoded))
    try:
        snap=app('supervisor.list')
        if name=='send_message' and any(x.get('super_agent') and x['address']==a['address'] for x in snap['sessions']):raise ValueError('Cannot delegate to yourself')
        if name=='create_session' and a['agent']=='claude':
            host=a.get('host') or '@local'
            if host!='@local' and not any(x.get('host')==host for x in snap['sessions']) and not configured_ssh_host(host):raise Refused('Unknown SSH host; add an explicit Host alias to ~/.ssh/config first. No worker created.')
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
                if a.get('mode')=='steer':raise Refused('This terminal adapter does not support steer; nothing sent')
                if target['status'] not in ['idle','done']:raise Refused('Terminal not idle; no message sent')
                if not a.get('text','').strip() or len(a['text'])>100000 or any(ord(ch)<32 and ch not in '\n\t' for ch in a['text']):raise Refused('Invalid message')
                r=subprocess.run([snap['core'],'send','--json','--if','idle',a['address'],a['text']],capture_output=True,text=True,timeout=15)
                if r.returncode:raise ValueError('Terminal send failed. Delivery is uncertain; inspect and repair this SAME session. Do not create a replacement worker')
                result={'core_receipt':json.loads(r.stdout),'request_id':ident}
        else:
            # Native FIFO is selected by the adapter, not an app command argument.
            # Passing tool-only mode through the CLI fails before any queue write.
            native_args={k:v for k,v in a.items() if k!='mode' or v=='steer'} if name=='send_message' else a
            result=app('supervisor.send' if name=='send_message' else 'supervisor.create',native_args)
            if name=='send_message':result=native_steer_receipt(a,result,wait=True)
            if name=='create_session':
                result['address']='chat:'+result['pane'] if result.get('pane') else result.get('terminal')
    except Refused:
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
