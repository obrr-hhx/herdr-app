import json,pathlib,subprocess,tempfile,unittest
from unittest.mock import patch
import session_release
class SessionReleaseTests(unittest.TestCase):
 def setUp(self):
  self.temp=tempfile.TemporaryDirectory(prefix='hrr-',dir='/tmp');self.home=pathlib.Path(self.temp.name)
  self.row={'address':'chat:p1','conversation':'a'*32,'host':'@local'};self.snap={'latest_user_intent':{'id':'human'},'sessions':[self.row],'closed_sessions':[]};self.archives=[]
 def tearDown(self):self.temp.cleanup()
 def archive(self,a):self.archives.append(a);return {'verified':True,'pending':False}
 def run_release(self,**args):return session_release.release(self.home,'/fixture/app',{'address':'chat:p1','request_id':'release-request-01','user_intent_id':'human',**args},lambda *_:self.snap,self.archive)
 def test_verified_native_receipt_is_durable_and_uses_original_identity(self):
  with patch.object(subprocess,'run',return_value=subprocess.CompletedProcess([],0,json.dumps({'runtime_stopped':True,'verified':True}),'')) as run:
   self.assertTrue(self.run_release()['released']);self.assertTrue(self.run_release()['released']);self.assertEqual(run.call_count,1)
   payload=json.loads(run.call_args.kwargs['input']);self.assertEqual(payload['conversation'],self.row['conversation']);self.assertEqual(len(self.archives),1)
   decorated=session_release.decorate(self.home,self.snap);self.assertTrue(decorated['runtime_releases'][0]['receipt']['released'])
 def test_archived_worker_uses_original_archive_receipt(self):
  self.snap['sessions']=[];self.snap['closed_sessions']=[{'address':'chat:p1','identity':{'conversation':'a'*32,'host':'@local'},'session':self.row,'request_id':'archive-original'}]
  with patch.object(subprocess,'run',return_value=subprocess.CompletedProcess([],0,'{"runtime_stopped":false,"verified":false}','')):
   self.assertFalse(self.run_release()['released']);self.assertEqual(self.archives[0]['request_id'],'archive-original')
 def test_explicit_intent_manager_and_background_guards(self):
  with self.assertRaises(ValueError):self.run_release(user_intent_id='old')
  self.row['super_agent']=True
  with self.assertRaises(ValueError):self.run_release()
  self.row['super_agent']=False;p=self.home/'data/super-agent';p.mkdir(parents=True);(p/'active-observation.json').write_text('{"id":"observe_result"}')
  with self.assertRaises(ValueError):self.run_release()
  self.assertEqual(self.archives,[])
 def test_unconfirmed_archive_never_stops_runtime(self):
  with patch.object(subprocess,'run') as run:
   result=session_release.release(self.home,'/fixture/app',{'address':'chat:p1','request_id':'release-request-01','user_intent_id':'human'},lambda *_:self.snap,lambda *_:{'verified':False,'pending':True})
   self.assertFalse(result['released']);run.assert_not_called()
 def test_delivery_unknown_is_not_blindly_retried(self):
  with patch.object(subprocess,'run',side_effect=subprocess.TimeoutExpired('fixture',1)) as run:
   with self.assertRaises(subprocess.TimeoutExpired):self.run_release()
   with self.assertRaisesRegex(ValueError,'unknown'):self.run_release()
   self.assertEqual(run.call_count,1)
 def test_core_identity_change_prevents_kill(self):
  self.row={'address':'t4','read_identity':{'pid':123,'created_ms':1}};self.snap['sessions']=[self.row];self.snap['core']='/fixture/hc'
  with patch.object(subprocess,'run',return_value=subprocess.CompletedProcess([],0,'{"pid":456,"created_ms":2}','')) as run:
   with self.assertRaisesRegex(ValueError,'identity changed'):self.run_release(address='t4')
   self.assertEqual(run.call_count,1);self.assertEqual(run.call_args.args[0][1],'get')
if __name__=='__main__':unittest.main()
