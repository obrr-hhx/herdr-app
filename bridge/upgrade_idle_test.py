import json,os,pathlib,tempfile,time,unittest
from unittest.mock import patch
import upgrade_idle
class UpgradeTests(unittest.TestCase):
 def test_unsafe_legacy_runtime_is_never_signalled(self):
  with tempfile.TemporaryDirectory(prefix='hsu-',dir='/tmp') as tmp:
   root=pathlib.Path(tmp);folder=root/'conversations'/('a'*32);folder.mkdir(parents=True)
   def row(cmd,parent=1):return {'command':cmd,'parent':parent,'birth':'fixture','state':'S','group':999}
   rows={999:row(f'node {root}/daemon.mjs {folder} /qa'),os.getppid():row(f'node {root}/client.mjs {folder.name} /qa')}
   base=[{'type':'state','status':'idle','time':int(time.time()*1000)-5000}]
   cases=[(base+[{'type':'permission','id':'q'}],rows,'approval'),(base+[{'type':'accepted','action':'send','request_id':'unconfirmed'}],rows,'unconfirmed_input'),([{'type':'state','status':'working','time':1}],rows,'working'),(base,{**rows,444:row(f'node {root}/client.mjs {folder.name} /qa')},'other_clients'),(base,{**rows,333:row(f'python3 {root}/runner.py {folder} /qa',999),334:row('claude',333),335:row('background tool',334)},'background_processes')]
   for events,processes,reason in cases:
    (folder/'events.jsonl').write_text(''.join(json.dumps(e)+'\n' for e in events))
    with patch.object(upgrade_idle,'processes',return_value=processes),patch.object(upgrade_idle.os,'kill') as kill:
     self.assertEqual(upgrade_idle.upgrade(folder)['reason'],reason);kill.assert_not_called()
 def test_revision_upgrade_counts_only_the_owned_lock_helper_and_keeps_background_tools(self):
  with tempfile.TemporaryDirectory(prefix='hsu-',dir='/tmp') as tmp:
   root=pathlib.Path(tmp);folder=root/'conversations'/('a'*32);folder.mkdir(parents=True)
   def row(cmd,parent=1):return {'command':cmd,'parent':parent,'birth':'fixture','state':'S','group':999}
   rows={999:row(f'node {root}/daemon.mjs {folder} /qa'),os.getppid():row(f'node {root}/client.mjs {folder.name} /qa'),331:row(f'python3 -u -c import os,sys ... {folder}/daemon.lock',999),333:row(f'python3 {root}/runner.py {folder} /qa',999),334:row('claude',333)}
   (folder/'events.jsonl').write_text(json.dumps({'type':'state','status':'idle','time':int(time.time()*1000)-5000})+'\n')
   (folder/'bridge-protocol.json').write_text(json.dumps({'version':2,'pid':999,'revision':'old'}))
   for revision,background,reason in [('old',False,'already_current'),('new',True,'background_processes')]:
    current={**rows,**({335:row('background tool',334)} if background else {})}
    with patch.object(upgrade_idle,'processes',return_value=current),patch.object(upgrade_idle.os,'kill') as kill:
     self.assertEqual(upgrade_idle.upgrade(folder,revision)['reason'],reason);kill.assert_not_called()
   stopped=[]
   def signal(pid,sig):stopped.append(pid)
   def processes():return {p:{**r,'state':'Z' if stopped and p!=os.getppid() else r['state']} for p,r in rows.items()}
   with patch.object(upgrade_idle,'processes',side_effect=processes),patch.object(upgrade_idle.os,'kill',side_effect=signal),patch.object(upgrade_idle.time,'sleep'):
    self.assertTrue(upgrade_idle.upgrade(folder,'new')['upgraded']);self.assertEqual(stopped,[999])
if __name__=='__main__':unittest.main()
