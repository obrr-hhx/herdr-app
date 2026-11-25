"""Goal ownership, cited mandates and bounded continuation; legacy tasks stay intact."""
import hashlib,json,pathlib,re,sqlite3,time,os
from contextlib import contextmanager
from redaction import redact

ACTIONS={'inspect','implement','test','open_pr','merge','deploy_stg','deploy_prd'}
PHASES={'active','blocked','observed_complete'}
def now():return int(time.time()*1000)
def root(home):return pathlib.Path(home)/'data/super-agent'
@contextmanager
def db(home):
    folder=root(home);folder.mkdir(parents=True,exist_ok=True,mode=0o700)
    c=sqlite3.connect(folder/'goals.sqlite',timeout=20);c.row_factory=sqlite3.Row
    try:
        c.execute('CREATE TABLE IF NOT EXISTS goals(id TEXT PRIMARY KEY,value TEXT NOT NULL)')
        c.execute('CREATE TABLE IF NOT EXISTS task_links(task_id TEXT PRIMARY KEY,goal_id TEXT NOT NULL)')
        c.execute('CREATE TABLE IF NOT EXISTS steps(id TEXT PRIMARY KEY,goal_id TEXT NOT NULL,step_id TEXT NOT NULL,digest TEXT NOT NULL,value TEXT NOT NULL)')
        c.execute('CREATE TABLE IF NOT EXISTS assignments(id TEXT PRIMARY KEY,value TEXT NOT NULL)')
        c.execute('CREATE TABLE IF NOT EXISTS audit(seq INTEGER PRIMARY KEY,value TEXT NOT NULL)')
        os.chmod(folder/'goals.sqlite',0o600)
        with c:yield c
    finally:c.close()
def ident(value):
    if not isinstance(value,str) or not re.fullmatch(r'[a-zA-Z0-9_-]{1,100}',value):raise ValueError('Use a stable goal/step ID')
    return value
def text(value,limit=12000):
    if not isinstance(value,str) or not value.strip() or len(value)>limit or redact(value)!=value:raise ValueError('Nonempty, credential-free goal text required')
    return value

def human_sources(home,ids):
    p=pathlib.Path(home)/'data/forge-chat/73757065726167656e74000000000001/events.jsonl'
    events=[json.loads(l) for l in p.read_text().splitlines()]
    found={e['id']:e for e in events if e.get('type')=='user' and not e.get('id','').startswith('observe_')}
    if not ids or any(i not in found for i in ids):raise ValueError('Mandate requires actual human message IDs, not worker claims')
    return [{'id':i,'text':text(found[i]['text']), 'time':found[i]['time']} for i in ids]
def active_request(home):
    try:return json.loads((root(home)/'active-observation.json').read_text()).get('id','')
    except (OSError,ValueError):return ''
def interactive(home):
    request=active_request(home)
    if request.startswith(('observe_','manual_review_')):return False
    # Ordinary turns must also have a real HUMAN event; missing context fails closed.
    try:return any(json.loads(l).get('type')=='user' and json.loads(l).get('id')==request for l in (pathlib.Path(home)/'data/forge-chat/73757065726167656e74000000000001/events.jsonl').read_text().splitlines())
    except (OSError,ValueError):return False

def _snapshot(c):
    goals=[g for r in c.execute('SELECT value FROM goals') if not (g:=json.loads(r['value'])).get('deleted_at')]
    receipts=[json.loads(r['value']) for r in c.execute('SELECT value FROM steps')]
    links={r['task_id']:r['goal_id'] for r in c.execute('SELECT * FROM task_links')}
    goals.sort(key=lambda g:(g.get('priority',5),g['id']))
    for g in goals:g['steps_receipts']=[r for r in receipts if r['goal_id']==g['id']]
    return {'goals':goals,'assignments':[json.loads(r['value']) for r in c.execute('SELECT value FROM assignments')],'task_links':links,'at':now()}
def snapshot(home):
    with db(home) as c:
        c.execute('BEGIN');return _snapshot(c)
def publish(home):
    import tempfile
    # Serialize the projection with DB writers so an older snapshot cannot
    # replace newer progress when separate management tools finish concurrently.
    with db(home) as c:
        c.execute('BEGIN IMMEDIATE');value=_snapshot(c);p=root(home)/'goals.json'
        with tempfile.NamedTemporaryFile(mode='w',dir=p.parent,delete=False) as f:
            json.dump(value,f,ensure_ascii=False);f.flush();os.fsync(f.fileno());tmp=f.name
        os.replace(tmp,p);os.chmod(p,0o600)
    return value

