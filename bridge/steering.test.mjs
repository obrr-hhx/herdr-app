import test from 'node:test';import assert from 'node:assert/strict';import fs from 'node:fs';import os from 'node:os';import path from 'node:path';import net from 'node:net';import {spawn}from'node:child_process';import {fileURLToPath}from'node:url';import {setTimeout as delay}from'node:timers/promises';import {Steering,unconfirmedSteering}from'./steering.mjs';
test('steering binds an exact active turn and exact prompt without resolving approvals',()=>{
 const s=new Steering(),c={request_id:'steer-test',expected_turn:'turn-a',text:'new direction'},ctx={manager:false,supported:true,busy:true,ready:true,approval:false,turn:'turn-a'};
 assert.equal(s.refusal(c,ctx),null);for(const patch of [{manager:true},{supported:false},{approval:true},{turn:'new-turn'},{busy:false}])assert(s.refusal(c,{...ctx,...patch}));
 s.begin(c,'new direction');assert(s.refusal({...c,request_id:'second'},ctx));assert.equal(s.confirm('different prompt'),null);assert.equal(s.confirm('new direction\r\n'),c);assert.equal(s.confirm('new direction'),null);
 assert.deepEqual(unconfirmedSteering([{type:'user',id:'sent',mode:'steer'},{type:'user',id:'unknown',mode:'steer'},{type:'prompt_confirmed',id:'sent'}]),['unknown']);
 assert(s.refusal({...c,text:'/compact'},ctx));assert(s.refusal({...c,text:'!rm fixture'},ctx));
});
test('ordinary interactive PTY steers before Stop, queue stays FIFO, replay and approvals are safe',{skip:process.platform==='win32',timeout:20000},async()=>{
 const root=fs.realpathSync(fs.mkdtempSync('/tmp/hst-'));const mock=root+'/claude';fs.writeFileSync(mock,`#!/usr/bin/env python3
import sys,json,os,socket,tty
if '--version' in sys.argv:print('2.1.319');sys.exit(0)
folder=os.environ['HERDR_STEER_FIXTURE'];session=sys.argv[sys.argv.index('--session-id')+1]
def hook(name,**fields):
 s=socket.socket(socket.AF_UNIX);s.connect(folder+'/run.sock');s.sendall((json.dumps({'action':'hook','event':{'hook_event_name':name,'session_id':session,'cwd':folder,**fields}})+'\\n').encode());s.recv(65536);s.close()
tty.setraw(0);hook('SessionStart');data=b''
while True:
 data+=os.read(0,65536)
 for suffix,mode in [(b'\\x18\\x13','steer'),(b'\\r','queue')]:
  if data.endswith(suffix) and b'\\x1b[201~' in data:
   text=data.split(b'\\x1b[200~',1)[1].split(b'\\x1b[201~',1)[0].decode();data=b''
   with open(folder+'/prompts.jsonl','a') as f:f.write(json.dumps({'mode':mode,'text':text})+'\\n')
   hook('UserPromptSubmit',prompt=text)
`,{mode:0o700});
 const child=spawn(process.execPath,[fileURLToPath(new URL('./daemon.mjs',import.meta.url)),root,root,'opus'],{stdio:'ignore',env:{...process.env,HERDR_CHAT_CLAUDE:mock,HERDR_STEER_FIXTURE:root}});let socket;const seen=[];
 const wait=async predicate=>{for(let i=0;i<400;i++){const e=seen.find(predicate);if(e)return e;await delay(20);}throw Error('steering fixture timeout: '+JSON.stringify(seen.slice(-5)));};
 const hook=event=>new Promise((resolve,reject)=>{const p=net.createConnection(root+'/run.sock');p.on('error',reject);p.on('connect',()=>p.write(JSON.stringify({action:'hook',event})+'\n'));p.on('data',()=>{p.end();resolve();});});
 try{
  for(let i=0;i<100&&!fs.existsSync(root+'/run.sock');i++)await delay(20);
  socket=net.createConnection(root+'/run.sock');socket.setEncoding('utf8');let buffer='';socket.on('data',chunk=>{buffer+=chunk;let n;while((n=buffer.indexOf('\n'))>=0){seen.push(JSON.parse(buffer.slice(0,n)));buffer=buffer.slice(n+1);}});socket.on('error',()=>{});const send=c=>socket.write(JSON.stringify(c)+'\n');
  send({action:'subscribe',after:0});await wait(e=>e.type==='connected');const sid=seen.find(e=>e.type==='session').session_id;
  send({action:'send',request_id:'original-turn-id',text:'original',images:[]});await wait(e=>e.type==='prompt_confirmed'&&e.id==='original-turn-id');await wait(e=>e.type==='capabilities'&&e.capabilities.includes('steer'));
  send({action:'send',request_id:'queued-after-turn',text:'later',images:[],mode:'queue'});await wait(e=>e.type==='ack'&&e.request_id==='queued-after-turn');
  const steer={action:'send',request_id:'steer-without-stop',text:'change direction',images:[],mode:'steer',expected_turn:'original-turn-id'};send(steer);await wait(e=>e.type==='prompt_confirmed'&&e.id===steer.request_id);
  send(steer);send({...steer,request_id:'stale-turn-message',expected_turn:'different-turn'});await wait(e=>e.type==='dispatch_receipt'&&e.id==='stale-turn-message'&&e.stage==='refused');
  let questionResolved=false;const question=hook({hook_event_name:'PreToolUse',session_id:sid,cwd:root,tool_name:'AskUserQuestion',tool_use_id:'fixture-question',tool_input:{questions:[]}}).then(()=>questionResolved=true);const permission=await wait(e=>e.type==='permission');
  send({...steer,request_id:'approval-blocked-message'});await wait(e=>e.type==='dispatch_receipt'&&e.id==='approval-blocked-message'&&e.stage==='refused');assert(!questionResolved);
  const prompts=fs.readFileSync(root+'/prompts.jsonl','utf8').trim().split('\n').map(JSON.parse);assert.deepEqual(prompts,[{mode:'queue',text:'original'},{mode:'steer',text:'change direction'}]);assert(!seen.some(e=>e.type==='prompt_confirmed'&&e.id==='queued-after-turn'));
  send({action:'permission',request_id:'decline-fixture-question',id:permission.id,decision:'deny'});await question;
  await hook({hook_event_name:'Stop',session_id:sid,cwd:root});await wait(e=>e.type==='prompt_confirmed'&&e.id==='queued-after-turn');
 }finally{socket?.destroy();child.kill('SIGTERM');await new Promise(r=>child.once('exit',r));fs.rmSync(root,{recursive:true,force:true});}
});

