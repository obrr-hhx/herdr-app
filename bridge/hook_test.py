import json,os,pathlib,socket,subprocess,tempfile,threading,time,unittest

@unittest.skipUnless(os.name=='posix','Native Claude hook sockets require Unix')
class HookTimeout(unittest.TestCase):
 def test_startup_reply_survives_disk_delay_longer_than_five_seconds(self):
  with tempfile.TemporaryDirectory(prefix='hslow-',dir='/tmp') as folder:
   server=socket.socket(socket.AF_UNIX);self.addCleanup(server.close);server.bind(folder+'/run.sock');server.listen(1);server.settimeout(10)
   received=[]
   def reply():
    peer,_=server.accept()
    with peer,peer.makefile('rb') as stream:
     received.append(json.loads(stream.readline()));time.sleep(5.2)
     peer.sendall((json.dumps({'output':{'systemMessage':'Startup confirmed after slow storage'}})+'\n').encode())
   thread=threading.Thread(target=reply,daemon=True);thread.start()
   event={'hook_event_name':'SessionStart','session_id':'fixture','cwd':folder}
   result=subprocess.run(['python3',str(pathlib.Path(__file__).with_name('hook.py')),folder],input=json.dumps(event),capture_output=True,text=True,timeout=10)
   thread.join(timeout=1)
   self.assertEqual(result.returncode,0);self.assertEqual(json.loads(result.stdout)['systemMessage'],'Startup confirmed after slow storage')
   self.assertEqual(received[0]['event'],event)