def save(home,args,sessions,admin=False):
    a=json.loads(json.dumps(args));
    if redact(a)!=a:raise ValueError('Do not store credentials in goals')
    gid=ident(a['id']);text(a['title'],300);text(a['outcome'])
    with db(home) as c:
        c.execute('BEGIN IMMEDIATE');row=c.execute('SELECT value FROM goals WHERE id=?',(gid,)).fetchone();old=json.loads(row['value']) if row else {}
        if old.get('deleted_at'):raise ValueError('Goal was deleted; create a distinct goal instead')
        if a.get('expected_revision',0)!=old.get('revision',0):raise ValueError('Goal changed; reread list_goals before updating')
        if old and old.get('enabled') and a['outcome']!=old['outcome'] and not admin and not interactive(home):raise ValueError('Background turns cannot rewrite the authorized outcome')
        mandate=a.get('mandate',old.get('mandate',{}));plan=a.get('plan',old.get('plan',[]));enabled=a.get('enabled',old.get('enabled',False))
        changed=mandate!=old.get('mandate',{}) or plan!=old.get('plan',[]) or enabled!=old.get('enabled',False)
        if changed:
            if not admin and not interactive(home):raise ValueError('Background turns cannot grant/change mandates, plans or pause state')
            if not admin and active_request(home) not in mandate.get('source_ids',[]):raise ValueError('Changed mandate/plan must cite the current human turn as well as any earlier grants')
            sources=human_sources(home,mandate.get('source_ids',[]))
            if not set(mandate.get('actions',[]))<=ACTIONS:raise ValueError('Unsupported mandate action; tool approvals are never delegated')
            text(mandate.get('scope',''))
            exclusions=mandate.get('exclusions','')
            if not isinstance(exclusions,str):raise ValueError('Goal exclusions must be text')
            if exclusions:text(exclusions)
            mandate={**mandate,'human_sources':sources}
            # Canonicalize stored evidence before comparing future contract edits.
            for step in plan:
                ident(step['id']);text(step['instruction']);text(step['postcondition'])
                if step.get('action') not in mandate.get('actions',[]):raise ValueError('Step action is outside the cited mandate')
                target=next((r for r in sessions if r['address']==step.get('address') and not r.get('super_agent')),None)
                if not target:raise ValueError('Use an exact managed worker; no replacement dispatch')
                binding={k:target.get(k) for k in ['conversation','host','read_identity']}
                if not binding.get('conversation') and not binding.get('read_identity'):raise ValueError('Worker identity unavailable')
                step['binding']=binding
                if not isinstance(step.get('depends_on',[]),list):raise ValueError('Milestone dependencies must be a list')
            if len({s['id'] for s in plan})!=len(plan):raise ValueError('Step IDs must be unique')
            # Never rewrite a step that has been queued/submitted, even after a new grant.
            for r in c.execute('SELECT step_id FROM steps WHERE goal_id=?',(gid,)):
                if next((s for s in plan if s['id']==r['step_id']),None)!=next((s for s in old.get('plan',[]) if s['id']==r['step_id']),None):raise ValueError('Submitted steps are immutable; add a distinct follow-up step')
        goal={**old,**{k:a[k] for k in ['id','title','outcome','summary','next_action','priority'] if k in a},'mandate':mandate,'plan':plan,'enabled':enabled,'revision':old.get('revision',0)+1,'phase':old.get('phase','active'),'milestones':old.get('milestones',{}),'updated_at':now(),'human_accepted':old.get('human_accepted',False)}
        c.execute('INSERT OR REPLACE INTO goals VALUES (?,?)',(gid,json.dumps(goal)))
        for task in a.get('task_ids',[]):
            previous=c.execute('SELECT goal_id FROM task_links WHERE task_id=?',(task,)).fetchone()
            if previous and previous['goal_id']!=gid:raise ValueError('Legacy task already belongs to a different goal')
            c.execute('INSERT OR IGNORE INTO task_links VALUES (?,?)',(task,gid))
        c.execute('INSERT INTO audit(value) VALUES (?)',(json.dumps({'kind':'goal_saved','goal_id':gid,'revision':goal['revision'],'contract_changed':changed,'at':now()}),))
    publish(home);return goal

