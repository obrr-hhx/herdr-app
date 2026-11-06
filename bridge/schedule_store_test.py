import concurrent.futures
import datetime as dt
import pathlib
import tempfile
import unittest
from unittest.mock import patch
import schedule_store as m


class Reminders(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup)
        self.home=pathlib.Path(self.tmp.name)
        self.intent={'id':'human-request'}
        self.args={'request_id':'reminder-test-01','title':'Review #352','text':'Ask for approval; /apply is separate.','at':'2026-11-06T09:00:00+08:00','timezone':'Asia/Shanghai','repeat':'none','user_intent_id':self.intent['id']}
        self.due=int(dt.datetime.fromisoformat(self.args['at']).timestamp()*1000)
        self.clock=self.due-10000

    def create(self,**kw):
        return m.create(self.home,{**self.args,**kw},self.intent,self.clock)

    def test_exact_timezone_and_due_boundary_survive_restart(self):
        r=self.create();self.assertEqual(r['next_local'],self.args['at'])
        self.assertEqual(m.poll(self.home,clock=self.due-1,notify=lambda *_:True)['deliveries'],[])
        sent=[]
        r=m.poll(self.home,clock=self.due,notify=lambda *v:sent.append(v) or True)
        self.assertEqual(len(r['deliveries']),1)
        ident=r['deliveries'][0]['delivery_id']
        self.assertEqual(m.poll(self.home,[ident],clock=self.due+100,notify=lambda *v:sent.append(v) or True)['deliveries'],[])
        self.assertEqual(len(sent),1)
        state=m.listing(self.home)
        self.assertEqual(state['schedules'][0]['state'],'fired')
        self.assertEqual(len(state['unread_reminders']),1)

    def test_idempotency_even_after_deadline_and_no_silent_overwrite(self):
        first=self.create()
        again=m.create(self.home,self.args,self.intent,self.due+100000)
        self.assertEqual(first,again)
        with self.assertRaises(ValueError):self.create(text='different work')
        self.assertEqual(len(m.listing(self.home)['schedules']),1)

    def test_naive_past_and_unrelated_authority_are_refused(self):
        for change in [{'at':'2026-11-06T09:00:00'},{'at':'2026-11-01T09:00:00+08:00'},{'user_intent_id':'background-event'}]:
            with self.assertRaises(ValueError):self.create(**change)
        self.assertEqual(m.listing(self.home)['schedules'],[])

    def test_two_clocks_create_and_notify_one_occurrence(self):
        self.create();sent=[]
        # Initialize tables before racing independent connections.
        def tick(_):return m.poll(self.home,clock=self.due,notify=lambda *v:sent.append(v) or True)
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:list(pool.map(tick,range(2)))
        self.assertEqual(len(m.listing(self.home)['unread_reminders']),1)
        self.assertEqual(len(sent),1)

    def test_offline_catchup_does_not_flood_missed_daily_occurrences(self):
        self.create(repeat='daily')
        r=m.poll(self.home,clock=self.due+10*86400000+1000,notify=lambda *_:False)
        self.assertEqual(len(r['deliveries']),1)
        self.assertEqual(r['deliveries'][0]['due_at'],self.due)
        job=m.listing(self.home)['schedules'][0]
        self.assertEqual(job['next_local'],'2026-11-17T09:00:00+08:00')
        self.assertEqual(job['state'],'pending')

    def test_daily_reminders_keep_wall_time_across_daylight_saving(self):
        p={'at':'2026-03-07T02:30:00-05:00','timezone':'America/New_York','repeat':'daily'}
        due=int(dt.datetime.fromisoformat(p['at']).timestamp()*1000)
        next_at=m.next_occurrence(p,due,due)
        # Missing 02:30 runs after the DST jump; the next day returns to 02:30.
        next_next=m.next_occurrence(p,next_at,next_at)
        from zoneinfo import ZoneInfo
        self.assertEqual(dt.datetime.fromtimestamp(next_next/1000,ZoneInfo(p['timezone'])).isoformat(),'2026-03-09T02:30:00-04:00')

    def test_cancel_prevents_future_delivery_and_dismisses_existing(self):
        self.create();m.cancel(self.home,self.args['request_id'])
        self.assertEqual(m.poll(self.home,clock=self.due,notify=lambda *_:self.fail('cancelled job fired'))['deliveries'],[])
        self.create(request_id='second-reminder')
        m.poll(self.home,clock=self.due,notify=lambda *_:True)
        m.cancel(self.home,'second-reminder')
        self.assertEqual(m.listing(self.home)['unread_reminders'],[])

    def test_unread_reminder_requires_explicit_acknowledgment(self):
        self.create();r=m.poll(self.home,clock=self.due,notify=lambda *_:True)
        ident=r['deliveries'][0]['delivery_id']
        for _ in range(3):self.assertEqual(len(m.listing(self.home)['unread_reminders']),1)
        m.acknowledge(self.home,[ident],self.due+500)
        r=m.poll(self.home,[ident],clock=self.due+1000,notify=lambda *_:True)
        self.assertEqual(r['deliveries'],[]);self.assertEqual(r['acknowledged'],[ident])
        self.assertEqual(m.listing(self.home)['unread_reminders'],[])

    def test_notification_failure_never_loses_the_in_app_reminder(self):
        self.create();sent=[]
        for offset in [0,100,300001]:m.poll(self.home,clock=self.due+offset,notify=lambda *v:sent.append(v) or False)
        self.assertEqual(len(sent),2)
        self.assertEqual(len(m.listing(self.home)['unread_reminders']),1)

    def test_model_cannot_acknowledge_using_the_original_schedule_request(self):
        self.create();r=m.poll(self.home,clock=self.due,notify=lambda *_:True)
        ident=r['deliveries'][0]['delivery_id']
        with self.assertRaises(ValueError):m.acknowledge_authorized(self.home,ident,{'id':'human-request','time':self.due+2000})
        with self.assertRaises(ValueError):m.acknowledge_authorized(self.home,ident,{'id':'old-user','time':self.due-1000})
        m.acknowledge_authorized(self.home,ident,{'id':'new-human-ack','time':self.due+2000})
        self.assertEqual(m.listing(self.home)['unread_reminders'],[])

    def test_recurring_delivery_displays_its_actual_occurrence_time(self):
        self.create(repeat='daily')
        first=m.poll(self.home,clock=self.due,notify=lambda *_:True)['deliveries'][0]
        second=m.poll(self.home,[first['delivery_id']],clock=self.due+86400000,notify=lambda *_:True)['deliveries'][0]
        self.assertEqual(second['at'],'2026-11-07T09:00:00+08:00')

    def test_isolated_test_homes_never_install_real_login_items(self):
        with patch.object(m.subprocess,'run',side_effect=AssertionError('must not touch launchd')):
            self.assertFalse(m.install_clock(self.home)['background_login_clock'])


if __name__=='__main__':unittest.main()
