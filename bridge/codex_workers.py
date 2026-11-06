"""Observe existing Codex runtimes and resolve their live requests, never their TUI."""
import hashlib
import json
import os
import pathlib
import re
import signal
import socket
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
from codex_rpc import CodexRPC
from redaction import redact


class DispatchRefused(ValueError):pass

def now():return int(time.time()*1000)
def socket_path():return pathlib.Path(os.environ.get('HERDR_CODEX_WORKER_SOCKET',str(pathlib.Path(__file__).with_name('codex-workers.sock'))))
def facts_path(home):return pathlib.Path(home)/'data/super-agent/codex-workers.json'
def identity(row):return row.get('read_identity')


def control(action,args):
    with socket.socket(socket.AF_UNIX) as s:
        s.settimeout(25 if action=='send' else 12 if action=='answer' else 3);s.connect(str(socket_path()))
        s.sendall((json.dumps({'action':action,**args},ensure_ascii=False)+'\n').encode())
        data=b''
        while b'\n' not in data:
            block=s.recv(65536)
            if not block or len(data)>2*1024*1024:raise ValueError('Codex bridge reply unavailable')
            data+=block
    r=json.loads(data.split(b'\n',1)[0])
    if 'error' in r:
        if r.get('kind')=='refused':raise DispatchRefused(r['error'])
        raise ValueError(r['error'])
    return r


def command(args):
    r=subprocess.run(args,capture_output=True,text=True,timeout=3)
    if r.returncode:raise ValueError('Source metadata unavailable')
    return json.loads(r.stdout)


def clean_title(value):
    value=re.sub(r'^[^\w]+','',value or '').strip()
    value=re.sub(r'^(?:Action Required|Needs input|Working|Done)\s*\|\s*','',value,flags=re.I)
    return value


def source_view(row,core):
    source=row.get('read_source') or {}
    if source.get('kind')=='codex_thread':
        return {'thread_id':source['thread_id'],'proof':source.get('thread_id'),'pid':None,'name':row.get('title',''),'cwd':row.get('folder')}
    if source.get('kind')=='herdr_attach':
        base=[source['binary'],'--session',source['session']]
        pane=command(base+['pane','get',source['pane']])['result']['pane']
        info=command(base+['pane','process-info','--pane',source['pane']])['result']['process_info']
        processes=[p for p in info.get('foreground_processes',[]) if p.get('name')=='codex' or pathlib.Path((p.get('argv') or [''])[0]).name=='codex']
        if len(processes)!=1:raise ValueError('Source is not a unique live Codex process')
        process=processes[0];argv=process.get('argv') or []
        explicit=pane.get('agent_session') or {}
        thread=explicit.get('session_id') or explicit.get('id')
        proof={'pane':pane['pane_id'],'terminal_id':pane['terminal_id'],'pid':process['pid']}
        return {'thread_id':thread,'argv':argv,'proof':proof,'pid':process['pid'],'name':clean_title(pane.get('terminal_title_stripped')),'cwd':pane.get('foreground_cwd') or pane.get('cwd')}
    record=command([core,'get','--json',row['address']])
    if identity(row) and identity(row)!={'pid':record.get('pid'),'created_ms':record.get('created_ms')}:
        raise ValueError('Source terminal identity changed')
    process=record.get('foreground') or {};argv=process.get('argv') or []
    if process.get('name')!='codex' and (not argv or pathlib.Path(argv[0]).name!='codex'):
        raise ValueError('Source is not Codex')
    return {'argv':argv,'proof':{'pid':process.get('pid'),'created_ms':record.get('created_ms')},'pid':process.get('pid'),'name':clean_title(record.get('title')),'cwd':process.get('cwd') or record.get('cwd')}


