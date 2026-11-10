"""Release one native conversation's owned process tree; never accept arbitrary PIDs."""
import hashlib,json,os,pathlib,re,shlex,signal,socket,subprocess,sys,time

def processes():
    result=subprocess.run(['ps','-axo','pid=,ppid=,pgid=,stat=,lstart=,args='],capture_output=True,text=True,check=True,env={**os.environ,'LC_ALL':'C'})
    rows={}
    for line in result.stdout.splitlines():
        parts=line.split(None,9)
        if len(parts)==10:
            try:rows[int(parts[0])]={'parent':int(parts[1]),'group':int(parts[2]),'state':parts[3],'birth':' '.join(parts[4:9]),'command':parts[9]}
            except ValueError:pass
    return rows

def owned(row,script,folder):
    # ps argv is used only in memory: it can contain configured CLI credentials.
    try:args=shlex.split(row['command'])
    except ValueError:return False
    return any(pathlib.Path(arg).resolve()==script and i+1<len(args) and pathlib.Path(args[i+1]).resolve()==folder for i,arg in enumerate(args) if arg.startswith('/'))

def alive(rows,pid,identity):
    row=rows.get(pid)
    return bool(row and not row['state'].startswith('Z') and row['birth']==identity['birth'])

def release(root,conversation,request_id,grace=7):
    if not re.fullmatch(r'[a-f0-9]{16,64}',conversation) or conversation=='73757065726167656e74000000000001':raise ValueError('Invalid worker conversation; manager cannot release itself')
    if not re.fullmatch(r'[A-Za-z0-9_-]{8,100}',request_id):raise ValueError('Invalid request ID')
    root=pathlib.Path(root).resolve();raw_folder=root/'conversations'/conversation
    if not raw_folder.is_dir() or raw_folder.is_symlink():raise ValueError('Conversation runtime directory unavailable; nothing stopped')
    # Local service roots link conversations into the durable app home.
    folder=raw_folder.resolve()
    if any(ch.isspace() for ch in str(folder)):raise ValueError('Ambiguous process argument path; nothing stopped')
    import fcntl
    with (folder/'release.lock').open('a') as lock:
        fcntl.flock(lock,fcntl.LOCK_EX)
        rows=processes();roots={pid:r for pid,r in rows.items() if owned(r,root/'daemon.mjs',folder) or owned(r,root/'runner.py',folder)}
        if any(pid in [os.getpid(),os.getppid()] for pid in roots):raise ValueError('Refusing shared caller process')
        # Folder ownership comes from exact executable arguments, never a stale pidfile.
        targets=dict(roots)
        while True:
            children={pid:r for pid,r in rows.items() if r['parent'] in targets and pid not in targets}
            if not children:break
            targets.update(children)
        groups={pid for pid,r in targets.items() if r['group']==pid}
        def refresh():
            current=processes()
            for pid,r in current.items():
                if r['group'] in groups and pid not in targets:targets[pid]=r
            return current
        marker=folder/'runtime-released.json'
        if not roots:
            peer=socket.socket(socket.AF_UNIX);peer.settimeout(.3)
            try:peer.connect(str(folder/'run.sock'))
            except (FileNotFoundError,ConnectionRefusedError):pass
            else:raise ValueError('Runtime socket is active but ownership is unverified; nothing stopped')
            finally:peer.close()
            try:previous=json.loads(marker.read_text())
            except (OSError,ValueError):previous={}
            if previous.get('verified') is not True or previous.get('runtime_stopped') is not True:
                return {'conversation':conversation,'request_id':request_id,'runtime_stopped':False,'verified':False,'reason':'No live ownership tree or prior verified exit receipt; runtime exit cannot be established'}
        # Prevent transport reconnects from recreating a released worker.
        value={'conversation':conversation,'request_id':request_id,'runtime_stopped':False,'at':int(time.time()*1000)}
        temp=folder/'runtime-released.tmp';temp.write_text(json.dumps(value));os.chmod(temp,0o600);os.replace(temp,marker)
        def send(pid,sig):
            if alive(processes(),pid,targets[pid]):
                try:os.kill(pid,sig)
                except ProcessLookupError:pass
        # Graceful daemon shutdown first; its runner reaps the ordinary Claude PTY.
        for pid in roots:send(pid,signal.SIGTERM)
        deadline=time.monotonic()+grace
        while time.monotonic()<deadline and any(alive(current,pid,r) for current in [refresh()] for pid,r in list(targets.items())):time.sleep(.1)
        # Stop surviving session-owned tools too, never an unrelated/shared daemon.
        for sig in [signal.SIGTERM,signal.SIGKILL]:
            for pid in reversed(list(targets)):send(pid,sig)
            deadline=time.monotonic()+1
            while time.monotonic()<deadline and any(alive(current,pid,r) for current in [refresh()] for pid,r in list(targets.items())):time.sleep(.05)
        current=refresh();remaining=[pid for pid,r in targets.items() if alive(current,pid,r)]
        # Detect a newly forked owned runner rather than claiming a false success.
        after=processes();new_roots=[pid for pid,r in after.items() if not r['state'].startswith('Z') and (owned(r,root/'daemon.mjs',folder) or owned(r,root/'runner.py',folder))]
        value.update(runtime_stopped=not remaining and not new_roots,verified=not remaining and not new_roots,owned_processes=len(targets),remaining_processes=len(set(remaining+new_roots)))
        temp.write_text(json.dumps(value));os.chmod(temp,0o600);os.replace(temp,marker)
        return value

if __name__=='__main__':
    try:
        a=json.load(sys.stdin);root=a.get('root') or str(pathlib.Path.home()/'.local/share/herdr-chat');print(json.dumps(release(root,a['conversation'],a['request_id'])))
    except Exception:
        print(json.dumps({'error':'Runtime release could not verify ownership/exit; do not report resources released','runtime_stopped':False,'verified':False}));sys.exit(1)
