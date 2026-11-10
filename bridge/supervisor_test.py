import importlib.util,json,os,pathlib,tempfile,unittest
ROOT=tempfile.TemporaryDirectory(prefix='herdr-super-test-')
os.environ['HERDR_APP_HOME']=ROOT.name
os.environ['HERDR_APP_BIN']='unused-test-binary'
spec=importlib.util.spec_from_file_location('supervisor',pathlib.Path(__file__).with_name('supervisor.py'));m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
REAL_APP=m.app
class Coordinator(unittest.TestCase):
 def setUp(self):
  if m.DB.exists():m.DB.unlink()
  self.calls=[]
  def app(name,args=None):
   self.calls.append((name,args))
   return {'sessions':[{'address':'chat:p','status':'idle'}]} if name=='supervisor.list' else {'queued':True}
  m.app=app
 def test_ssh_alias_discovery_follows_includes_without_accepting_wildcards(self):
  with tempfile.TemporaryDirectory() as directory:
   root=pathlib.Path(directory);(root/'Parts').mkdir()
   config=root/'config';config.write_text('Host *\n  ServerAliveInterval 10\nInclude="Parts/*.conf"\nHost "direct-alias" another-alias # comment\n')
   (root/'Parts'/'hosts.conf').write_text('Include config\nHost=env-new\n  HostName 172.30.1.178\nHost !denied prefix-*\nHost blocked !blocked\n')
   for host in ['env-new','direct-alias','another-alias']:self.assertTrue(m.configured_ssh_host(host,config))
   for host in ['unconfigured','prefix-any','denied','blocked','-bad','host;command']:self.assertFalse(m.configured_ssh_host(host,config))
 def test_first_worker_on_configured_ssh_alias_is_created_once(self):
  from unittest.mock import patch
  from subprocess import CompletedProcess
  args={'agent':'claude','host':'env-new','folder':'/home/forge/project','title':'New VM worker','request_id':'first-new-vm-worker'}
  def app(name,args=None):
   self.calls.append((name,args));return {'sessions':[]} if name=='supervisor.list' else {'pane':'new-pane'}
  m.app=app
  with patch.object(m,'configured_ssh_host',return_value=True),patch.object(m.subprocess,'run',return_value=CompletedProcess([],0,'','')) as install:
   self.assertEqual(m.call('create_session',args)['address'],'chat:new-pane')
   self.assertEqual(m.call('create_session',args)['address'],'chat:new-pane')
   self.assertEqual(install.call_count,1)
  self.assertEqual(sum(n=='supervisor.create' for n,a in self.calls),1)
 def test_unknown_host_refusal_does_not_poison_creation_request(self):
  from unittest.mock import patch
  from subprocess import CompletedProcess
  args={'agent':'claude','host':'env-new','folder':'/home/forge/project','title':'New VM worker','request_id':'unknown-host-retry'}
  with patch.object(m,'configured_ssh_host',return_value=False),patch.object(m.subprocess,'run') as install:
   with self.assertRaisesRegex(m.Refused,'Unknown SSH host'):m.call('create_session',args)
   install.assert_not_called()
  c=m.db();self.assertIsNone(c.execute('SELECT id FROM sends WHERE id=?',(args['request_id'],)).fetchone());c.close()
  with patch.object(m,'configured_ssh_host',return_value=True),patch.object(m.subprocess,'run',return_value=CompletedProcess([],0,'','')):
   self.assertEqual(m.call('create_session',args),{'queued':True,'address':None})
 def test_no_ledger_stage_mechanically_creates_a_human_decision(self):
  for stage in ['working','waiting_user','awaiting_review','failed','paused']:
   m.call('record_task',{'id':'task-'+stage,'title':'Task snapshot','summary':'Status is not a decision','address':'chat:p','mandate':'Authorized work','stage':stage})
  self.assertFalse(any(n in ['supervisor.decision','supervisor.decision_curate'] for n,a in self.calls))
  c=m.db();self.assertEqual(c.execute('SELECT COUNT(*) FROM tasks').fetchone()[0],5);c.close()
 def test_curation_is_a_separate_explanatory_observation_not_worker_dispatch(self):
  args={'action':'merge','id':'question','duplicate_ids':['old-task'],'reason':'Same actual choice and original evidence'}
  self.assertEqual(m.call('curate_decision',args),{'queued':True});self.assertEqual(self.calls,[('supervisor.decision_curate',args)])
  with self.assertRaises(ValueError):m.call('curate_decision',{**args,'reason':''})
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
  a={'id':'question-1','title':'Choose','context':'Full question','address':'t10','options':['A','B'],'decision_key':'migration-scope','reason':'The owner must choose between different scopes','source_ids':['msg_question_fixture']}
  with self.assertRaises(ValueError):m.call('record_decision',{k:v for k,v in a.items() if k!='reason'})
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
 def test_cli_refusal_with_exit_one_keeps_reason_and_allows_same_request_retry(self):
  from unittest.mock import patch
  from subprocess import CompletedProcess
  a={'address':'chat:p','text':'original authorized work','request_id':'native-refusal-01'}
  def run(argv,**kwargs):
   if argv[2]=='supervisor.list':return CompletedProcess(argv,0,json.dumps({'sessions':[{'address':'chat:p','status':'idle'}]}),'')
   if not self.calls:
    self.calls.append(argv[2]);return CompletedProcess(argv,1,json.dumps({'error':'Session is not idle; no message sent'}),'')
   return CompletedProcess(argv,0,json.dumps({'queued':True}),'')
  m.app=REAL_APP
  with patch.object(m.subprocess,'run',side_effect=run):
   with self.assertRaisesRegex(m.Refused,'Session is not idle'):m.call('send_message',a)
   self.assertTrue(m.call('send_message',a)['queued'])
 def test_cli_transport_and_window_timeouts_stay_unknown_and_do_not_leak(self):
  from unittest.mock import patch
  from subprocess import CompletedProcess
  m.app=REAL_APP
  for stdout in ['',json.dumps({'error':'the window did not answer'}),json.dumps({'error':'private credential must not be printed'})]:
   with patch.object(m.subprocess,'run',return_value=CompletedProcess([],1,stdout,'private stderr')):
    with self.assertRaises(ValueError) as caught:REAL_APP('supervisor.send',{})
    self.assertNotIsInstance(caught.exception,m.Refused);self.assertNotIn('private',str(caught.exception))
 def test_native_queue_and_explicit_capable_steer_preserve_modes(self):
  m.app=lambda name,args=None:{'sessions':[{'address':'chat:p','status':'working','steer_available':True}]} if name=='supervisor.list' else self.calls.append((name,args)) or {'queued':True,'confirmed':False}
  a={'address':'chat:p','text':'After current work','request_id':'queue-busy-native','mode':'queue'}
  self.assertTrue(m.call('send_message',a)['queued'])
  self.assertEqual(self.calls[0][0],'supervisor.send');self.assertNotIn('mode',self.calls[0][1])
  self.assertTrue(m.call('send_message',{**a,'mode':'steer','request_id':'steer-native-yes'})['queued']);self.assertEqual(next(a for n,a in self.calls if n=='supervisor.send' and a.get('mode')=='steer')['mode'],'steer')
  m.app=lambda *_:{'sessions':[{'address':'chat:p','status':'working','steer_available':False}]}
  with self.assertRaisesRegex(m.Refused,'unavailable'):m.call('send_message',{**a,'mode':'steer','request_id':'steer-native-nope'})
  self.assertEqual(len([n for n,a in self.calls if n=='supervisor.send']),2)
 def test_retry_of_queued_codex_returns_updated_runtime_receipt(self):
  from unittest.mock import patch
  m.app=lambda *_:{'sessions':[{'address':'t4','app':'codex','read_identity':{'pid':1}}]}
  a={'address':'t4','text':'Next work','request_id':'queue-refresh-01'}
  with patch.object(m,'codex_control',side_effect=[{'queued':True,'confirmed':False,'source':'codex_app_server'},{'queued':False,'confirmed':True,'turn_id':'next'}]):
   self.assertTrue(m.call('send_message',a)['queued']);self.assertTrue(m.call('send_message',a)['confirmed'])
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
  self.assertEqual(names,['plan_followup','get_checkin_settings','configure_checkins','list_sessions','inspect_sessions','read_session','send_message','close_session','release_session','create_session','reply_to_question','schedule_reminder','list_reminders','cancel_reminder','acknowledge_reminder','list_decisions','record_decision','curate_decision','record_task','list_goals','record_goal','record_goal_progress','ask_goal_session','advance_goal','search_memories','list_memories','read_memory','review_memories','consolidate_memories','record_memory','forget_memory'])
if __name__=='__main__':unittest.main()
