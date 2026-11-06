import pathlib,tempfile,unittest,json
import checkins as m
class Preferences(unittest.TestCase):
 def test_defaults_settings_survive_restart_and_reject_invalid_values(self):
  with tempfile.TemporaryDirectory() as home:
   self.assertEqual(m.load(home)['idle_minutes'],15)
   m.configure(home,{'enabled':False,'idle_minutes':10,'interval_minutes':20})
   self.assertFalse(m.describe(home)['enabled']);self.assertEqual(m.load(home)['interval_minutes'],20)
   for args in [{'idle_minutes':0},{'interval_minutes':1441},{'idle_minutes':True},{'enabled':'yes'}]:
    with self.assertRaises(ValueError):m.configure(home,args)
   self.assertEqual(m.load(home)['interval_minutes'],20)
   m.configure(home,{'idle_minutes':60});self.assertEqual(m.load(home)['idle_minutes'],60)
   self.assertEqual(m.path(home).stat().st_mode&0o777,0o600)
if __name__=='__main__':unittest.main()
