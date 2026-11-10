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
if __name__=='__main__':unittest.main()