def bind(view,threads):
    explicit=view.get('thread_id')
    argv=view.get('argv',[])
    if not explicit and 'resume' in argv:
        i=argv.index('resume')
        if len(argv)>i+1 and re.fullmatch(r'[0-9a-f-]{36}',argv[i+1]):explicit=argv[i+1]
    if explicit:
        if explicit not in threads:raise ValueError('The exact source thread is not loaded on this Codex runtime')
        return explicit,'explicit_session_identity'
    # Older TUI clients did not publish the session ID to Herdr. Bootstrap only
    # from one exact LIVE runtime name + cwd, never newest files or fuzzy titles.
    candidates=[t['id'] for t in threads.values() if t.get('name') and (view['name']==t['name'] or view['name'].startswith(t['name']+' | ')) and os.path.realpath(view.get('cwd') or '/')==os.path.realpath(t.get('cwd') or '/')]
    if len(candidates)!=1:raise ValueError('Codex source binding is ambiguous or unavailable; no decision can be sent')
    return candidates[0],'unique_live_name_and_cwd'


def history_item(entry):
    item=entry.get('item',entry)
    kind=item.get('type');key=item.get('id')
    if not key:return None
    if kind=='userMessage':
        text='\n'.join(v.get('text','') for v in item.get('content',[]) if v.get('type')=='text');role='You'
    elif kind=='agentMessage':text=item.get('text','');role='Assistant'
    elif kind=='commandExecution':
        text=json.dumps({k:item.get(k) for k in ['command','status','exitCode','aggregatedOutput']},ensure_ascii=False);role='Command'
    elif kind=='mcpToolCall':
        text=json.dumps({k:item.get(k) for k in ['server','tool','arguments','status','result','error']},ensure_ascii=False);role='Tool'
    elif kind=='fileChange':text=json.dumps(item.get('changes',[]),ensure_ascii=False);role='File changes'
    else:return None  # Never expose internal reasoning as transcript evidence.
    return {'id':key,'role':role,'text':redact(text)[-12000:],'turn_id':entry.get('turnId'),'at':entry.get('completedAtMs') or entry.get('startedAtMs'),**({'command':redact(item.get('command','')),'exit_code':item.get('exitCode'),'status':item.get('status')} if kind=='commandExecution' else {})}


def question_record(message,first_seen):
    method=message['method'];p=message.get('params',{})
    if method not in ['item/tool/requestUserInput','tool/requestUserInput','item/commandExecution/requestApproval','item/fileChange/requestApproval']:
        return None
    scope={k:p.get(k) for k in ['threadId','turnId','itemId']};scope['method']=method
    ident='codexq_'+hashlib.sha256(json.dumps(scope,sort_keys=True).encode()).hexdigest()[:40]
    pure='requestUserInput' in method
    questions=p.get('questions',[]) if pure else []
    supported=bool(p.get('itemId')) and not any(q.get('isSecret') for q in questions) and (not pure or bool(questions)) and len({q.get('question') for q in questions})==len(questions)
    inputs={'questions':[{**q,'question':q.get('question',''),'header':q.get('header') or q.get('id',''),'options':q.get('options') or []} for q in questions]} if pure else {k:p.get(k) for k in ['command','cwd','reason','changes','availableDecisions']}
    return {'id':ident,'rpc_id':message['id'],'thread_id':p.get('threadId'),'turn_id':p.get('turnId'),'item_id':p.get('itemId'),'method':method,'time':first_seen.get(ident,now()),'name':'AskUserQuestion' if pure else 'Codex command approval' if 'commandExecution' in method else 'Codex file-change approval','input':inputs,'reply_supported':supported,'questions':questions,'requires_explicit_ui':False}


