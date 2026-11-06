import importlib.util,json,os,pathlib,tempfile,unittest
ROOT=tempfile.TemporaryDirectory(prefix='herdr-super-test-')
os.environ['HERDR_APP_HOME']=ROOT.name
os.environ['HERDR_APP_BIN']='unused-test-binary'
spec=importlib.util.spec_from_file_location('supervisor',pathlib.Path(__file__).with_name('supervisor.py'));m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
class Coordinator(unittest.TestCase):
 def setUp(self):
  if m.DB.exists():m.DB.unlink()
  self.calls=[]
  def app(name,args=None):
   self.calls.append((name,args))
   return {'sessions':[{'address':'chat:p','status':'idle'}]} if name=='supervisor.list' else {'queued':True}
  m.app=app
 def test_only_human_blockers_are_automatically_captured_not_every_result(self):
  task={'id':'task-x','title':'Choose','summary':'Full question','address':'chat:p','mandate':'Authorized work','stage':'awaiting_review'}
  m.call('record_task',task)
  self.assertFalse(any(n=='supervisor.decision' for n,a in self.calls))
  m.call('record_task',{**task,'id':'task-y','stage':'waiting_user'})
  captures=[a for n,a in self.calls if n=='supervisor.decision']
  self.assertEqual(len(captures),1);self.assertEqual(captures[0]['context'],'Full question\n\n')
 def test_old_ledger_snapshots_are_not_blindly_imported_as_new_decisions(self):
  c=m.db()
  tasks=[{'id':'active','title':'Review','summary':'Evidence','stage':'waiting_user','address':'t10','updated_at':123},{'id':'closed','title':'Old','summary':'Old','stage':'awaiting_review','address':'t4'}]
  with c:
   for task in tasks:c.execute('INSERT INTO tasks VALUES (?,?)',(task['id'],json.dumps(task)))
  c.close()
  def app(name,args=None):
   self.calls.append((name,args))
   return {'sessions':[{'address':'t10'}],'closed_sessions':[{'address':'t4'}]} if name=='supervisor.list' else {'status':'pending'}
  m.app=app
  snapshot=m.call('list_sessions',{})
  captures=[a for n,a in self.calls if n=='supervisor.decision']
  self.assertEqual(captures,[]);self.assertEqual(len(snapshot['tasks']),2)
 def test_decision_cards_reject_secrets_unknown_sessions_and_use_app_owned_store(self):
  m.app=lambda name,args=None: {'sessions':[{'address':'t10'}]} if name=='supervisor.list' else self.calls.append((name,args)) or {'status':'pending'}
  a={'id':'question-1','title':'Choose','context':'Full question','address':'t10','options':['A','B']}
  self.assertEqual(m.call('record_decision',a)['status'],'pending')
  self.assertEqual(self.calls[-1],('supervisor.decision',a))
  with self.assertRaises(ValueError):m.call('record_decision',{**a,'address':'unknown'})
 def test_close_uses_app_control_exact_identity_and_verifies_pending_pane_removal(self):
  calls=[];state={'address':'t4','read_identity':{'pid':4,'created_ms':10},'pane':'p4'}
  def app(name,args=None):
   calls.append((name,args))
   if name=='supervisor.close':return {'archived':True,'verified':False,'pending':True,'runtime_stopped':False}
   if len(calls)==1:return {'sessions':[state],'latest_user_intent':{'id':'human-close'}}
   return {'sessions':[{'address':'t10'}],'closed_sessions':[{'address':'t4','request_id':'close-test-01','identity':state['read_identity'],'pane_present':False}]}
  m.app=app
  result=m.call('close_session',{'address':'t4','request_id':'close-test-01','user_intent_id':'human-close'})
  self.assertTrue(result['verified']);self.assertFalse(result['pending'])
  sent=next(a for name,a in calls if name=='supervisor.close')
  self.assertEqual(sent['expected_identity'],state['read_identity']);self.assertTrue(sent['confirmed'])
 def test_close_refuses_stale_authority_unknown_address_and_manager(self):
  m.app=lambda *_:{'sessions':[{'address':'chat:manager','super_agent':True}],'latest_user_intent':{'id':'human'}}
  for address,intent in [('t4','old'),('unknown','human'),('chat:manager','human')]:
   with self.assertRaises(ValueError):m.call('close_session',{'address':address,'request_id':'close-test-02','user_intent_id':intent})
 def test_close_retry_reuses_saved_identity_without_targeting_a_reused_address(self):
  identity={'pid':4,'created_ms':10};calls=[]
  def app(name,args=None):
   calls.append((name,args))
   if name=='supervisor.list':return {'sessions':[],'closed_sessions':[{'address':'t4','request_id':'close-test-03','identity':identity}]}
   return {'verified':True,'pending':False}
  m.app=app
  self.assertTrue(m.call('close_session',{'address':'t4','request_id':'close-test-03','user_intent_id':'earlier'})['verified'])
  self.assertEqual(calls[-1][1]['expected_identity'],identity)
  with self.assertRaises(ValueError):m.call('close_session',{'address':'t10','request_id':'close-test-03','user_intent_id':'earlier'})
 def test_checkin_settings_only_change_on_the_latest_explicit_human_request(self):
  m.app=lambda *_:{'latest_user_intent':{'id':'human-checkin','time':int(__import__('time').time()*1000)}}
  with self.assertRaises(ValueError):m.call('configure_checkins',{'enabled':False,'user_intent_id':'old'})
  result=m.call('configure_checkins',{'enabled':False,'idle_minutes':10,'interval_minutes':20,'user_intent_id':'human-checkin'})
  self.assertFalse(result['enabled']);self.assertEqual(m.call('get_checkin_settings',{})['interval_minutes'],20)
 def test_native_retry_is_delivered_once_and_request_cannot_change(self):
  a={'address':'chat:p','text':'authorized work','request_id':'assignment_001'}
  self.assertEqual(m.call('send_message',a),m.call('send_message',a))
  self.assertEqual(sum(n=='supervisor.send' for n,a in self.calls),1)
  with self.assertRaises(ValueError):m.call('send_message',{**a,'text':'different work'})
 def test_unknown_delivery_is_not_replayed(self):
  a={'address':'chat:p','text':'work','request_id':'assignment_002'}
  def fail(name,*args):
   if name=='supervisor.list':return {'sessions':[{'address':'chat:p','status':'idle'}]}
   raise TimeoutError('transport broke')
  m.app=fail
  with self.assertRaises(TimeoutError):m.call('send_message',a)
  with self.assertRaisesRegex(ValueError,'Delivery unknown'):m.call('send_message',a)
 def test_pending_review_survives_worker_idle_and_adapter_restart(self):
  a={'id':'task','title':'work','address':'chat:p','mandate':'user asked','stage':'awaiting_review','summary':'ready'}
  m.call('record_task',a)
  self.assertEqual(m.call('list_sessions',{})['tasks'][0]['stage'],'awaiting_review')
  with self.assertRaises(ValueError):m.call('record_task',{**a,'stage':'working'})
  with self.assertRaises(ValueError):m.call('record_task',{**a,'stage':'paused'})
  with self.assertRaises(ValueError):m.call('record_task',{**a,'stage':'accepted'})
  m.app=lambda *_:{'sessions':[],'latest_user_intent':{'id':'human-intent','time':int(__import__('time').time()*1000)+1000}}
  self.assertEqual(m.call('record_task',{**a,'stage':'accepted','user_intent_id':'human-intent'})['stage'],'accepted')
 def test_refused_send_can_be_corrected_without_unknown_delivery(self):
  a={'address':'chat:p','text':'work','request_id':'assignment_004'}
  count=[]
  def refuse(name,args=None):
   if name=='supervisor.list':return {'sessions':[{'address':'chat:p','status':'idle'}]}
   count.append(name)
   if len(count)==1:raise m.Refused('User draft in progress; no message sent')
   return {'queued':True}
  m.app=refuse
  with self.assertRaises(m.Refused):m.call('send_message',a)
  self.assertTrue(m.call('send_message',a)['queued'])
 def test_local_codex_uses_runtime_dispatch_and_never_outer_terminal_send(self):
  from unittest.mock import patch
  row={'address':'t4','app':'codex','status':'working','execution_state':'idle','read_identity':{'pid':1}}
  m.app=lambda *_:{'sessions':[row]}
  args={'address':'t4','text':'Continue the original task','request_id':'codex-route-01'}
  with patch.object(m,'codex_control',return_value={'thread_id':'original','turn_id':'next'}) as rpc,patch.object(m.subprocess,'run',side_effect=AssertionError('Terminal send must not run')):
   self.assertEqual(m.call('send_message',args)['thread_id'],'original')
   rpc.assert_called_once_with('send',{**args,'read_identity':{'pid':1}})
 def test_definite_codex_refusal_can_retry_same_id_but_unknown_delivery_cannot(self):
  from unittest.mock import patch
  m.app=lambda *_:{'sessions':[{'address':'t4','app':'codex','status':'idle'}]}
  args={'address':'t4','text':'Continue','request_id':'codex-refused-01'}
  with patch.object(m,'codex_control',side_effect=[m.DispatchRefused('busy; nothing sent'),{'confirmed':True}]) as rpc:
   with self.assertRaises(m.Refused):m.call('send_message',args)
   self.assertTrue(m.call('send_message',args)['confirmed']);self.assertEqual(rpc.call_count,2)
  args={**args,'request_id':'codex-unknown-01'}
  with patch.object(m,'codex_control',side_effect=TimeoutError('lost confirmation')) as rpc:
   with self.assertRaises(TimeoutError):m.call('send_message',args)
   with self.assertRaisesRegex(ValueError,'Delivery unknown'):m.call('send_message',args)
   self.assertEqual(rpc.call_count,1)
 def test_remote_terminal_keeps_its_existing_transport(self):
  from unittest.mock import patch
  from types import SimpleNamespace
  m.app=lambda *_:{'sessions':[{'address':'vm/t1','app':'codex','status':'idle'}],'core':'hc'}
  args={'address':'vm/t1','text':'Authorized remote work','request_id':'remote-send-01'}
  with patch.object(m,'codex_control',side_effect=AssertionError('Local runtime must not receive a remote terminal assignment')),patch.object(m.subprocess,'run',return_value=SimpleNamespace(returncode=0,stdout='{"accepted":true}')) as run:
   self.assertTrue(m.call('send_message',args)['core_receipt']['accepted']);self.assertEqual(run.call_args.args[0][:5],['hc','send','--json','--if','idle'])
 def test_self_delegation_is_refused(self):
  m.app=lambda *_:{'sessions':[{'address':'chat:manager','super_agent':True}]}
  with self.assertRaisesRegex(ValueError,'yourself'):m.call('send_message',{'address':'chat:manager','text':'loop','request_id':'assignment_003'})
 def test_history_and_ledgers_redact_credentials(self):
  text='Bearer example-token-material-12345678 and sk-examplecredentialmaterial0123456789'
  safe=m.redact({'text':text,'password':'example-private-value','url':'https://example.test/oauth/authorize?code=example'})
  self.assertNotIn('example-token',str(safe));self.assertNotIn('sk-example',str(safe));self.assertEqual(safe['password'],'[credential redacted]');self.assertNotIn('code=',safe['url'])
  with self.assertRaises(ValueError):m.call('send_message',{'address':'chat:p','text':text,'request_id':'credential-test-01'})
 def test_mcp_only_exposes_management_tools(self):
  names=[x['name'] for x in m.handle({'method':'tools/list'})['tools']]
  self.assertEqual(names,['plan_followup','get_checkin_settings','configure_checkins','list_sessions','inspect_sessions','read_session','send_message','close_session','create_session','reply_to_question','schedule_reminder','list_reminders','cancel_reminder','acknowledge_reminder','list_decisions','record_decision','record_task'])
if __name__=='__main__':unittest.main()
