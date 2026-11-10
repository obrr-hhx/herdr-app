"""Bounded progress questions to exact quiet goal workers, never replacement tasks."""
import json,pathlib,sqlite3,hashlib,time,re,os
import goal_store as goals

def ask(home,args,sessions,read,send,refresh):
    if goals.active_request(home).startswith(('manual_review_','observe_memory_')):raise ValueError('This observation cannot contact workers')
    goal=next((g for g in goals.snapshot(home)['goals'] if g['id']==args['goal_id']),None)
    if not goal or not goal.get('enabled') or goal.get('needs_user') or goal.get('phase')=='observed_complete':raise ValueError('Goal is paused, complete, or waiting for the human')
    step=next((s for s in goal.get('plan',[]) if s['id']==args['step_id']),None)
    if not step:raise ValueError('Use an existing goal worker binding')
    target=next((s for s in sessions if s['address']==step['address'] and not s.get('super_agent')),None)
    binding={k:target.get(k) for k in ['conversation','host','read_identity']} if target else None
    if not target or binding!=step['binding']:raise ValueError('Original worker missing or identity changed')
    if target.get('attention') or (target.get('execution_state') or target.get('status')) not in ['idle','done'] or re.search('reconnect|disconnect|connecting|error|interrupted',target.get('detail',''),re.I):raise ValueError('Do not nudge active, blocked, disconnected or unverified workers')
    now=int(time.time()*1000);watch=json.loads((goals.root(home)/'goal-session-watch.json').read_text())
    entry=watch.get('workers',{}).get(step['address'])
    if not entry or not 0<=now-watch['at']<=30000 or entry['binding']!=binding or entry['completion']!=target.get('completion',0) or now-entry['since']<15*60000:raise ValueError('No fresh evidence of a quiet worker for at least 15 minutes')
    evidence=read({'address':step['address'],'limit':12})
    if not evidence.get('available') or evidence.get('cached') or evidence.get('last_known_only') or evidence.get('pending_permissions') or evidence.get('pending_questions'):raise ValueError('Fresh readable evidence required; do not bypass a pending question or approval')
    current=next((r for r in refresh() if r['address']==step['address']),None)
    if not current or {k:current.get(k) for k in ['conversation','host','read_identity']}!=binding or current.get('completion',0)!=entry['completion'] or current.get('attention') or (current.get('execution_state') or current.get('status')) not in ['idle','done'] or re.search('reconnect|disconnect|connecting|error|interrupted',current.get('detail',''),re.I):raise ValueError('Worker changed while reading; no progress question sent')
    key=hashlib.sha256(json.dumps([step['address'],binding,entry['since']],sort_keys=True).encode()).hexdigest()
    request='goal_question_'+key[:40];p=goals.root(home)/'goal-questions.sqlite'
    with sqlite3.connect(p,timeout=10) as c:
        c.execute('CREATE TABLE IF NOT EXISTS questions(id TEXT PRIMARY KEY,value TEXT NOT NULL)');os.chmod(p,0o600)
        c.execute('BEGIN IMMEDIATE');old=c.execute('SELECT value FROM questions WHERE id=?',(request,)).fetchone()
        if old:return json.loads(old[0])
        record={'request_id':request,'goal_id':goal['id'],'address':step['address'],'delivery':'unknown','confirmed':False,'at':now}
        c.execute('INSERT INTO questions VALUES (?,?)',(request,json.dumps(record)))
    text=f"[目标进度询问 {goal['id']}] 目标：{goal['outcome']}。这个会话已有一段时间没有新的进展信号。请简短说明：已完成什么、当前阻塞、下一步及预计何时能有结果。若已完成请给出证据；若缺少授权或需要用户决定请明确问题。继续原有授权内的工作即可，不新增任务范围、不重复执行或发布、不替用户审批。"
    try:
        result=send({'address':step['address'],'request_id':request,'text':text,'mode':'queue'})
        record.update(delivery='confirmed' if result.get('confirmed') else 'queued',confirmed=bool(result.get('confirmed')))
    except Exception:
        record.update(delivery='unknown')
    with sqlite3.connect(p,timeout=10) as c:c.execute('UPDATE questions SET value=? WHERE id=?',(json.dumps(record),request))
    return record