class Bridge:
    def __init__(self,home,endpoint):
        self.home=pathlib.Path(home);self.endpoint=endpoint;self.client=None;self.stopped=False;self.streaming=False
        self.send_lock=threading.Lock();self.lock=threading.RLock();self.pending={};self.resolved=set();self.rows={};self.threads={};self.bound={};self.subscribed=set();self.history={};self.live_items={};self.completion={};self.last_completed={};self.first_seen={}
        try:
            old=json.loads(facts_path(home).read_text())
            for address,row in old.get('sessions',{}).items():
                self.completion[address]=row.get('completion',0);self.last_completed[address]=row.get('last_completed_turn')
                for q in row.get('pending_questions',{}).values():self.first_seen[q['id']]=q['time']
        except (OSError,ValueError,KeyError):pass
        self.receipts=self.home/'data/super-agent/codex-decisions.sqlite'
        self.receipts.parent.mkdir(parents=True,exist_ok=True,mode=0o700)
        with sqlite3.connect(self.receipts) as c:
            c.execute('CREATE TABLE IF NOT EXISTS answers(id TEXT PRIMARY KEY,digest TEXT NOT NULL,result TEXT)')
            c.execute('CREATE TABLE IF NOT EXISTS claims(question_id TEXT PRIMARY KEY,receipt_id TEXT NOT NULL)')
            c.execute('CREATE TABLE IF NOT EXISTS dispatches(id TEXT PRIMARY KEY,digest TEXT NOT NULL,thread_id TEXT NOT NULL,result TEXT)')
        os.chmod(self.receipts,0o600)

    def message(self,m):
        p=m.get('params',{});method=m.get('method','')
        with self.lock:
            if method=='serverRequest/resolved':
                for ident,q in list(self.pending.items()):
                    if str(q['rpc_id'])==str(p.get('requestId')) and q['thread_id']==p.get('threadId'):
                        self.pending.pop(ident);self.resolved.add(ident)
            elif method=='thread/status/changed' and p.get('threadId') in self.threads:
                self.threads[p['threadId']]['status']=p['status']
            elif method in ['item/started','item/completed'] and p.get('threadId') in self.threads:
                item=p.get('item',{});self.live_items[(p['threadId'],item.get('id'))]=item
            elif m.get('id') is not None and p.get('threadId') in {b['thread_id'] for b in self.bound.values()}:
                q=question_record(m,self.first_seen)
                if q:
                    item=self.live_items.get((q['thread_id'],q['item_id']),{})
                    if item.get('type')=='mcpToolCall':q['requires_explicit_ui']=True
                    if q['method']=='item/fileChange/requestApproval':
                        q['input']['changes']=item.get('changes');q['reply_supported']=q['reply_supported'] and bool(item.get('changes'))
                    if len(json.dumps(q['input']).encode())>120000:
                        q['input']={'notice':'Complete approval context exceeds the safe display limit'};q['reply_supported']=False
                    self.pending[q['id']]=q;self.first_seen[q['id']]=q['time']
        if method in ['serverRequest/resolved','thread/status/changed'] or m.get('id') is not None and 'method' in m:
            try:self.publish()
            except OSError:pass

    def disconnected(self):
        with self.lock:self.subscribed.clear()

    def publish(self):
        output={};current=now()
        with self.lock:
            for address,row in self.rows.items():
                b=self.bound.get(address);entry={'address':address,'read_identity':identity(row),'at':current,'source':'codex_app_server','available':bool(b) or bool(self.history.get(address)),'binding_live':bool(b),'messages':self.history.get(address,[]),'connected':bool(self.client and not self.client.closed)}
                if b:
                    t=self.threads.get(b['thread_id'],{});qs={k:{f:v for f,v in q.items() if f not in ['rpc_id','questions']} for k,q in self.pending.items() if q['thread_id']==b['thread_id']}
                    active=t.get('status',{});word='blocked' if qs or any(v in active.get('activeFlags',[]) for v in ['waitingOnApproval','waitingOnUserInput']) else 'working' if active.get('type')=='active' else 'idle' if active.get('type')=='idle' else 'unavailable'
                    entry.update(thread_id=b['thread_id'],binding_method=b['method'],source_identity=b['view']['proof'],runtime_status=active,execution_state=word,pending_questions=qs,messages=self.history.get(address,[]),completion=self.completion.get(address,0),last_completed_turn=self.last_completed.get(address),title=t.get('name'))
                else:entry.update(error=row.get('binding_error','Codex runtime binding unavailable'),execution_state='unavailable',pending_questions={})
                output[address]=entry
        path=facts_path(self.home);path.parent.mkdir(parents=True,exist_ok=True,mode=0o700)
        with tempfile.NamedTemporaryFile(mode='w',dir=path.parent,delete=False) as f:json.dump(redact({'at':current,'sessions':output}),f,ensure_ascii=False);temp=f.name
        os.replace(temp,path)
        if self.streaming:print('{"updated":true}',flush=True)

    def refresh(self):
        snapshot=json.loads((self.home/'data/super-agent/sessions.json').read_text());self.core=snapshot['core']
        if now()-snapshot.get('at',0)>30000:return
        rows={r['address']:r for r in snapshot['sessions'] if not r.get('super_agent') and not r['address'].startswith('chat:') and (r.get('app')=='codex' or r.get('program',{}).get('key')=='codex' or r.get('read_source',{}).get('kind')=='codex_thread')}
        if not self.client or self.client.closed:
            self.pending.clear();self.subscribed.clear();self.client=CodexRPC(self.endpoint,self.message,self.disconnected)
        ids=self.client.rpc('thread/loaded/list',{})['data'];threads={}
        for ident in ids:
            threads[ident]=self.client.rpc('thread/read',{'threadId':ident,'includeTurns':False})['thread']
        with self.lock:self.rows=rows;self.threads=threads
        for address,row in rows.items():
            try:
                view=source_view(row,self.core);tid,method=bind(view,threads)
                with self.lock:self.bound[address]={'thread_id':tid,'method':method,'view':view}
                items=self.client.rpc('thread/items/list',{'threadId':tid,'limit':100,'sortDirection':'desc'})['data']
                with self.lock:
                    for entry in items:
                        item=entry.get('item',entry);self.live_items[(tid,item.get('id'))]=item
                if tid not in self.subscribed:
                    self.client.rpc('thread/resume',{'threadId':tid,'excludeTurns':True},timeout=10);self.subscribed.add(tid)
                messages=[v for v in (history_item(i) for i in items) if v]
                self.history[address]=list(reversed(messages[:40]))
                turns=self.client.rpc('thread/turns/list',{'threadId':tid,'limit':3,'itemsView':'notLoaded','sortDirection':'desc'})['data']
                completed=next((t['id'] for t in turns if t.get('status') in ['completed','failed','interrupted']),None)
                if completed and completed!=self.last_completed.get(address):self.completion[address]=self.completion.get(address,0)+1;self.last_completed[address]=completed
            except (OSError,ValueError,TimeoutError,KeyError):
                with self.lock:self.bound.pop(address,None);row['binding_error']='Cannot uniquely bind/read the live Codex source; no answer was sent'
        self.publish()

    def read(self,args):
        address=args['address']
        with self.lock:b=self.bound.get(address);row=self.rows.get(address)
        if not b or not self.client or self.client.closed:raise ValueError('Structured Codex source disconnected')
        view=source_view(row,self.core);tid,_=bind(view,self.threads)
        if tid!=b['thread_id'] or view['proof']!=b['view']['proof']:raise ValueError('Source session changed; wait for a fresh binding')
        offset=max(0,min(10000,int(args.get('offset',0))));limit=max(1,min(40,int(args.get('limit',40))))
        messages=[];cursor=None;more=False
        for _ in range(100):
            page=self.client.rpc('thread/items/list',{'threadId':tid,'limit':100,'sortDirection':'desc','cursor':cursor})
            messages.extend(v for v in (history_item(i) for i in page['data']) if v)
            cursor=page.get('nextCursor');more=bool(cursor)
            if len(messages)>=offset+limit or not cursor:break
        with self.lock:self.history[address]=list(reversed(messages[:40]))
        self.publish()
        result=json.loads(facts_path(self.home).read_text())['sessions'][address]
        result.update(messages=list(reversed(messages[offset:offset+limit])),next_offset=offset+min(limit,max(0,len(messages)-offset)),has_more=more or len(messages)>offset+limit,history_paginated=True)
        return result

    def send_message(self,args):
        # Serialize this bridge's dispatches; never type through the attached TUI.
        with self.send_lock:return self._send_message(args)

    def _send_message(self,args):
        ident=args.get('request_id','');address=args.get('address');text=args.get('text','')
        if not re.fullmatch(r'[a-zA-Z0-9_-]{8,100}',ident):raise DispatchRefused('Invalid request_id; nothing sent')
        if not isinstance(text,str) or not text.strip() or len(text)>100000 or any(ord(ch)<32 and ch not in '\n\t' for ch in text) or redact(text)!=text:raise DispatchRefused('Invalid/credential-bearing assignment; nothing sent')
        digest=hashlib.sha256(json.dumps({'address':address,'text':text},sort_keys=True).encode()).hexdigest()
        with sqlite3.connect(self.receipts) as db:
            previous=db.execute('SELECT digest,result FROM dispatches WHERE id=?',(ident,)).fetchone()
            if previous:
                if previous[0]!=digest:raise DispatchRefused('Request identity already belongs to a different assignment')
                if previous[1] is None:raise ValueError('Codex dispatch delivery unknown. Inspect this SAME session; do not retry or create a replacement worker')
                return json.loads(previous[1])
        with self.lock:
            b=self.bound.get(address);row=self.rows.get(address)
            if not b or not row or not self.client or self.client.closed:raise DispatchRefused('Original Codex runtime is disconnected/unbound; nothing sent. Keep this session and repair its connection')
            if args.get('read_identity')!=identity(row):raise DispatchRefused('Source terminal identity changed; nothing sent')
            if any(q['thread_id']==b['thread_id'] for q in self.pending.values()):raise DispatchRefused('Original Codex is awaiting a decision; answer that request instead. Nothing sent')
        try:
            view=source_view(row,self.core);tid,_=bind(view,self.threads)
            if tid!=b['thread_id'] or view['proof']!=b['view']['proof']:raise DispatchRefused('Source session changed; nothing sent')
            runtime=self.client.rpc('thread/read',{'threadId':tid,'includeTurns':False})['thread']
            if runtime.get('status',{}).get('type')!='idle':raise DispatchRefused('Original Codex is not idle; nothing sent')
        except (OSError,ValueError,KeyError,TimeoutError) as error:
            raise DispatchRefused(str(error)) from error
        # Persist uncertainty BEFORE writing to the runtime. A timeout or broken
        # reply must never turn into a retry with a new ID or a new worker.
        with sqlite3.connect(self.receipts) as db:db.execute('INSERT INTO dispatches VALUES (?,?,?,NULL)',(ident,digest,tid))
        reply=self.client.rpc('turn/start',{'threadId':tid,'input':[{'type':'text','text':text}],'clientUserMessageId':ident},timeout=12)
        turn=reply.get('turn',{}).get('id')
        if not turn:raise ValueError('Codex accepted no confirmed turn ID; inspect the same session before further dispatch')
        result={'submitted':True,'confirmed':True,'source':'codex_app_server','address':address,'thread_id':tid,'turn_id':turn,'request_id':ident,'note':'Original runtime acknowledged the input; this does not mean the task completed'}
        with sqlite3.connect(self.receipts) as db:db.execute('UPDATE dispatches SET result=? WHERE id=?',(json.dumps(result),ident))
        self.publish();return result

    def answer(self,args):
        address=args['address'];ident=args['question_id'];ui=args.get('ui',False)
        intent=args.get('intent') or {};receipt_id=args.get('request_id') if ui else intent.get('id')
        if not receipt_id:raise ValueError('Missing human decision identity')
        digest=hashlib.sha256(json.dumps({k:args.get(k) for k in ['address','question_id','answers','decision','intent']},sort_keys=True).encode()).hexdigest();receipt=ident+':'+receipt_id
        with sqlite3.connect(self.receipts,timeout=3) as c:
            previous=c.execute('SELECT digest,result FROM answers WHERE id=?',(receipt,)).fetchone()
            if previous:
                if previous[0]!=digest:raise ValueError('Decision identity already has a different response')
                if previous[1] is None:raise ValueError('Decision delivery is unknown. Inspect the worker; do not resend automatically')
                return json.loads(previous[1])
        with self.lock:
            q=self.pending.get(ident);b=self.bound.get(address);row=self.rows.get(address)
            if not q or not b or q['thread_id']!=b['thread_id'] or not self.client or self.client.closed:raise ValueError('This live question has already resolved, changed or disconnected')
            if not q['reply_supported']:raise ValueError('Secret/unsupported input must use its original authentication flow')
            q=dict(q)
        # Revalidate the current pane/process AND the runtime thread before every
        # reply. A switched/reused pane must never receive an old decision.
        view=source_view(row,self.core);tid,_=bind(view,self.threads)
        if tid!=b['thread_id'] or view['proof']!=b['view']['proof']:raise ValueError('Source session changed; nothing sent')
        pure='requestUserInput' in q['method']
        if not ui and (not pure or q.get('requires_explicit_ui') or intent.get('time',0)<q['time']):raise ValueError('Use a newer explicit human reply; approvals require the Super Agent approval buttons')
        if pure:
            if ui:
                if args.get('decision')!='allow':raise ValueError('The question remains pending until you answer it')
                supplied=args.get('answers') or {}
                answers={v['id']:{'answers':[supplied.get(v.get('question',''),'')]} for v in q['questions']}
            else:answers={v['id']:{'answers':[intent.get('text','')]} for v in q['questions']}
            if any(not v['answers'][0].strip() for v in answers.values()):raise ValueError('Answer every question first')
            response={'answers':answers}
        else:
            if not ui or args.get('decision') not in ['allow','deny']:raise ValueError('An explicit approval button is required')
            response={'decision':'accept' if args.get('decision')=='allow' else 'decline'}
        if redact(response)!=response:raise ValueError('Credentials cannot be relayed; use the original secure authentication flow')
        try:
            with sqlite3.connect(self.receipts,timeout=3) as c:
                c.execute('INSERT INTO claims VALUES (?,?)',(ident,receipt))
                c.execute('INSERT INTO answers VALUES (?,?,NULL)',(receipt,digest))
        except sqlite3.IntegrityError:raise ValueError('This request already has a submitted or unconfirmed decision; do not resend')
        with self.lock:
            if ident not in self.pending:
                with sqlite3.connect(self.receipts,timeout=3) as c:
                    c.execute('DELETE FROM answers WHERE id=?',(receipt,))
                    c.execute('DELETE FROM claims WHERE question_id=?',(ident,))
                raise ValueError('The question resolved before submission; nothing sent')
            self.client.send({'id':q['rpc_id'],'result':response})
        deadline=time.monotonic()+6
        while time.monotonic()<deadline:
            with self.lock:resolved=ident in self.resolved
            if resolved:break
            time.sleep(.03)
        result={'submitted':True,'request_resolved':resolved,'question_id':ident,'thread_id':tid,'address':address,'note':'Structured response sent to the live request; resolution is not proof the whole task completed'}
        with sqlite3.connect(self.receipts,timeout=3) as c:c.execute('UPDATE answers SET result=? WHERE id=?',(json.dumps(result),receipt))
        self.publish();return result

    def serve(self):
        path=socket_path()
        try:path.unlink()
        except FileNotFoundError:pass
        listener=socket.socket(socket.AF_UNIX);listener.bind(str(path));os.chmod(path,0o600);listener.listen();listener.settimeout(.5)
        def peer(s):
            try:
                s.settimeout(10);data=b''
                while b'\n' not in data:
                    block=s.recv(65536)
                    if not block or len(data)>1024*1024:raise ValueError('Invalid decision request')
                    data+=block
                args=json.loads(data.split(b'\n',1)[0]);result=self.answer(args) if args.get('action')=='answer' else self.read(args) if args.get('action')=='read' else self.send_message(args) if args.get('action')=='send' else {'error':'Unsupported bridge action'}
            except DispatchRefused as e:result={'error':redact(str(e))[:350],'kind':'refused'}
            except Exception as e:result={'error':redact(str(e))[:350],'kind':'unknown'}
            try:s.sendall((json.dumps(result,ensure_ascii=False)+'\n').encode())
            except OSError:pass
            s.close()
        while not self.stopped:
            try:s,_=listener.accept();threading.Thread(target=peer,args=(s,),daemon=True).start()
            except socket.timeout:pass
        listener.close()
        try:path.unlink()
        except FileNotFoundError:pass

    def run(self):
        threading.Thread(target=self.serve,daemon=True).start()
        while not self.stopped:
            try:self.refresh()
            except Exception:
                with self.lock:
                    self.bound.clear()
                self.publish()
            for _ in range(20):
                if self.stopped:break
                time.sleep(.1)
        if self.client:self.client.close()


if __name__=='__main__':
    home=pathlib.Path(os.environ['HERDR_APP_HOME']);endpoint=os.environ.get('HERDR_CODEX_CONTROL_SOCKET',str(pathlib.Path(os.environ.get('CODEX_HOME',str(pathlib.Path.home()/'.codex')))/'app-server-control/app-server-control.sock'))
    bridge=Bridge(home,endpoint);bridge.streaming=True
    def stop(*_):bridge.stopped=True
    signal.signal(signal.SIGTERM,stop);signal.signal(signal.SIGINT,stop)
    bridge.run()
