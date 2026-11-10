import json,os,pathlib,signal,subprocess,sys,tempfile,time,unittest
from runtime_release import release,processes,alive
class ReleaseTests(unittest.TestCase):
 def test_owned_tree_stops_neighbor_survives_and_reconnect_is_guarded(self):
  with tempfile.TemporaryDirectory(prefix='hrr-',dir='/tmp') as temp:
   root=pathlib.Path(temp);durable=root/'durable';durable.mkdir();(root/'conversations').symlink_to(durable,target_is_directory=True);script=root/'daemon.mjs'
   script.write_text("import subprocess,sys,pathlib,time\np=subprocess.Popen([sys.executable,'-c','import time;time.sleep(120)'],start_new_session=True)\npathlib.Path(sys.argv[1]+'/child.pid').write_text(str(p.pid))\ntime.sleep(120)\n")
   ids=['a'*32,'b'*32];children=[];ps=[]
   try:
    for ident in ids:
     folder=root/'conversations'/ident;folder.mkdir(parents=True)
     p=subprocess.Popen([sys.executable,str(script),str(folder)],start_new_session=True);ps.append(p)
     for _ in range(100):
      if (folder/'child.pid').exists():break
      time.sleep(.01)
     children.append(int((folder/'child.pid').read_text()))
    # A malicious/stale pidfile must not target the unrelated daemon.
    (root/'conversations'/ids[0]/'daemon.pid').write_text(str(ps[1].pid))
    result=release(root,ids[0],'release-test-one',grace=.1)
    self.assertTrue(result['runtime_stopped']);self.assertTrue(result['verified']);ps[0].wait(timeout=3)
    self.assertIsNone(ps[1].poll());os.kill(children[1],0)
    self.assertTrue(release(root,ids[0],'release-test-again',grace=.1)['runtime_stopped'])
    client=pathlib.Path(__file__).with_name('client.mjs');env={**os.environ,'HERDR_CHAT_HOME':str(root)}
    r=subprocess.run(['node',str(client),ids[0],temp,'opus'],input='',capture_output=True,text=True,env=env,timeout=5)
    self.assertEqual(r.returncode,0);self.assertIn('explicitly released',r.stdout)
   finally:
    for p in ps:
     if p.poll() is None:p.kill()
     p.wait()
    for pid in children:
     try:os.kill(pid,signal.SIGKILL)
     except ProcessLookupError:pass
 def test_manager_and_invalid_ids_are_refused(self):
  for ident in ['../elsewhere','73757065726167656e74000000000001']:
   with self.assertRaises(ValueError):release('/tmp',ident,'release-test-safe')
 def test_missing_owner_is_not_claimed_as_released(self):
  with tempfile.TemporaryDirectory(prefix='hrr-',dir='/tmp') as temp:
   root=pathlib.Path(temp);(root/'conversations'/('c'*32)).mkdir(parents=True)
   self.assertFalse(release(root,'c'*32,'release-missing')['verified'])
 def test_pid_identity_change_is_not_signalled(self):
  self.assertFalse(alive({7:{'birth':'new','state':'S'}},7,{'birth':'old'}))
if __name__=='__main__':unittest.main()