def progress(home,a):
    if a.get('needs_user') and not a.get('why_user'):raise ValueError('Explain the concrete decision existing authority cannot resolve')
    if redact(a)!=a:raise ValueError('Do not store credentials in goal evidence')
    if a.get('phase') not in PHASES:raise ValueError('Use observational active/blocked/observed_complete; not human acceptance')
    text(a['summary']);text(a['next_action']);proof=a.get('source_ids',[])
    if not proof or not all(isinstance(s,str) and s for s in proof):raise ValueError('Current source IDs required')
    observed=a.get('observed_at',0)
    if not isinstance(observed,int) or observed>now()+1000 or observed<=0:raise ValueError('Invalid observation time')
    with db(home) as c:
        c.execute('BEGIN IMMEDIATE');r=c.execute('SELECT value FROM goals WHERE id=?',(a['id'],)).fetchone()
        if not r:raise ValueError('Unknown goal')
        g=json.loads(r['value'])
        if g.get('deleted_at'):raise ValueError('Goal was deleted')
        if g['revision']!=a['expected_revision']:raise ValueError('Goal changed; reread before updating')
        if observed<g.get('observed_at',0):raise ValueError('Older evidence cannot replace the current goal state')
        milestones=dict(g.get('milestones',{}))
        for key,value in a.get('milestones',{}).items():
            ident(key)
            if not isinstance(value.get('achieved'),bool) or not value.get('source_ids') or not value.get('summary'):raise ValueError('Verified milestones need actual evidence IDs and summary')
            milestones[key]={**value,'observed_at':observed}
        # Accepted work is distinct from an objective whose postcondition was observed.
        g.update(needs_user=bool(a.get('needs_user',False)),why_user=a.get('why_user',''),phase=a['phase'],summary=a['summary'],next_action=a['next_action'],source_ids=proof,observed_at=observed,milestones=milestones,revision=g['revision']+1,updated_at=now())
        c.execute('UPDATE goals SET value=? WHERE id=?',(json.dumps(g),g['id']))
        c.execute('INSERT INTO audit(value) VALUES (?)',(json.dumps({'kind':'progress','goal_id':g['id'],'source_ids':proof,'phase':g['phase'],'at':now()}),))
    publish(home);return g

def assignment_text(home,goal,step):
    # Verified human wording is the task, not the manager's interpretation of permission.
    sources=goal['mandate'].get('human_sources') or human_sources(home,goal['mandate']['source_ids'])
    quotes='\n\n'.join('\n'.join('> '+line for line in source['text'].split('\n')) for source in sorted(sources,key=lambda s:s['time']))
    return f"[目标 {goal['id']} / 步骤 {step['id']}]\n用户原话（按时间顺序，后续更正优先）：\n{quotes}\n\n管家补充（仅作上下文，不增加或缩减用户要求）：\n{step['instruction']}\n\n交付结果：{step['postcondition']}"

def advance(home,a,sessions,send,refused=()):
    if active_request(home).startswith('manual_review_'):raise ValueError('Manual inspection does not dispatch workers')
    gid=ident(a['goal_id']);sid=ident(a['step_id']);request='goal_'+hashlib.sha256((gid+':'+sid).encode()).hexdigest()[:48]
    with db(home) as c:
        c.execute('BEGIN IMMEDIATE');row=c.execute('SELECT value FROM goals WHERE id=?',(gid,)).fetchone()
        if not row:raise ValueError('Unknown goal')
        g=json.loads(row['value']);step=next((s for s in g.get('plan',[]) if s['id']==sid),None)
        if g.get('deleted_at') or not step or not g.get('enabled') or g.get('needs_user') or g.get('phase')=='observed_complete':raise ValueError('Goal/step is not enabled for continuation')
        if step.get('after_step_id') and not any(json.loads(r['value']).get('result_state')=='result_available' for r in c.execute('SELECT value FROM steps WHERE goal_id=? AND step_id=?',(gid,step['after_step_id']))):raise ValueError('Predecessor result unavailable')
        if step['action'] not in g.get('mandate',{}).get('actions',[]):raise ValueError('No mandate for this action')
        if any(not g.get('milestones',{}).get(dep,{}).get('achieved') for dep in step.get('depends_on',[])):raise ValueError('Wait for verified prerequisite milestones; receipt is not completion')
        target=next((r for r in sessions if r['address']==step['address'] and not r.get('super_agent')),None)
        if not target or {k:target.get(k) for k in ['conversation','host','read_identity']}!=step['binding']:raise ValueError('Original worker missing or identity changed; no dispatch')
        digest=hashlib.sha256(json.dumps(step,sort_keys=True).encode()).hexdigest();old=c.execute('SELECT * FROM steps WHERE id=?',(request,)).fetchone()
        if old:
            if old['digest']!=digest:raise ValueError('Step changed after dispatch')
            previous=json.loads(old['value'])
            if previous.get('delivery')!='refused' or previous.get('retry_after',0)>now():return previous
            c.execute('DELETE FROM steps WHERE id=?',(request,))
        instruction=assignment_text(home,g,step)
        record={'request_id':request,'goal_id':gid,'step_id':sid,'address':step['address'],'action':step['action'],'delivery':'submitting','confirmed':False,'at':now(),'mandate_revision':g['revision'],'binding':step['binding']}
        c.execute('INSERT INTO steps VALUES (?,?,?,?,?)',(request,gid,sid,digest,json.dumps(record)))
    # Durable claim precedes any write; recovery reconciles receipts rather than replays.
    try:
        receipt=send({'address':step['address'],'text':instruction,'request_id':request,'mode':'queue'})
        record.update(receipt=receipt,delivery='confirmed' if receipt.get('confirmed') else 'queued',confirmed=bool(receipt.get('confirmed')))
    except Exception as error:
        # A refusal permits explicit retry only after reconciliation; no timer blind-retry.
        record.update(delivery='refused' if isinstance(error,refused) else 'unknown',retry_after=now()+60000,error=redact(str(error))[:400])
    with db(home) as c:c.execute('UPDATE steps SET value=? WHERE id=?',(json.dumps(record),request))
    publish(home);return record

