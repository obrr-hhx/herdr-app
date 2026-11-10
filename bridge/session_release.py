"""Explicit release orchestration, separate from reversible list archiving."""
import contextlib,hashlib,json,os,pathlib,re,shlex,sqlite3,subprocess

def release(home,binary,args,app,archive):
    context=pathlib.Path(home)/'data/super-agent/active-observation.json'
    try:active=json.loads(context.read_text()).get('id','')
    except (OSError,ValueError):active=''
    if active.startswith(('observe_','manual_review_')):raise ValueError('Background/manual observations cannot release workers')
    address=args['address'];request=args['request_id']
    if not re.fullmatch(r'[A-Za-z0-9_-]{8,100}',request):raise ValueError('Invalid request ID')
    snap=app('supervisor.list');intent=snap.get('latest_user_intent') or {}
    if not intent.get('id') or args['user_intent_id']!=intent['id']:raise ValueError('Release requires latest explicit human authorization')
    row=next((r for r in snap['sessions'] if r['address']==address),None)
    old=next((r for r in reversed(snap.get('closed_sessions',[])) if r['address']==address),None)
    if not row and old:row=old['session']
    if not row or row.get('super_agent') or row.get('conversation')=='73757065726167656e74000000000001':raise ValueError('Exact worker unavailable or is Super Agent; nothing released')
    identity={'conversation':row['conversation'],'host':row.get('host')} if row.get('conversation') else row.get('read_identity')
    if not identity:raise ValueError('Worker identity unavailable; nothing released')
    if old and old.get('identity')!=identity:old=None
    folder=pathlib.Path(home)/'data/super-agent';folder.mkdir(parents=True,exist_ok=True)
    with contextlib.closing(sqlite3.connect(folder/'releases.sqlite')) as c:
        os.chmod(folder/'releases.sqlite',0o600)
        c.execute('CREATE TABLE IF NOT EXISTS releases(id TEXT PRIMARY KEY,address TEXT,identity TEXT,result TEXT)')
        encoded=json.dumps(identity,sort_keys=True)
        with c:
            previous=c.execute('SELECT address,identity,result FROM releases WHERE id=?',(request,)).fetchone()
            if previous:
                if previous[:2]!=(address,encoded):raise ValueError('Release request reused for a different session')
                if previous[2]:return json.loads(previous[2])
                raise ValueError('Earlier release outcome is unknown; inspect same runtime before retrying with a new explicit request')
        # Archive first so transport clients disconnect before the runtime is stopped.
        closed=archive({'address':address,'request_id':old['request_id'] if old else request,'user_intent_id':args['user_intent_id']})
        if not closed.get('verified') or closed.get('pending'):return {**closed,'runtime_stopped':False,'released':False}
        with c:c.execute('INSERT INTO releases VALUES (?,?,?,NULL)',(request,address,encoded))
        if row.get('conversation'):
            host=row.get('host') or '@local'
            if host!='@local' and not re.fullmatch(r'[A-Za-z0-9_][A-Za-z0-9_.@:-]*',host):raise ValueError('Invalid worker host')
            source=pathlib.Path(__file__).with_name('runtime_release.py').read_text()
            payload={'conversation':row['conversation'],'request_id':request}
            if host=='@local':
                payload['root']=str(pathlib.Path('/tmp')/('hchat-'+hashlib.sha256(str(home).encode()).hexdigest()[:16]));command=['python3','-c',source]
            else:
                # Install reconnect guards only; this does not restart any worker.
                install=subprocess.run([binary,'chat-install',host],capture_output=True,timeout=35)
                if install.returncode:raise ValueError('Could not install remote reconnect guard; runtime not released')
                command=['ssh','-o','BatchMode=yes','-o','ConnectTimeout=8','--',host,'python3 -c '+shlex.quote(source)]
            proc=subprocess.run(command,input=json.dumps(payload),capture_output=True,text=True,timeout=25)
            try:result=json.loads(proc.stdout)
            except ValueError:raise ValueError('Release confirmation unavailable; inspect same worker, do not claim released')
        else:
            core=snap['core']
            def get():
                p=subprocess.run([core,'get',address,'--json'],capture_output=True,text=True,timeout=10)
                if p.returncode:raise ValueError('Core worker unavailable; runtime exit not verified')
                value=json.loads(p.stdout);return value.get('terminal',value)
            before=get()
            if {'pid':before.get('pid'),'created_ms':before.get('created_ms')}!=identity:raise ValueError('Original core worker identity changed; nothing stopped')
            proc=subprocess.run([core,'kill',address,'--json'],capture_output=True,text=True,timeout=25)
            after=get();stopped=proc.returncode==0 and after.get('state')=='finished' and {'pid':after.get('pid'),'created_ms':after.get('created_ms')}==identity
            result={'runtime_stopped':stopped,'verified':stopped,'adapter':'herdr_core'}
        result={**result,'address':address,'request_id':request,'archived':True,'history_preserved':True,'released':result.get('runtime_stopped') is True and result.get('verified') is True}
        # Failed/partial receipts are retained too, never reinterpreted as success.
        with c:c.execute('UPDATE releases SET result=? WHERE id=?',(json.dumps(result),request))
        return result


def decorate(home,snapshot):
    """Expose current release receipts without rewriting historical archive records."""
    path=pathlib.Path(home)/'data/super-agent/releases.sqlite'
    snapshot['runtime_releases']=[]
    if not path.exists():return snapshot
    with contextlib.closing(sqlite3.connect('file:'+str(path)+'?mode=ro',uri=True)) as c:
        for address,identity,result in c.execute('SELECT address,identity,result FROM releases'):
            receipt=json.loads(result) if result else {'runtime_stopped':False,'verified':False,'released':False,'delivery':'unknown'}
            snapshot['runtime_releases'].append({'address':address,'identity':json.loads(identity),'receipt':receipt})
            for row in snapshot.get('closed_sessions',[]):
                if row.get('address')==address and row.get('identity')==json.loads(identity):row['runtime_release']=receipt
    return snapshot
