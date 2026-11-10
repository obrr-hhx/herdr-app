import test from 'node:test';import assert from 'node:assert/strict';import fs from 'node:fs';import path from 'node:path';import net from 'node:net';import {spawn,spawnSync} from 'node:child_process';import {setTimeout as delay} from 'node:timers/promises';import {fileURLToPath} from 'node:url';
const base=path.dirname(fileURLToPath(import.meta.url));
test('interactive Claude receives summary in hook context once per version; text and existing runtime remain intact',async()=>{
 const home=fs.mkdtempSync('/tmp/hmem-claude-'),folder=home+'/service/conversations/73757065726167656e74000000000001';fs.mkdirSync(folder,{recursive:true});
 const edit=a=>{const r=spawnSync('python3',[base+'/memory_store.py','--manual',home],{input:JSON.stringify(a),encoding:'utf8'});assert.equal(r.status,0,r.stdout);};
 edit({action:'save',id:'personal.fixture',expected_revision:0,title:'报告偏好',content:'先给结论再给证据'});
 fs.writeFileSync(folder+'/events.jsonl',JSON.stringify({type:'settings',provider:'claude',seq:1,time:Date.now()})+'\n');
 const fake=home+'/claude';fs.writeFileSync(fake,`#!/usr/bin/env python3
import os,sys,json,socket,pathlib,tty
if '--version' in sys.argv:print('2.1.319');sys.exit(0)
f=pathlib.Path(sys.argv[sys.argv.index('--settings')+1]).parent;sess=sys.argv[sys.argv.index('--session-id')+1]
def hook(name,**kw):
 s=socket.socket(socket.AF_UNIX);s.connect(str(f/'run.sock'));s.sendall((json.dumps({'action':'hook','event':{'session_id':sess,'cwd':os.getcwd(),'hook_event_name':name,**kw}})+'\\n').encode());data=s.recv(65536);s.close();return json.loads(data)
tty.setraw(0);hook('SessionStart');data=b''
while True:
 data+=os.read(0,65536)
 if data.endswith(b'\\r') and b'\\x1b[201~' in data:
  text=data.split(b'\\x1b[200~',1)[1].split(b'\\x1b[201~',1)[0].decode();data=b'';reply=hook('UserPromptSubmit',prompt=text)
  with open(os.environ['QA_TRACE'],'a') as out:out.write(json.dumps({'text':text,'reply':reply,'pid':os.getpid()})+'\\n')
  hook('Stop')
`,{mode:0o700});
 let child,socket;try{
 child=spawn(process.execPath,[base+'/daemon.mjs',folder,home,'opus'],{env:{...process.env,HERDR_APP_HOME:home,HERDR_APP_BIN:home+'/unused',HERDR_SUPER_AGENT:'1',HERDR_CHAT_LOCAL:'0',HERDR_CHAT_CLAUDE:fake,QA_TRACE:home+'/trace',HERDR_CODEX_WORKER_SOCKET:home+'/workers.sock',HERDR_CODEX_CONTROL_SOCKET:home+'/no.sock'},stdio:'ignore'});
 for(let i=0;i<100&&!fs.existsSync(folder+'/run.sock');i++)await delay(20);
 socket=net.connect(folder+'/run.sock');socket.on('data',()=>{});await new Promise((r,j)=>{socket.once('connect',r);socket.once('error',j);});socket.write(JSON.stringify({action:'subscribe',after:0})+'\n');
 for(let n=0;n<4;n++){
 if(n===2)edit({action:'save',id:'personal.fixture',expected_revision:1,title:'改过的偏好',content:'新的协作方式'});
 if(n===3)edit({action:'delete',id:'personal.fixture',expected_revision:2});
 socket.write(JSON.stringify({action:'send',request_id:'human_memory_'+n,text:'继续',images:[]})+'\n');
 for(let i=0;i<200;i++){await delay(25);if(fs.existsSync(home+'/trace')&&fs.readFileSync(home+'/trace','utf8').trim().split('\n').length>=n+1)break;}
 }
 const traces=fs.readFileSync(home+'/trace','utf8').trim().split('\n').map(JSON.parse);assert.equal(traces.length,4);assert(traces.every(r=>r.text==='继续'));assert.equal(new Set(traces.map(r=>r.pid)).size,1);
 const ctx=n=>traces[n].reply.output?.hookSpecificOutput?.additionalContext;
 assert.match(ctx(0),/先给结论再给证据/);assert.equal(ctx(1),undefined);assert.match(ctx(2),/新的协作方式/);assert.doesNotMatch(ctx(2),/先给结论再给证据/);assert.doesNotMatch(ctx(3),/新的协作方式|改过的偏好/);
 }finally{socket?.destroy();if(child){child.kill('SIGTERM');await new Promise(r=>child.once('exit',r));}fs.rmSync(home,{recursive:true,force:true});}
});
