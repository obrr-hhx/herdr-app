import json, pathlib, tempfile, unittest
import memory_store as m
class Memory(unittest.TestCase):
 def setUp(self):
  self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup);self.home=pathlib.Path(self.tmp.name)
  self.journal=self.home/'data/forge-chat'/m.SUPER/'events.jsonl';self.journal.parent.mkdir(parents=True)
  m.root(self.home).mkdir(parents=True)
  self.human('u1','以后先给结论和证据，不要只报中间进度。')
 def human(self,id,text,type='user'):
  with self.journal.open('a') as f:f.write(json.dumps({'type':type,'id':id,'text':text,'time':int(__import__('time').time()*1000)-180000})+'\n')
  (m.root(self.home)/'active-observation.json').write_text(json.dumps({'id':id}))
 def args(self,**kw):
  return {'topic':'report.evidence','expected_revision':0,'title':'报告方式','content':'先给结论和证据，不只报告中间进度。','kind':'workflow','confidence':'explicit','importance':5,'sources':[{'id':'u1','quote':'先给结论和证据'}],**kw}
 def test_persist_dedup_revision_and_projection(self):
  r=m.save(self.home,self.args());self.assertEqual(r['revision'],1);self.assertFalse(r['authority'])
  self.assertEqual(m.search(self.home,{'query':'结论'})['memories'][0],r)
  with self.assertRaises(ValueError):m.save(self.home,self.args())
  self.human('u2','更正：保持简短，先给证据。')
  a=self.args(expected_revision=1,content='保持简短，先给证据。',sources=[{'id':'u2','quote':'保持简短，先给证据。'}])
  self.assertEqual(m.save(self.home,a)['revision'],2)
  self.assertEqual(m.search(self.home,{})['total'],1)
  self.assertEqual(json.loads((m.root(self.home)/'memories.json').read_text())['memories'][0]['content'],a['content'])
 def test_only_actual_current_human_quotes(self):
  for refs in [[{'id':'worker','quote':'先给结论'}],[{'id':'u1','quote':'伪造要求'}]]:
   with self.assertRaises(ValueError):m.save(self.home,self.args(sources=refs))
  self.human('observe_2','后台学习')
  with self.assertRaises(ValueError):m.save(self.home,self.args())
  self.human('w','先给结论和证据','assistant')
  with self.assertRaises(ValueError):m.save(self.home,self.args(sources=[{'id':'w','quote':'先给结论和证据'}]))
 def test_secrets_and_inference(self):
  with self.assertRaises(ValueError):m.save(self.home,self.args(content='API_KEY=sk-'+'a'*30))
  with self.assertRaises(ValueError):m.save(self.home,self.args(confidence='inferred'))
  self.human('u2','也请先给结论和证据')
  r=m.save(self.home,self.args(confidence='inferred',sources=[{'id':'u1','quote':'先给结论和证据'},{'id':'u2','quote':'先给结论和证据'}]))
  self.assertEqual(r['confidence'],'inferred')
 def test_forget_removes_payload_and_prevents_old_relearning(self):
  m.save(self.home,self.args());self.human('u2','请忘记 report.evidence 这条记忆')
  a={'id':'report.evidence','expected_revision':1,'user_intent_id':'u2','quote':'请忘记 report.evidence 这条记忆'}
  self.assertTrue(m.forget(self.home,a)['forgotten']);self.assertEqual(m.search(self.home,{})['total'],0)
  self.assertNotIn('先给结论',(m.root(self.home)/'memories.sqlite').read_bytes().decode('utf8','ignore'))
  with self.assertRaises(ValueError):m.save(self.home,self.args(),admin=True)
  with self.assertRaises(ValueError):m.save(self.home,self.args(topic='renamed.forgotten'),admin=True)
  self.human('u3','重新记住 report.evidence：先给结论和证据')
  m.save(self.home,self.args(sources=[{'id':'u3','quote':'先给结论和证据'}]));self.assertEqual(m.search(self.home,{})['total'],1)
 def test_forget_needs_exact_request_not_worker_or_general_cleanup(self):
  m.save(self.home,self.args());self.human('u2','继续工作')
  with self.assertRaises(ValueError):m.forget(self.home,{'id':'report.evidence','expected_revision':1,'user_intent_id':'u2','quote':'继续工作'})
 def test_direct_manual_crud_without_manager_or_chat_connection(self):
  args={'action':'save','id':'personal.fixture','expected_revision':0,'title':'汇报偏好','content':'先给结果和证据'}
  r=m.manual_edit(self.home,args)['memories'][0];self.assertTrue(r['edited_by_user']);self.assertFalse(r['authority']);self.assertEqual(r['sources'][0]['origin'],'user_editor')
  self.assertEqual(m.search(self.home,{})['total'],1)
  with self.assertRaises(ValueError):m.manual_edit(self.home,args)
  r=m.manual_edit(self.home,{**args,'expected_revision':1,'content':'更简短'})['memories'][0]
  self.assertEqual(r['revision'],2);self.assertEqual(r['content'],'更简短')
  with self.assertRaises(ValueError):m.manual_edit(self.home,{'action':'delete','id':r['id'],'expected_revision':1})
  self.assertEqual(len(m.manual_edit(self.home,{'action':'delete','id':r['id'],'expected_revision':2})['memories']),0)
 def test_direct_manual_rejects_empty_and_credentials_without_altering_record(self):
  for text in ['', 'API_KEY=sk-'+'z'*30]:
   with self.assertRaises(ValueError):m.manual_edit(self.home,{'action':'save','id':'personal.fixture','expected_revision':0,'title':'偏好','content':text})
  self.assertEqual(m.search(self.home,{})['total'],0)
 def test_layered_summary_index_and_exact_reads(self):
  m.save(self.home,self.args());s=m.snapshot(self.home)
  self.assertIn('先给结论',s['summary']);self.assertFalse(s['compiled'])
  self.assertNotIn('content',m.list_notes(self.home,{})['memories'][0])
  self.assertEqual(m.read_note(self.home,{'id':'report.evidence'})['memory']['sources'][0]['id'],'u1')
  self.assertTrue(m.read_note(self.home,{'id':'absent'})['missing'])
 def test_consolidation_covers_sources_and_invalidates_immediately_on_edit_delete(self):
  r=m.save(self.home,self.args());s=m.snapshot(self.home)
  a={'expected_version':s['version'],'reviewed_source_ids':['u1'],'profile':[],'preferences':[{'text':r['content'],'memory_ids':[r['id']]}],'topics':[]}
  self.assertTrue(m.consolidate(self.home,a)['compiled']);self.assertTrue(m.snapshot(self.home)['compiled'])
  with self.assertRaises(ValueError):m.consolidate(self.home,{**a,'preferences':[]})
  m.manual_edit(self.home,{'action':'save','id':r['id'],'expected_revision':1,'title':'已更正','content':'全新的偏好'})
  fresh=m.snapshot(self.home);self.assertFalse(fresh['compiled']);self.assertNotIn(r['content'],fresh['summary'])
  with self.assertRaises(ValueError):m.consolidate(self.home,a)
  m.manual_edit(self.home,{'action':'delete','id':r['id'],'expected_revision':2})
  self.assertNotIn('全新的偏好',m.snapshot(self.home)['summary'])
 def test_maintenance_can_extract_only_pending_actual_human_feedback_and_cannot_change_manual_notes(self):
  self.human('observe_memory_fixture','maintenance','observation')
  r=m.save(self.home,self.args())
  self.assertEqual(r['revision'],1)
  self.human('observe_regular_fixture','regular','observation')
  with self.assertRaises(ValueError):m.save(self.home,self.args(topic='other'))
  m.manual_edit(self.home,{'action':'save','id':r['id'],'expected_revision':1,'title':'手动','content':'用户手动固定的偏好'})
  self.human('observe_memory_fixture2','maintenance','observation')
  with self.assertRaises(ValueError):m.save(self.home,self.args(expected_revision=2))
  s=m.snapshot(self.home)
  with self.assertRaises(ValueError):m.consolidate(self.home,{'expected_version':s['version'],'reviewed_source_ids':['u1'],'profile':[],'preferences':[{'text':'随意改写用户偏好','memory_ids':[r['id']]}],'topics':[]})
