import unittest,tempfile,pathlib,json,time
import goal_store as g,goal_watch as w
class Watch(unittest.TestCase):
 def setUp(self):
  self.t=tempfile.TemporaryDirectory();self.addCleanup(self.t.cleanup);self.h=pathlib.Path(self.t.name);g.root(self.h).mkdir(parents=True)
  self.row={'address':'chat:p','conversation':'c','host':'qa','status':'idle','completion':1};binding={k:self.row.get(k) for k in ['conversation','host','read_identity']}
  goal={'id':'goal','enabled':True,'phase':'active','needs_user':False,'outcome':'完成验证','plan':[{'id':'step','address':'chat:p','binding':binding}]}
  with g.db(self.h) as c:c.execute('INSERT INTO goals VALUES (?,?)',('goal',json.dumps(goal)))
  (g.root(self.h)/'active-observation.json').write_text(json.dumps({'id':'observe_quiet'}))
  (g.root(self.h)/'goal-session-watch.json').write_text(json.dumps({'at':int(time.time()*1000),'workers':{'chat:p':{'binding':binding,'completion':1,'since':int(time.time()*1000)-16*60000}}}))
  self.calls=[]
 def ask(self,read=None,refresh=None):return w.ask(self.h,{'goal_id':'goal','step_id':'step'},[self.row],read or (lambda _:{'available':True,'cached':False}),lambda a:self.calls.append(a) or {'queued':True},refresh or (lambda:[self.row]))
 def test_once_per_episode_survives_restart_and_uses_only_fixed_original_question(self):
  a=self.ask();self.assertEqual(a,self.ask());self.assertEqual(len(self.calls),1);self.assertEqual(self.calls[0]['address'],'chat:p');self.assertEqual(self.calls[0]['mode'],'queue');self.assertIn('当前阻塞',self.calls[0]['text'])
 def test_pending_approval_offline_busy_and_identity_changes_refuse(self):
  for key,value in [('status','working'),('attention','question'),('detail','SSH disconnected')]:
   old=dict(self.row);self.row[key]=value
   with self.assertRaises(ValueError):self.ask()
   self.row=old
  with self.assertRaises(ValueError):self.ask(read=lambda _:{'available':True,'cached':True})
  with self.assertRaises(ValueError):self.ask(read=lambda _:{'available':True,'pending_permissions':{'q':{}}})
  with self.assertRaises(ValueError):self.ask(refresh=lambda:[{**self.row,'conversation':'different'}])
  self.assertEqual(self.calls,[])
 def test_uncertain_delivery_is_not_replayed(self):
  def fail(a):self.calls.append(a);raise TimeoutError('lost receipt')
  a=w.ask(self.h,{'goal_id':'goal','step_id':'step'},[self.row],lambda _:{'available':True,'cached':False},fail,lambda:[self.row])
  self.assertEqual(a['delivery'],'unknown');self.assertEqual(self.ask(),a);self.assertEqual(len(self.calls),1)
