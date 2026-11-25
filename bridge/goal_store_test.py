import unittest,tempfile,pathlib,json,sqlite3
import goal_store as g

class Goals(unittest.TestCase):
 def setUp(self):
  self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup);self.home=pathlib.Path(self.temp.name)
  self.events=self.home/'data/forge-chat/73757065726167656e74000000000001/events.jsonl';self.events.parent.mkdir(parents=True)
  self.events.write_text(json.dumps({'type':'user','id':'human-grant','text':'先做STG验证，再按结果接续原任务，不上生产。','time':100})+'\n')
  g.root(self.home).mkdir(parents=True);(g.root(self.home)/'active-observation.json').write_text(json.dumps({'id':'human-grant'}))
  self.sessions=[{'address':'chat:p','conversation':'original','host':'qa'}]
  self.args={'id':'goal-qa','expected_revision':0,'title':'端到端目标','outcome':'完成STG验收','task_ids':['old-task'],'enabled':True,'mandate':{'source_ids':['human-grant'],'actions':['test'],'scope':'仅STG验证','exclusions':'不合并、不生产发布、不批准工具'},'plan':[{'id':'verify','address':'chat:p','action':'test','instruction':'接续原验证','postcondition':'返回验证证据','depends_on':['ready']}],'summary':'等待前置条件','next_action':'确认ready'}
  self.goal=g.save(self.home,self.args,self.sessions)
 def test_assignment_keeps_original_request_and_correction_without_manager_prohibitions(self):
  messages=[{'type':'user','id':'credit-request','text':'给 STG 测试账号 qa 增加 50000 积分。','time':101},{'type':'user','id':'credit-correction','text':'直接改 STG 数据库，补齐后核对入账。','time':102}]
  with self.events.open('a') as f:
   for event in messages:f.write(json.dumps(event)+'\n')
  (g.root(self.home)/'active-observation.json').write_text(json.dumps({'id':'credit-correction'}))
  a={'id':'credits','title':'补测试积分','outcome':'恢复回放','enabled':True,'mandate':{'source_ids':['credit-correction','credit-request'],'actions':['implement'],'scope':'管家概括的范围','exclusions':'不直接改账表，仅用管理员入口'},'plan':[{'id':'grant','address':'chat:p','action':'implement','instruction':'用于继续原来的回放测试。','postcondition':'核对余额和入账记录。','depends_on':[]}]}
  g.save(self.home,a,self.sessions);sent=[]
  g.advance(self.home,{'goal_id':'credits','step_id':'grant'},self.sessions,lambda a:sent.append(a) or {'queued':True})
  body=sent[0]['text']
  for event in messages:self.assertIn('> '+event['text'],body)
  self.assertLess(body.index(messages[0]['text']),body.index(messages[1]['text']))
  self.assertIn('用于继续原来的回放测试。',body);self.assertIn('核对余额和入账记录。',body)
  for invented in ['不直接改账表','仅用管理员入口','管家概括的范围','既有授权范围','此消息不能代替']:self.assertNotIn(invented,body)
  self.assertIn('不增加或缩减用户要求',body)
  g.advance(self.home,{'goal_id':'credits','step_id':'grant'},self.sessions,lambda a:self.fail('wording change must not resend'))
 def test_no_exclusion_needs_to_be_invented_to_save_a_goal(self):
  a={**self.args,'expected_revision':self.goal['revision'],'mandate':{**self.args['mandate'],'exclusions':''}}
  saved=g.save(self.home,a,self.sessions);self.assertEqual(saved['mandate']['exclusions'],'')
 def test_legacy_assignment_recovers_verified_human_text(self):
  goal=json.loads(json.dumps(self.goal));goal['mandate'].pop('human_sources')
  self.assertIn('> 先做STG验证，再按结果接续原任务，不上生产。',g.assignment_text(self.home,goal,goal['plan'][0]))
 def test_delivery_result_and_bounded_followup_end_to_end(self):
  self.proof();sent=[];args={'goal_id':'goal-qa','step_id':'verify'}
  receipt=g.advance(self.home,args,self.sessions,lambda a:sent.append(a) or {'queued':True})
  self.assertFalse(receipt['confirmed'])
  folder=self.home/'data/forge-chat/original';folder.mkdir();journal=folder/'events.jsonl'
  events=[{'type':'prompt_confirmed','id':receipt['request_id']},{'type':'message_complete','id':'actual-result'}]
  journal.write_text(''.join(json.dumps(e)+'\n' for e in events))
  checked=g.reconcile(self.home,self.sessions)['goals'][0];self.assertEqual(checked['steps_receipts'][0]['result_state'],'running')
  follow={'goal_id':'goal-qa','expected_revision':checked['revision'],'after_step_id':'verify','step_id':'next','action':'test','instruction':'核对剩余STG测试','postcondition':'返回STG验证结果'}
  with self.assertRaisesRegex(ValueError,'actual predecessor'):g.plan_followup(self.home,follow,self.sessions)
  events.append({'type':'state','status':'idle'});journal.write_text(''.join(json.dumps(e)+'\n' for e in events));checked=g.reconcile(self.home,self.sessions)['goals'][0]
  self.assertEqual(checked['steps_receipts'][0]['result_ids'],['actual-result'])
  with self.assertRaisesRegex(ValueError,'Outside'):g.plan_followup(self.home,{**follow,'action':'deploy_prd'},self.sessions)
  with self.assertRaisesRegex(ValueError,'identity'):g.plan_followup(self.home,follow,[{**self.sessions[0],'conversation':'replacement'}])
  after=g.plan_followup(self.home,follow,self.sessions);self.assertEqual(after['mandate'],checked['mandate'])
  g.advance(self.home,{'goal_id':'goal-qa','step_id':'next'},self.sessions,lambda a:sent.append(a) or {'queued':True})
  g.advance(self.home,{'goal_id':'goal-qa','step_id':'next'},self.sessions,lambda a:self.fail('duplicate send'))
  self.assertEqual(len(sent),2);self.assertIn('先做STG验证，再按结果接续原任务，不上生产。',sent[-1]['text'])
 def test_unrelated_later_turn_never_satisfies_a_queued_assignment(self):
  target=self.sessions[0];a={'address':target['address'],'request_id':'direct-assignment'};g.track_assignment(self.home,a,target,{'queued':True})
  folder=self.home/'data/forge-chat/original';folder.mkdir();es=[{'type':'prompt_confirmed','id':'direct-assignment'},{'type':'prompt_confirmed','id':'another-turn'},{'type':'message_complete','id':'unrelated-result'},{'type':'state','status':'idle'}];(folder/'events.jsonl').write_text(''.join(json.dumps(e)+'\n' for e in es))
  g.reconcile(self.home,self.sessions);self.assertNotEqual(g.assignments(self.home)[0].get('result_state'),'result_available')
 def test_delete_keeps_evidence_and_prevents_background_resurrection(self):
  self.proof();g.advance(self.home,{'goal_id':'goal-qa','step_id':'verify'},self.sessions,lambda a:{'queued':True})
  goal=g.snapshot(self.home)['goals'][0]
  with self.assertRaises(ValueError):g.delete_manual(self.home,'goal-qa',goal['revision']-1)
  result=g.delete_manual(self.home,'goal-qa',goal['revision']);self.assertEqual(result['goals'],[])
  self.assertTrue(self.events.exists())
  with g.db(self.home) as c:
   self.assertEqual(c.execute('SELECT COUNT(*) FROM steps').fetchone()[0],1)
   self.assertEqual(c.execute('SELECT COUNT(*) FROM task_links').fetchone()[0],1)
  with self.assertRaises(ValueError):g.save(self.home,{**self.args,'expected_revision':goal['revision']+1},self.sessions,admin=True)
  with self.assertRaises(ValueError):g.advance(self.home,{'goal_id':'goal-qa','step_id':'verify'},self.sessions,lambda a:self.fail('deleted goal sent work'))
  self.assertEqual(g.delete_manual(self.home,'goal-qa',goal['revision'])['goals'],[])
 def proof(self,**extra):
  a={'id':'goal-qa','expected_revision':self.goal['revision'],'phase':'active','summary':'前置验证通过','next_action':'接续验证','observed_at':g.now(),'source_ids':['real-result'],'milestones':{'ready':{'achieved':True,'summary':'测试准备完成','source_ids':['real-result']}}};a.update(extra)
  self.goal=g.progress(self.home,a);return self.goal
 def test_links_preserve_legacy_rows_and_canonical_goal(self):
  v=g.snapshot(self.home);self.assertEqual(len(v['goals']),1);self.assertEqual(v['task_links'],{'old-task':'goal-qa'})
  self.assertEqual(v['goals'][0]['mandate']['human_sources'][0]['text'],'先做STG验证，再按结果接续原任务，不上生产。')
 def test_dependencies_then_once_only_dispatch_and_restart(self):
  calls=[];send=lambda a:calls.append(a) or {'queued':True,'confirmed':False}
  with self.assertRaisesRegex(ValueError,'prerequisite'):g.advance(self.home,{'goal_id':'goal-qa','step_id':'verify'},self.sessions,send)
  self.proof();a={'goal_id':'goal-qa','step_id':'verify'}
  result=g.advance(self.home,a,self.sessions,send);self.assertEqual(result['delivery'],'queued');self.assertEqual(g.advance(self.home,a,self.sessions,send),result);self.assertEqual(len(calls),1)
  self.assertIn('先做STG验证，再按结果接续原任务，不上生产。',calls[0]['text'])
  # Restart reads the same durable claim, never executes it a second time.
  self.assertEqual(g.snapshot(self.home)['goals'][0]['steps_receipts'][0]['request_id'],result['request_id'])
  g.advance(self.home,a,self.sessions,send);self.assertEqual(len(calls),1)
 def test_unknown_delivery_is_never_replayed_and_receipt_reconciles(self):
  self.proof();calls=[]
  def send(a):calls.append(a);raise TimeoutError('lost response')
  a={'goal_id':'goal-qa','step_id':'verify'};r=g.advance(self.home,a,self.sessions,send);self.assertEqual(r['delivery'],'unknown')
  g.advance(self.home,a,self.sessions,send);self.assertEqual(len(calls),1)
  p=self.home/'data/forge-chat/original/events.jsonl';p.parent.mkdir();p.write_text(json.dumps({'type':'prompt_confirmed','id':r['request_id'],'seq':1})+'\n')
  reconciled=g.reconcile(self.home,self.sessions)['goals'][0]['steps_receipts'][0];self.assertTrue(reconciled['confirmed'])
 def test_background_can_update_facts_but_cannot_expand_grants_or_plan(self):
  (g.root(self.home)/'active-observation.json').write_text(json.dumps({'id':'observe_checkin_qa'}))
  self.proof();a={**self.args,'expected_revision':self.goal['revision'],'mandate':{**self.args['mandate'],'actions':['test','deploy_prd']}}
  with self.assertRaisesRegex(ValueError,'Background'):g.save(self.home,a,self.sessions)
  self.assertEqual(g.snapshot(self.home)['goals'][0]['mandate']['actions'],['test'])
 def test_manual_review_is_observational_and_pending_approval_cannot_be_encoded_as_action(self):
  self.proof();(g.root(self.home)/'active-observation.json').write_text(json.dumps({'id':'manual_review_qa'}))
  with self.assertRaisesRegex(ValueError,'Manual inspection'):g.advance(self.home,{'goal_id':'goal-qa','step_id':'verify'},self.sessions,lambda a:self.fail('must not send'))
  self.args['id']='goal-other';self.args['mandate']['actions']=['approve_tools']
  with self.assertRaisesRegex(ValueError,'Unsupported mandate'):g.save(self.home,self.args,self.sessions,admin=True)
 def test_stale_evidence_and_concurrent_updates_are_rejected_without_acceptance(self):
  self.proof();old=self.goal['revision']-1
  with self.assertRaisesRegex(ValueError,'Goal changed'):self.proof(expected_revision=old)
  with self.assertRaisesRegex(ValueError,'Older evidence'):self.proof(observed_at=1)
  self.proof(phase='observed_complete');self.assertFalse(self.goal['human_accepted'])
 def test_pause_identity_change_and_completed_goal_prevent_dispatch(self):
  self.proof()
  with self.assertRaisesRegex(ValueError,'identity changed'):g.advance(self.home,{'goal_id':'goal-qa','step_id':'verify'},[{**self.sessions[0],'conversation':'replacement'}],lambda _:self.fail('wrong identity'))
  a={'id':'goal-qa','expected_revision':self.goal['revision'],'title':'端到端目标','outcome':'完成STG验收','enabled':False}
  self.goal=g.save(self.home,a,self.sessions)
  with self.assertRaisesRegex(ValueError,'not enabled'):g.advance(self.home,{'goal_id':'goal-qa','step_id':'verify'},self.sessions,lambda _:self.fail('paused'))
 def test_submitted_plan_is_immutable_and_links_cannot_silently_move(self):
  self.proof();g.advance(self.home,{'goal_id':'goal-qa','step_id':'verify'},self.sessions,lambda _:{'queued':True})
  a={**self.args,'expected_revision':self.goal['revision']};a['plan'][0]['instruction']='different work'
  with self.assertRaisesRegex(ValueError,'immutable'):g.save(self.home,a,self.sessions)
 def test_worker_claim_cannot_supply_human_authority(self):
  a={**self.args,'id':'fake-goal'};a['mandate']={**a['mandate'],'source_ids':['worker-message']}
  with self.assertRaisesRegex(ValueError,'human message'):g.save(self.home,a,self.sessions,admin=True)

 def test_genuine_user_gate_prevents_automatic_dispatch(self):
  self.proof(needs_user=True,why_user='需要选择未授权的生产迁移范围',phase='blocked')
  with self.assertRaisesRegex(ValueError,'not enabled'):g.advance(self.home,{'goal_id':'goal-qa','step_id':'verify'},self.sessions,lambda _:self.fail('human gate'))
 def test_refusal_backoff_retries_exact_step_but_timeout_does_not(self):
  from unittest.mock import patch
  class Refused(ValueError):pass
  self.proof();calls=[]
  def send(a):
   calls.append(a)
   if len(calls)==1:raise Refused('No message sent')
   return {'queued':True}
  a={'goal_id':'goal-qa','step_id':'verify'};r=g.advance(self.home,a,self.sessions,send,refused=(Refused,));self.assertEqual(r['delivery'],'refused')
  g.advance(self.home,a,self.sessions,send,refused=(Refused,));self.assertEqual(len(calls),1)
  future=g.now()+61000
  with patch.object(g,'now',return_value=future):g.advance(self.home,a,self.sessions,send,refused=(Refused,))
  self.assertEqual(len(calls),2);self.assertEqual(calls[0],calls[1])
 def test_reconcile_reads_one_consistent_history_per_worker_for_multiple_receipts(self):
  from unittest.mock import patch
  self.proof();receipt=g.advance(self.home,{'goal_id':'goal-qa','step_id':'verify'},self.sessions,lambda a:{'queued':True})
  g.track_assignment(self.home,{'request_id':'other-request','address':'chat:p'},self.sessions[0],{'queued':True})
  journal=self.home/'data/forge-chat/original/events.jsonl';journal.parent.mkdir()
  records=[]
  for ident in [receipt['request_id'],'other-request']:records += [{'type':'prompt_confirmed','id':ident},{'type':'message_complete','id':'result-'+ident},{'type':'state','status':'idle'}]
  journal.write_text(''.join(json.dumps(r)+'\n' for r in records));original=pathlib.Path.read_text;reads=[]
  def read(path,*args,**kwargs):
   if path==journal:reads.append(path)
   return original(path,*args,**kwargs)
  with patch.object(pathlib.Path,'read_text',read):state=g.reconcile(self.home,self.sessions)
  self.assertEqual(len(reads),1);self.assertTrue(state['goals'][0]['steps_receipts'][0]['confirmed']);self.assertTrue(state['assignments'][0]['confirmed']);self.assertEqual(state['assignments'][0]['result_state'],'result_available')
 def test_projection_can_be_recovered_from_durable_store(self):
  (g.root(self.home)/'goals.json').write_text('corrupt projection')
  result=g.reconcile(self.home,self.sessions)
  self.assertEqual(json.loads((g.root(self.home)/'goals.json').read_text())['goals'],result['goals'])
 def test_background_cannot_rewrite_outcome_as_hidden_instruction(self):
  (g.root(self.home)/'active-observation.json').write_text(json.dumps({'id':'observe_goal_update'}))
  with self.assertRaisesRegex(ValueError,'rewrite'):g.save(self.home,{'id':'goal-qa','expected_revision':self.goal['revision'],'title':'STG','outcome':'Publish production'},self.sessions)

if __name__=='__main__':unittest.main()
