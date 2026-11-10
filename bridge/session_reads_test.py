import json
import pathlib
import subprocess
import tempfile
import threading
import time
import unittest
from unittest.mock import patch
import session_reads as m


class Reads(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.home = pathlib.Path(self.temp.name)
        self.redact = lambda v: v
        self.app = lambda *a: self.fail('GUI/network read was not needed')

    def test_legacy_attach_reads_original_pane_without_scrolling(self):
        row = {'address': 't2', 'status': 'working'}
        snap = {'sessions': [row], 'core': 'hc', 'at':m.now()}
        calls = []
        def run(args):
            calls.append(args)
            if args[0] == 'hc':
                self.assertEqual(args[1], 'get')
                return json.dumps({'pid': 12, 'created_ms': 100, 'foreground': {'argv': ['/bin/herdr','--session','work','agent','attach','w1:p1','--takeover']}})
            if 'read' in args:
                self.assertIn('visible', args)
                self.assertNotIn('--lines', args)
                return 'Approval required: inspect this change'
            return json.dumps({'result': {'agent': {'agent_status': 'blocked'}}})
        with patch.object(m, 'command', side_effect=run):
            r = m.read_session(self.home, snap, {'address': 't2'}, self.app, self.redact)
        self.assertEqual(r['source'], 'herdr_visible')
        self.assertEqual(r['execution_state'], 'blocked')
        self.assertIn('Approval required', r['text'])
        self.assertTrue(r['partial'])
        self.assertTrue(all('w1:p1' in c for c in calls[1:]))

    def test_disconnected_native_history_is_readable_without_gui_and_preserved(self):
        ident = 'a' * 32
        p = self.home / 'data/forge-chat' / ident / 'events.jsonl'
        p.parent.mkdir(parents=True)
        events = [
            {'type':'user','id':'u','text':'my task','time':10},
            {'type':'delta','id':'a','text':'ready ','time':20},
            {'type':'delta','id':'a','text':'for review','time':21},
            {'type':'permission','id':'q','name':'AskUserQuestion','input':{'question':'A or B?'},'time':22},
        ]
        data = b'\n'.join(json.dumps(e).encode() for e in events[:2]) + b'\ninvalid record\n' + b'\n'.join(json.dumps(e).encode() for e in events[2:]) + b'\n{"partial":'
        p.write_bytes(data)
        row = {'address':'chat:p','conversation':ident,'detail':'connecting','status':'idle'}
        r = m.read_session(self.home, {'sessions':[row]}, {'address':'chat:p'}, self.app, self.redact)
        self.assertTrue(r['available'])
        self.assertEqual(r['messages'][-1]['text'], 'ready for review')
        self.assertEqual(r['content_updated_at'], 21)
        self.assertEqual(r['execution_state'], 'unknown')
        self.assertTrue(r['last_known_only'])
        self.assertEqual(r['malformed_records_skipped'], 1)
        self.assertEqual(r['pending_permissions']['q']['name'], 'AskUserQuestion')
        self.assertEqual(p.read_bytes(), data)

    def test_native_steering_receipts_are_available_without_reading_the_terminal(self):
        ident='b'*32;p=self.home/'data/forge-chat'/ident/'events.jsonl';p.parent.mkdir(parents=True)
        events=[{'type':'state','status':'working','active_turn':'original'}, {'type':'user','id':'s','text':'change','mode':'steer'}, {'type':'dispatch_receipt','id':'s','mode':'steer','stage':'submitted'}, {'type':'prompt_confirmed','id':'s','mode':'steer'}]
        p.write_text(''.join(json.dumps(e)+'\n' for e in events))
        r=m.local_native(self.home,{'conversation':ident},0,4)
        self.assertEqual(r['dispatches']['s']['stage'],'confirmed');self.assertEqual(r['active_turn'],'original')

    def test_timeout_falls_back_but_never_reuses_another_occupants_cache(self):
        row = {'address':'t1','read_identity':{'pid':1},'read_source':{'kind':'core'},'status':'working'}
        m.save(self.home,row,{'text':'previous evidence','captured_at':100,'source':'terminal_screen'},self.redact)
        with patch.object(m,'command',side_effect=subprocess.TimeoutExpired('hc',2.5)):
            r = m.read_session(self.home,{'sessions':[row],'core':'hc'},{'address':'t1'},self.app,self.redact)
            self.assertTrue(r['cached'])
            self.assertEqual(r['text'],'previous evidence')
            self.assertEqual(r['execution_state'],'unknown')
            fresh = {**row,'read_identity':{'pid':2}}
            r = m.read_session(self.home,{'sessions':[fresh],'core':'hc'},{'address':'t1'},self.app,self.redact)
            self.assertFalse(r['available'])
            self.assertNotIn('text',r)

    def test_batch_is_concurrent_and_one_failure_does_not_hide_other_results(self):
        rows = [{'address':f't{i}','read_source':{'kind':'core'},'read_identity':i+1} for i in range(8)]
        active, peak = 0, 0
        lock = threading.Lock()
        def run(args):
            nonlocal active, peak
            with lock:
                active += 1
                peak = max(active, peak)
            try:
                time.sleep(.02)
                if args[-1] == 't3':
                    raise subprocess.TimeoutExpired('hc',2.5)
                return json.dumps({'text':'actual progress','complete':True})
            finally:
                with lock:
                    active -= 1
        with patch.object(m,'command',side_effect=run):
            r=m.inspect_sessions(self.home,{'sessions':rows,'core':'hc'},{},self.app,self.redact)
        self.assertGreater(peak,1)
        self.assertEqual((r['available'],r['unavailable']),(7,1))
        self.assertEqual(len(r['sessions']),8)

    def test_large_unicode_excerpts_are_bounded_and_explicitly_partial(self):
        r=m.bounded({'messages':[{'id':str(i),'text':'中文'*20000} for i in range(4)]},12000)
        self.assertLess(len(json.dumps(r,ensure_ascii=False).encode()),13000)
        self.assertTrue(r['excerpt_truncated'])
        self.assertTrue(r['partial'])
        self.assertEqual(r['messages'][-1]['id'],'3')

    def test_codex_uses_structured_pagination_and_complete_questions_without_terminal_reads(self):
        row={'address':'t2','app':'codex','read_identity':{'pid':1},'status':'working'}
        p=self.home/'data/super-agent/codex-workers.json';p.parent.mkdir(parents=True);p.write_text(json.dumps({'sessions':{}}))
        data={'read_identity':{'pid':1},'at':m.now(),'binding_live':True,'connected':True,'available':True,'messages':[{'id':'older','text':'Structured history'}],'pending_questions':{'q':{'input':{'questions':[{'question':'Full body?'}]}}},'history_paginated':True,'next_offset':41,'has_more':True,'execution_state':'blocked'}
        with patch.object(m,'codex_control',return_value=data) as rpc,patch.object(m,'command',side_effect=AssertionError('No terminal reads')):
            r=m.read_session(self.home,{'sessions':[row],'at':m.now()},{'address':'t2','offset':40,'limit':1},self.app,self.redact)
        self.assertEqual(r['messages'][0]['id'],'older');self.assertEqual(r['next_offset'],41)
        self.assertEqual(r['pending_questions']['q']['input']['questions'][0]['question'],'Full body?')
        self.assertEqual(r['execution_state'],'blocked');rpc.assert_called_once_with('read',{'address':'t2','offset':40,'limit':1})

    def test_shell_text_is_not_a_legacy_source(self):
        self.assertIsNone(m.attach_source({'argv':['sh','-c','herdr agent attach w1:p1']}))
        self.assertIsNone(m.attach_source({'argv':['herdr','agent','attach','w1:p1'],'foreground':{'argv':['zsh']}}))


if __name__ == '__main__':
    unittest.main()
