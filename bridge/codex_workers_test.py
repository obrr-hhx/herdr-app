import json
import pathlib
import tempfile
import unittest
from unittest.mock import patch
import codex_workers as m

class Workers(unittest.TestCase):
    def question(self,**params):
        return m.question_record({'id':42,'method':'item/tool/requestUserInput','params':{'threadId':'thread','turnId':'turn','itemId':'call','questions':[{'id':'route','question':'Which route?','options':[{'label':'B','description':'Exact description'}]}],**params}}, {})
    def test_binding_requires_exact_live_identity(self):
        threads={'one':{'id':'one','name':'Task','cwd':'/tmp'}}
        self.assertEqual(m.bind({'name':'Task','cwd':'/tmp'},threads),('one','unique_live_name_and_cwd'))
        self.assertEqual(m.bind({'thread_id':'one'},threads)[1],'explicit_session_identity')
        with self.assertRaises(ValueError):m.bind({'name':'Task','cwd':'/other'},threads)
        with self.assertRaises(ValueError):m.bind({'name':'Task','cwd':'/tmp'},{**threads,'two':{'id':'two','name':'Task','cwd':'/tmp'}})
        with self.assertRaises(ValueError):m.bind({'thread_id':'missing'},threads)
    def test_structured_history_unwraps_retains_exit_code_excludes_reasoning(self):
        entry={'turnId':'turn','completedAtMs':10,'item':{'id':'cmd','type':'commandExecution','command':'false','exitCode':2,'status':'completed','aggregatedOutput':'real result'}}
        result=m.history_item(entry)
        self.assertEqual((result['turn_id'],result['exit_code']),('turn',2))
        self.assertIn('real result',result['text'])
        self.assertIsNone(m.history_item({'id':'private','type':'reasoning','text':'not for transcript'}))
    def test_full_question_is_stable_and_secrets_are_not_answerable(self):
        q=self.question();self.assertEqual(q['input']['questions'][0]['options'][0]['description'],'Exact description')
        self.assertEqual(q['id'],self.question()['id'])
        self.assertNotEqual(q['id'],self.question(turnId='other')['id'])
        self.assertFalse(self.question(questions=[{'id':'password','isSecret':True}])['reply_supported'])
        self.assertFalse(self.question(questions=[])['reply_supported'])
    def fixture(self):
        temp=tempfile.TemporaryDirectory();self.addCleanup(temp.cleanup);bridge=m.Bridge(temp.name,'unused');q=self.question();bridge.pending[q['id']]=q
        bridge.rows['t2']={'address':'t2','read_source':{'kind':'codex_thread','thread_id':'thread'}};bridge.core='unused'
        bridge.threads={'thread':{'id':'thread'}};bridge.bound['t2']={'thread_id':'thread','view':{'proof':'thread'}}
        class Client:
            closed=False
            def send(self,value):self.sent=value;bridge.pending.pop(q['id']);bridge.resolved.add(q['id'])
        bridge.client=Client();bridge.publish=lambda:None
        return bridge,q
    def test_ui_answer_resolves_exact_request_and_is_idempotent(self):
        bridge,q=self.fixture();args={'address':'t2','question_id':q['id'],'request_id':'human-1','ui':True,'decision':'allow','answers':{'Which route?':'B'}}
        self.assertTrue(bridge.answer(args)['request_resolved']);self.assertEqual(bridge.client.sent,{'id':42,'result':{'answers':{'route':{'answers':['B']}}}})
        self.assertEqual(bridge.answer(args)['question_id'],q['id'])
        with self.assertRaises(ValueError):bridge.answer({**args,'request_id':'other'})
    def test_reused_pane_and_older_text_reply_send_nothing(self):
        bridge,q=self.fixture();args={'address':'t2','question_id':q['id'],'intent':{'id':'human','time':q['time']-1,'text':'B'}}
        with self.assertRaises(ValueError):bridge.answer(args)
        with patch.object(m,'source_view',return_value={'thread_id':'thread','proof':'new-process'}):
            with self.assertRaises(ValueError):bridge.answer({**args,'intent':{**args['intent'],'time':q['time']+1}})
        self.assertFalse(hasattr(bridge.client,'sent'))
    def test_text_cannot_approve_and_all_questions_need_answers(self):
        bridge,q=self.fixture();q['requires_explicit_ui']=True
        with self.assertRaises(ValueError):bridge.answer({'address':'t2','question_id':q['id'],'intent':{'id':'human','time':q['time']+1,'text':'approve'}})
        q['requires_explicit_ui']=False
        with self.assertRaises(ValueError):bridge.answer({'address':'t2','question_id':q['id'],'ui':True,'request_id':'human','decision':'allow','answers':{}})
    def test_dispatch_uses_original_thread_preserves_settings_and_is_idempotent(self):
        bridge,q=self.fixture();bridge.pending.clear();bridge.rows['t2']['read_identity']={'pid':1}
        calls=[]
        def rpc(method,args,**kwargs):
            calls.append((method,args))
            return {'thread':{'status':{'type':'idle'}}} if method=='thread/read' else {'turn':{'id':'next-turn'}}
        bridge.client.rpc=rpc
        args={'address':'t2','read_identity':{'pid':1},'text':'Continue original context','request_id':'dispatch-1'}
        result=bridge.send_message(args);self.assertEqual(result['thread_id'],'thread');self.assertEqual(result['turn_id'],'next-turn')
        self.assertEqual(calls[-1],('turn/start',{'threadId':'thread','input':[{'type':'text','text':'Continue original context'}],'clientUserMessageId':'dispatch-1'}))
        self.assertEqual(bridge.send_message(args),result);self.assertEqual(len(calls),2)
    def test_dispatch_busy_source_change_and_uncertain_wire_never_duplicate(self):
        bridge,q=self.fixture();bridge.pending.clear();bridge.rows['t2']['read_identity']={'pid':1}
        args={'address':'t2','read_identity':{'pid':1},'text':'Continue','request_id':'dispatch-2'}
        with patch.object(m,'source_view',return_value={'thread_id':'thread','proof':'reused'}):
            with self.assertRaises(m.DispatchRefused):bridge.send_message(args)
        def rpc(method,*a,**kw):
            if method=='thread/read':return {'thread':{'status':{'type':'idle'}}}
            raise TimeoutError('Wire response lost')
        bridge.client.rpc=rpc
        with self.assertRaises(TimeoutError):bridge.send_message(args)
        with self.assertRaisesRegex(ValueError,'delivery unknown'):bridge.send_message(args)

    def test_busy_followup_is_durable_fifo_and_repeated_clicks_do_not_resubmit(self):
        bridge,q=self.fixture();bridge.pending.clear();calls=[];active=[True]
        def rpc(method,args,**kw):
            calls.append((method,args))
            if method=='thread/read':return {'thread':{'status':{'type':'active' if active[0] else 'idle'}}}
            active[0]=True;return {'turn':{'id':'next-turn'}}
        bridge.client.rpc=rpc
        a={'address':'t2','text':'Next task','request_id':'queue-first'}
        self.assertTrue(bridge.send_message(a)['queued']);self.assertFalse(bridge.send_message(a)['confirmed'])
        bridge.send_message({**a,'request_id':'queue-second','text':'Then another task'})
        self.assertFalse(any(m=='turn/start' for m,p in calls))
        # A replacement bridge process recovers only queued intent, not unknown writes.
        recovered=m.Bridge(bridge.home,'unused')
        for k in ['client','rows','threads','bound','core']:setattr(recovered,k,getattr(bridge,k))
        recovered.publish=lambda:None
        active[0]=False;recovered.drain_followups()
        starts=[p for m,p in calls if m=='turn/start'];self.assertEqual([p['clientUserMessageId'] for p in starts],['queue-first'])
        self.assertTrue(recovered.send_message(a)['confirmed'])
        active[0]=False;recovered.drain_followups()
        self.assertEqual([p['clientUserMessageId'] for m,p in calls if m=='turn/start'],['queue-first','queue-second'])
        recovered.drain_followups();self.assertEqual(len([m for m,p in calls if m=='turn/start']),2)
    def test_steer_targets_verified_active_turn_without_interrupt_or_settings_override(self):
        bridge,q=self.fixture();bridge.pending.clear();calls=[]
        def rpc(method,args,**kw):
            calls.append((method,args))
            if method=='thread/read':return {'thread':{'status':{'type':'active'}}}
            if method=='thread/turns/list':return {'data':[{'id':'live-turn','status':'inProgress'}]}
            return {'turnId':'live-turn'}
        bridge.client.rpc=rpc
        a={'address':'t2','text':'Keep scope small','request_id':'steer-active','mode':'steer'}
        self.assertTrue(bridge.send_message(a)['confirmed']);self.assertEqual(calls[-1],('turn/steer',{'threadId':'thread','expectedTurnId':'live-turn','clientUserMessageId':'steer-active','input':[{'type':'text','text':'Keep scope small'}]}))
        self.assertEqual(bridge.send_message(a)['mode'],'steer');self.assertEqual(len(calls),3)
    def test_queue_waits_for_decisions_steer_cannot_approve_and_unknown_send_is_not_replayed(self):
        bridge,q=self.fixture();calls=[]
        def rpc(method,args,**kw):
            calls.append(method)
            if method=='thread/read':return {'thread':{'status':{'type':'idle'}}}
            raise TimeoutError('lost acknowledgement')
        bridge.client.rpc=rpc
        a={'address':'t2','text':'Follow-up','request_id':'queue-decision'}
        self.assertTrue(bridge.send_message(a)['queued']);bridge.drain_followups();self.assertNotIn('turn/start',calls)
        with self.assertRaises(m.DispatchRefused):bridge.send_message({**a,'mode':'steer','request_id':'steer-blocked'})
        bridge.pending.clear();bridge.drain_followups();self.assertEqual(calls.count('turn/start'),1)
        bridge.drain_followups();self.assertEqual(calls.count('turn/start'),1)
        with self.assertRaisesRegex(ValueError,'delivery unknown'):bridge.send_message(a)
    def test_queued_followup_never_retargets_reused_session_identity(self):
        bridge,q=self.fixture();bridge.pending.clear()
        bridge.client.rpc=lambda *a,**kw:{'thread':{'status':{'type':'active'}}}
        a={'address':'t2','text':'Follow-up','request_id':'queue-identity'}
        bridge.send_message(a);bridge.bound['t2']['view']['proof']='replacement';bridge.drain_followups()
        result=bridge.send_message(a);self.assertFalse(result['submitted']);self.assertEqual(result['delivery'],'refused')

    def test_plain_text_reply_is_human_verbatim_not_model_rewrite(self):
        bridge,q=self.fixture();bridge.answer({'address':'t2','question_id':q['id'],'intent':{'id':'human','time':q['time']+1,'text':'选 B，保留我的说明。'}})
        self.assertEqual(bridge.client.sent['result']['answers']['route']['answers'],['选 B，保留我的说明。'])

if __name__=='__main__':unittest.main()
