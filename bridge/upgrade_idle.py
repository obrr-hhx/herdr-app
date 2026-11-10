"""Upgrade only an idle legacy worker with no other client or background runtime."""
import contextlib,fcntl,json,os,pathlib,signal,sys,time
from runtime_release import processes,owned,alive

def upgrade(folder):
 raw_folder=pathlib.Path(folder).absolute();root=raw_folder.parent.parent.resolve();folder=raw_folder.resolve()
 if folder.name=='73757065726167656e74000000000001':return {'upgraded':False,'reason':'manager'}
 with (folder/'upgrade.lock').open('a') as lock:
  fcntl.flock(lock,fcntl.LOCK_EX)
  def inspect():
   rows=processes();daemons={pid:r for pid,r in rows.items() if owned(r,root/'daemon.mjs',folder) and not r['state'].startswith('Z')}
   if len(daemons)!=1:return None,'ownership'
   pid,identity=next(iter(daemons.items()))
   try:
    marker=json.loads((folder/'bridge-protocol.json').read_text())
    if marker.get('pid')==pid and marker.get('version',0)>=2:return None,'already_current'
   except (OSError,ValueError):pass
   # Every client for this exact conversation is counted; another app's client
   # makes upgrading unsafe because its incoming input cannot be held here.
   # Client argv contains an ID, not a path. Match it without shell expansion.
   import shlex
   clients=[]
   for r in rows.values():
    try:a=shlex.split(r['command'])
    except ValueError:continue
    if any(arg.startswith('/') and pathlib.Path(arg).resolve()==root/'client.mjs' and i+1<len(a) and a[i+1]==folder.name for i,arg in enumerate(a)):clients.append(r)
   if len(clients)!=1 or not any(r is rows.get(os.getppid()) for r in clients):return None,'other_clients'
   tree={pid:identity}
   while True:
    more={p:r for p,r in rows.items() if r['parent'] in tree and p not in tree and not r['state'].startswith('Z')}
    if not more:break
    tree.update(more)
   if len(tree)>3:return None,'background_processes'
   if len(tree)>1 and not any(owned(r,root/'runner.py',folder) for r in tree.values()):return None,'unknown_runtime'
   try:es=[json.loads(l) for l in (folder/'events.jsonl').read_text().splitlines()]
   except (OSError,ValueError):return None,'history'
   state=next((e for e in reversed(es) if e['type']=='state'),{})
   if state.get('status')!='idle' or time.time()*1000-state.get('time',0)<1000:return None,'working'
   pending=set();confirmed={e.get('id') for e in es if e['type']=='prompt_confirmed'}
   for e in es:
    if e['type']=='permission':pending.add(e['id'])
    if e['type']=='permission_resolved':pending.discard(e['id'])
   if pending:return None,'approval'
   if any(e['type']=='accepted' and e.get('action')=='send' and e.get('request_id') not in confirmed for e in es):return None,'unconfirmed_input'
   if any(e['type']=='tool' and e.get('name') in ['Task','Agent'] and e.get('input',{}).get('run_in_background') for e in es):return None,'background_agent_not_verified'
   return (pid,identity,tree),'idle'
  candidate,reason=inspect()
  if not candidate:return {'upgraded':False,'reason':reason}
  time.sleep(.1);again,reason=inspect()
  if not again or again[0]!=candidate[0] or again[1]['birth']!=candidate[1]['birth']:return {'upgraded':False,'reason':reason}
  pid,identity,tree=again
  if not alive(processes(),pid,identity):return {'upgraded':False,'reason':'identity_changed'}
  os.kill(pid,signal.SIGTERM)
  deadline=time.monotonic()+8
  while time.monotonic()<deadline:
   rows=processes()
   if all(not alive(rows,p,r) for p,r in tree.items()):
    return {'upgraded':True,'history_preserved':True}
   time.sleep(.1)
  return {'upgraded':False,'reason':'exit_unconfirmed'}
if __name__=='__main__':
 try:print(json.dumps(upgrade(sys.argv[1])))
 except Exception:print(json.dumps({'upgraded':False,'reason':'guard_unavailable'}))
