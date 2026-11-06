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
        bridge.client.rpc=lambda *a,**kw:{'thread':{'status':{'type':'active'}}}
        with self.assertRaises(m.DispatchRefused):bridge.send_message(args)
        with patch.object(m,'source_view',return_value={'thread_id':'thread','proof':'reused'}):
            with self.assertRaises(m.DispatchRefused):bridge.send_message(args)
        def rpc(method,*a,**kw):
            if method=='thread/read':return {'thread':{'status':{'type':'idle'}}}
            raise TimeoutError('Wire response lost')
        bridge.client.rpc=rpc
        with self.assertRaises(TimeoutError):bridge.send_message(args)
        with self.assertRaisesRegex(ValueError,'delivery unknown'):bridge.send_message(args)

    def test_plain_text_reply_is_human_verbatim_not_model_rewrite(self):
        bridge,q=self.fixture();bridge.answer({'address':'t2','question_id':q['id'],'intent':{'id':'human','time':q['time']+1,'text':'选 B，保留我的说明。'}})
        self.assertEqual(bridge.client.sent['result']['answers']['route']['answers'],['选 B，保留我的说明。'])

if __name__=='__main__':unittest.main()