def reconcile(home,sessions):
    """Track queue, receipt and actual result independently; never replay unknown input."""
    with db(home) as c:
        native_events={};codex_facts=None
        for row in c.execute("SELECT id,value,'steps' AS source FROM steps UNION ALL SELECT id,value,'assignments' AS source FROM assignments").fetchall():
            record=json.loads(row['value']);target=next((s for s in sessions if s['address']==record['address']),None)
            if not target:continue
            binding=record.get('binding')
            if not binding and record.get('goal_id'):
                saved=c.execute('SELECT value FROM goals WHERE id=?',(record['goal_id'],)).fetchone()
                binding=next((v.get('binding') for v in json.loads(saved['value']).get('plan',[]) if v['id']==record.get('step_id')),None) if saved else None
            if not binding:continue
            if binding and {k:target.get(k) for k in ['conversation','host','read_identity']}!=binding:continue
            updated=dict(record)
            try:
                if target.get('conversation'):
                    conversation=target['conversation']
                    if conversation not in native_events:
                        native_events[conversation]=None
                        native_events[conversation]=[json.loads(l) for l in (pathlib.Path(home)/'data/forge-chat'/conversation/'events.jsonl').read_text().splitlines()]
                    events=native_events[conversation]
                    if events is None:continue
                    index=next((i for i,e in enumerate(events) if e.get('type')=='prompt_confirmed' and e.get('id')==row['id']),None)
                    if index is not None:
                        updated.update(delivery='confirmed',confirmed=True,receipt_evidence=events[index]);updated.setdefault('result_state','running')
                        segment=[]
                        for e in events[index+1:]:
                            if e.get('type')=='prompt_confirmed' and e.get('id')!=row['id']:break
                            if e.get('type')=='state' and e.get('status')=='working' and e.get('active_turn') not in [None,row['id']]:break
                            segment.append(e)
                        results=[e['id'] for e in segment if e.get('type')=='message_complete' and e.get('id')]
                        stopped=any(e.get('type')=='hook' and e.get('event',{}).get('hook_event_name')=='Stop' or e.get('type')=='state' and e.get('status')=='idle' for e in segment)
                        if results and stopped:updated.update(result_state='result_available',result_ids=results)
                        elif any(e.get('type')=='state' and e.get('status') in ['error','interrupted'] for e in segment):updated['result_state']='interrupted'
                else:
                    if codex_facts is None:
                        codex_facts={}
                        codex_facts=json.loads((root(home)/'codex-workers.json').read_text())['sessions']
                    facts=codex_facts.get(record['address'],{})
                    receipt=next((d for d in facts.get('dispatches',[]) if d.get('request_id')==row['id'] and d.get('confirmed')),None)
                    if receipt:
                        updated.update(delivery='confirmed',confirmed=True,receipt_evidence=receipt);updated.setdefault('result_state','running')
                        if receipt.get('turn_id') and facts.get('last_completed_turn')==receipt['turn_id']:
                            results=[m['id'] for m in facts.get('messages',[]) if m.get('turn_id')==receipt['turn_id'] and m.get('role')=='Assistant']
                            if results:updated.update(result_state='result_available',result_ids=results)
            except (OSError,ValueError,KeyError):continue
            if updated!=record:c.execute('UPDATE '+row['source']+' SET value=? WHERE id=?',(json.dumps(updated),row['id']))
    return publish(home)