test('legacy bridge upgrades only through its sole idle client and reconnects to protocol 2',{skip:process.platform==='win32',timeout:15000},async()=>{
 const root=fs.realpathSync(fs.mkdtempSync('/tmp/hsu-'));const service=root+'/svc';fs.mkdirSync(service);for(const name of fs.readdirSync(fileURLToPath(new URL('.',import.meta.url))))if(/\.(mjs|py)$/.test(name)&&!name.endsWith('.test.mjs')&&!name.endsWith('_test.py'))fs.copyFileSync(fileURLToPath(new URL(name,import.meta.url)),service+'/'+name);
 fs.mkdirSync(root+'/durable');fs.symlinkSync(root+'/durable',service+'/conversations');const id='d'.repeat(32),folder=service+'/conversations/'+id;fs.mkdirSync(folder,{recursive:true});const initial=[{type:'session',session_id:'10000000-0000-4000-8000-000000000001',seq:1,time:Date.now()-5000},{type:'state',status:'idle',seq:2,time:Date.now()-5000}];fs.writeFileSync(folder+'/events.jsonl',initial.map(JSON.stringify).join('\n')+'\n');
 const actual=fs.readFileSync(service+'/daemon.mjs');fs.writeFileSync(service+'/daemon.mjs',`import net from 'node:net';import fs from 'node:fs';const folder=process.argv[2];const server=net.createServer(s=>{s.on('error',()=>{});let buffer='';s.on('data',b=>{buffer+=b;let n;while((n=buffer.indexOf('\\n'))>=0){const c=JSON.parse(buffer.slice(0,n));buffer=buffer.slice(n+1);if(c.action==='subscribe')s.write(fs.readFileSync(folder+'/events.jsonl')+JSON.stringify({type:'connected',status:'idle',head:2,capabilities:[]})+'\\n');}});});server.listen(folder+'/run.sock');process.on('SIGTERM',()=>process.exit(0));`);
 const old=spawn(process.execPath,[service+'/daemon.mjs',folder,root],{stdio:'ignore'});let client,second;try{
  for(let i=0;i<100&&!fs.existsSync(folder+'/run.sock');i++)await delay(20);fs.writeFileSync(service+'/daemon.mjs',actual);
  const env={...process.env,HERDR_CHAT_HOME:service};client=spawn(process.execPath,[service+'/client.mjs',id,root],{env,stdio:['pipe','pipe','ignore']});let data='';client.stdout.on('data',b=>data+=b);client.stdin.write(JSON.stringify({action:'subscribe',after:0})+'\n');for(let i=0;i<250&&client.exitCode===null&&client.signalCode===null;i++)await delay(20);assert(client.exitCode!==null||client.signalCode!==null,'idle upgrade did not finish: '+data.slice(-300));
  assert(old.exitCode!==null||old.signalCode!==null,'legacy daemon did not exit');assert(data.includes('connected'));assert(fs.readFileSync(folder+'/events.jsonl','utf8').startsWith(initial.map(JSON.stringify).join('\n')));
  second=spawn(process.execPath,[service+'/client.mjs',id,root],{env,stdio:['pipe','pipe','ignore']});let wire='';second.stdout.on('data',b=>wire+=b);second.stdin.write(JSON.stringify({action:'subscribe',after:0})+'\n');for(let i=0;i<200&&!wire.includes('"bridge_protocol":2');i++)await delay(20);assert(wire.includes('"bridge_protocol":2'),'reconnect did not load updated daemon');
 }finally{
  client?.kill();second?.kill();if(old.exitCode===null&&old.signalCode===null)old.kill();
  const p=spawn('python3',[service+'/runtime_release.py'],{stdio:['pipe','ignore','ignore']});p.stdin.end(JSON.stringify({root:service,conversation:id,request_id:'upgrade-fixture-cleanup'}));await new Promise(r=>p.once('exit',r));fs.rmSync(root,{recursive:true,force:true});
 }
});
