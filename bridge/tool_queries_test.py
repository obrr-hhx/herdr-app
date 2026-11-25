import json,os,pathlib,tempfile,time,unittest
from unittest.mock import patch
import tool_queries as q

class Queries(unittest.TestCase):
 def setUp(self):
  self.temp=tempfile.TemporaryDirectory(prefix='hquery-');self.home=pathlib.Path(self.temp.name);self.calls=[]
  self.data={
   'list_decisions':{'decisions':[{'id':str(i),'status':'resolved' if i<49 else 'withdrawn' if i<92 else 'answered' if i==92 else 'pending','title':f'Card {i}','context':('长背景🙂 '*1000)+('hiddenneedle' if i==3 else ''),'options':['A','B']} for i in range(95)]},
   'list_sessions':{'sessions':[{'address':f't{i}','title':f'Worker {i}','status':'idle','program':{'huge':'X'*10000}} for i in range(35)],'tasks':[{'id':'task','summary':'ledger'}],'goals':[{'id':'goal'}],'assignments':[{'id':'dispatch'}],'machines':[{'id':'machine'}],'latest_user_intent':{'id':'human','text':'Do the work','time':10}},
   'list_goals':{'goals':[{'id':str(i),'title':'Goal','revision':1,'plan':[{'instruction':'X'*10000}]} for i in range(100)]},
   '_memory_snapshot':{'version':'v1','memories':[{'id':str(i),'title':'Note','content':'Chinese 中文 target' if i==123 else 'Other','sources':[{'quote':'X'*5000}]} for i in range(150)]},
   'review_memories':{'version':'v1','memories':[{'id':'note','content':'Preference'}],'pending_human_feedback':[{'id':str(i),'text':'Full human feedback '+('x'*1000),'time':i} for i in range(12)]},
   'list_reminders':{'schedules':[{'id':str(i),'title':'Reminder','text':'x'*10000} for i in range(100)],'unread_reminders':[{'id':'due'}],'now':10,'timezone':'Asia/Shanghai'},
  }
 def tearDown(self):self.temp.cleanup()
 def raw(self,name,args):self.calls.append((name,args));return self.data['list_sessions' if name=='_inventory' else name]
 def invoke(self,name,args=None):
  result=q.invoke(self.home,name,args or {},self.raw)
  self.assertLessEqual(len(q.encoded(result)),q.MAX_OUTPUT)
  return result
 def test_decisions_default_excludes_history_but_search_can_find_full_archived_context(self):
  v=self.invoke('list_decisions');self.assertEqual(v['total'],3);self.assertEqual(len(v['decisions']),3);self.assertEqual(v['collection_total'],95)
  self.assertNotIn('长背景 '*1000,json.dumps(v));self.assertTrue(v['decisions'][0]['summary_only'])
  found=self.invoke('list_decisions',{'status':'all','query':'hiddenneedle','limit':1});self.assertEqual(found['total'],1);self.assertEqual(found['decisions'][0]['id'],'3')
  exact=self.invoke('read_tool_result',{'result_id':found['result_id'],'path':found['decisions'][0]['detail_path']+'/context','query':'hiddenneedle'})
  self.assertEqual(exact['total'],1);self.assertIn('match_offset',exact['matches'][0]);self.assertTrue(exact['cached']);self.assertIsInstance(exact['snapshot_at'],int)
 def test_stable_paging_visits_every_entry_once_without_refetching_or_nested_collections(self):
  first=self.invoke('list_sessions');self.assertNotIn('tasks',first);self.assertNotIn('goals',first);self.assertNotIn('assignments',first);self.assertNotIn('program',first['sessions'][0])
  self.data['list_sessions']['sessions'].insert(0,{'address':'new','title':'New row'})
  seen=[r['address'] for r in first['sessions']];offset=first['next_offset']
  while offset is not None:
   page=self.invoke('list_sessions',{'offset':offset,'result_id':first['result_id']});seen += [r['address'] for r in page['sessions']];offset=page['next_offset']
  self.assertEqual(seen,[f't{i}' for i in range(35)]);self.assertEqual(len(self.calls),1)
  tasks=self.invoke('list_sessions',{'collection':'tasks','query':'ledger'});self.assertEqual(tasks['tasks'][0]['id'],'task')
 def test_all_list_tools_expose_search_and_pages_without_unbounded_arrays(self):
  for name in q.LISTS:
   if name=='inspect_sessions':continue
   v=self.invoke(name,{'limit':2});self.assertLessEqual(v['returned'],2);self.assertIn('next_offset',v);self.assertIn('result_id',v)
  found=self.invoke('search_memories',{'query':'中文 target'});self.assertEqual(found['total'],1);self.assertEqual(found['memories'][0]['id'],'123')
  self.assertNotIn('review_memories',[n for n,a in self.calls[:-3]])
 def test_full_unicode_text_roundtrips_and_dictionary_fields_are_pageable(self):
  v=self.invoke('list_decisions');path=v['decisions'][0]['detail_path']+'/context';text='';offset=0
  while offset is not None:
   page=self.invoke('read_tool_result',{'result_id':v['result_id'],'path':path,'offset':offset,'limit':401});text+=page['text'];offset=page['next_offset']
  self.assertEqual(text,self.data['list_decisions']['decisions'][92]['context'])
  fields=self.invoke('read_tool_result',{'result_id':v['result_id'],'path':v['decisions'][0]['detail_path'],'query':'context'})
  self.assertEqual(fields['total'],1);self.assertTrue(fields['fields'][0]['detail_path'].endswith('/context'))
 def test_search_history_is_bounded_and_continues_using_source_offsets(self):
  messages=[{'id':str(i),'role':'Claude','text':('needle' if i in [0,160] else 'other')+' 中'*1000} for i in range(200)]
  def raw(name,args):
   self.calls.append((name,args));end=max(0,len(messages)-args['offset']);begin=max(0,end-args['limit'])
   return {'messages':messages[begin:end],'has_more':begin>0,'status':'idle','pending_questions':{'question':{'text':'full question'*1000}}}
  self.raw=raw
  v=self.invoke('read_session',{'address':'t1','query':'needle'});self.assertLessEqual(v['searched_messages'],120);self.assertEqual(v['messages'][0]['id'],'160');self.assertEqual(v['next_offset'],120);self.assertEqual(len(self.calls),3)
  next_page=self.invoke('read_session',{'address':'t1','query':'needle','offset':v['next_offset']});self.assertEqual(next_page['messages'][0]['id'],'0');self.assertIn('pending_questions_detail_path',next_page)
 def test_inspection_selects_one_inventory_page_before_reading_workers(self):
  def raw(name,args):
   self.calls.append((name,args))
   if name=='_inventory':return self.data['list_sessions']
   self.assertLessEqual(len(args['addresses']),2)
   return {'sessions':[{'address':a,'available':True,'messages':[{'text':'X'*100000}]} for a in args['addresses']]}
  self.raw=raw;v=self.invoke('inspect_sessions',{'query':'Worker','offset':4,'limit':2});self.assertEqual([r['address'] for r in v['sessions']],['t4','t5']);self.assertEqual(v['total'],35);self.assertEqual(v['next_offset'],6)
 def test_large_mutation_receipts_preserve_truth_and_details_never_reexecute(self):
  self.data['send_message']={'confirmed':False,'queued':True,'verified':False,'request_id':'original','details':{'payload':'巨'*100000}}
  v=self.invoke('send_message',{'address':'t1','text':'Do it','request_id':'original'});self.assertFalse(v['confirmed']);self.assertFalse(v['verified']);self.assertTrue(v['queued']);self.assertTrue(v['response_shortened'])
  detail=self.invoke('read_tool_result',{'result_id':v['result_id'],'path':'/details/payload','offset':2000,'limit':2000});self.assertEqual(detail['text'],'巨'*2000);self.assertEqual(len(self.calls),1)
  with patch.object(q.Results,'save',return_value=None):v=self.invoke('send_message',{'request_id':'same'})
  self.assertTrue(v['queued']);self.assertFalse(v['confirmed']);self.assertTrue(v['details_unavailable'])
 def test_snapshots_redact_secrets_are_private_expire_and_reject_file_paths(self):
  self.data['get_checkin_settings']={'password':'example-secret-password','url':'https://example.test/oauth/authorize?code=private','text':'sk-examplecredential1234567890'}
  v=self.invoke('get_checkin_settings');p=q.Results(self.home).root/(v['result_id']+'.json');body=p.read_text();self.assertNotIn('example-secret-password',body);self.assertNotIn('code=private',body);self.assertNotIn('sk-examplecredential',body);self.assertEqual(p.stat().st_mode&0o777,0o600)
  for ident in ['../../other',v['result_id']+'/../other']:
   with self.assertRaises(ValueError):self.invoke('read_tool_result',{'result_id':ident})
  os.utime(p,(time.time()-q.CACHE_TTL-1,)*2)
  with self.assertRaisesRegex(ValueError,'never repeat a write'):self.invoke('read_tool_result',{'result_id':v['result_id']})
 def test_text_paging_preserves_cursor_under_json_escaping_budget(self):
  text='\x00'*6000;ident=q.Results(self.home).save('read_session',{'text':text});result='';offset=0
  while offset is not None:
   page=self.invoke('read_tool_result',{'result_id':ident,'path':'/text','offset':offset});self.assertFalse(page.get('response_shortened',False));result+=page['text'];offset=page['next_offset']
  self.assertEqual(result,text)
 def test_invalid_paging_and_cross_tool_snapshots_are_refused(self):
  for args in [{'limit':0},{'limit':26},{'offset':-1},{'offset':True},{'query':'x'*301}]:
   with self.assertRaises(ValueError):self.invoke('list_decisions',args)
  v=self.invoke('list_decisions')
  with self.assertRaises(ValueError):self.invoke('list_goals',{'result_id':v['result_id']})