def track_assignment(home,args,target,result):
    if not target:return
    record={'request_id':args['request_id'],'address':args['address'],'binding':{k:target.get(k) for k in ['conversation','host','read_identity']},'at':now(),'delivery':result.get('delivery') or ('confirmed' if result.get('confirmed') else 'queued'),'confirmed':bool(result.get('confirmed')),'receipt':result}
    with db(home) as c:
        old=c.execute('SELECT value FROM assignments WHERE id=?',(args['request_id'],)).fetchone()
        if old:
            previous=json.loads(old['value'])
            if previous['binding']!=record['binding']:return
            record={**previous,**record,'at':previous['at']}
        c.execute('INSERT OR REPLACE INTO assignments VALUES (?,?)',(args['request_id'],json.dumps(record)))

def assignments(home):
    with db(home) as c:return [json.loads(r['value']) for r in c.execute('SELECT value FROM assignments')]

def plan_followup(home,a,sessions):
    """Append only to an existing grant, existing worker and verified predecessor."""
    if active_request(home).startswith(('manual_review_','observe_memory_')):raise ValueError('Observation is read-only')
    with db(home) as c:
        c.execute('BEGIN IMMEDIATE');row=c.execute('SELECT value FROM goals WHERE id=?',(ident(a['goal_id']),)).fetchone()
        if not row:raise ValueError('Unknown goal')
        g=json.loads(row['value'])
        if g.get('deleted_at') or not g.get('enabled') or g.get('needs_user') or g.get('phase')=='observed_complete':raise ValueError('Goal is not enabled for continuation')
        if g['revision']!=a['expected_revision']:raise ValueError('Goal changed; reread')
        prior=next((s for s in g['plan'] if s['id']==a['after_step_id']),None)
        receipts=[json.loads(r['value']) for r in c.execute('SELECT value FROM steps WHERE goal_id=? AND step_id=?',(g['id'],a['after_step_id']))]
        if not prior or not any(r.get('result_state')=='result_available' for r in receipts):raise ValueError('Read actual predecessor result before planning follow-up')
        if a['action'] not in g['mandate']['actions']:raise ValueError('Outside existing mandate')
        target=next((s for s in sessions if s['address']==prior['address']),None)
        if not target or {k:target.get(k) for k in ['conversation','host','read_identity']}!=prior['binding']:raise ValueError('Original worker identity changed')
        sid=ident(a['step_id']);text(a['instruction']);text(a['postcondition'])
        step={k:a[k] for k in ['action','instruction','postcondition']};step.update(id=sid,address=prior['address'],binding=prior['binding'],depends_on=[],after_step_id=prior['id'])
        previous=next((s for s in g['plan'] if s['id']==sid),None)
        if previous:
            if previous!=step:raise ValueError('Step ID already used')
            return g
        if any(s.get('after_step_id')==prior['id'] for s in g['plan']):raise ValueError('Predecessor already has a follow-up; inspect it first')
        if len(g['plan'])>=100:raise ValueError('Plan limit reached; needs human review')
        g['plan'].append(step);g['revision']+=1;g['updated_at']=now()
        c.execute('UPDATE goals SET value=? WHERE id=?',(json.dumps(g),g['id']))
        c.execute('INSERT INTO audit(value) VALUES (?)',(json.dumps({'kind':'authorized_followup','goal_id':g['id'],'step_id':sid,'at':now()}),))
    publish(home);return g


def delete_manual(home,goal_id,expected_revision):
    """Human UI deletion stops future tracking; retain audit and sent receipts."""
    with db(home) as c:
        c.execute('BEGIN IMMEDIATE')
        row=c.execute('SELECT value FROM goals WHERE id=?',(ident(goal_id),)).fetchone()
        if not row:raise ValueError('目标不存在')
        goal=json.loads(row['value'])
        if goal.get('deleted_at'):return _snapshot(c)
        if goal['revision']!=expected_revision:raise ValueError('目标已更新，请查看最新内容后再删除')
        goal.update(deleted_at=now(),enabled=False,revision=goal['revision']+1)
        c.execute('UPDATE goals SET value=? WHERE id=?',(json.dumps(goal),goal_id))
        c.execute('INSERT INTO audit(value) VALUES (?)',(json.dumps({'kind':'goal_deleted','goal_id':goal_id,'at':now()}),))
    return publish(home)

if __name__=='__main__':
    import sys
    try:
        if len(sys.argv)!=3 or sys.argv[1]!='--delete-manual':raise ValueError('Unsupported operation')
        args=json.load(sys.stdin)
        print(json.dumps(delete_manual(sys.argv[2],args['id'],args['expected_revision']),ensure_ascii=False))
    except Exception as error:
        print(json.dumps({'error':str(error)},ensure_ascii=False));sys.exit(1)
