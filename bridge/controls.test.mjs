import test from 'node:test';import assert from 'node:assert/strict';import fs from 'node:fs';import os from 'node:os';import net from 'node:net';import path from 'node:path';import {spawn}from 'node:child_process';import {fileURLToPath}from 'node:url';import {setTimeout as delay}from 'node:timers/promises';
test('runtime changes are refused during a turn and acknowledged after idle', {skip:process.platform==='win32'},async()=>{
 const root=fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'herdr-controls-')));const mock=root+'/claude';fs.writeFileSync(mock,'#!/usr/bin/env python3\nimport sys\nif "--version" in sys.argv: print("2.1.319"); sys.exit(0)\nimport time\ntime.sleep(60)\n',{mode:0o755});
 const child=spawn(process.execPath,[fileURLToPath(new URL('./daemon.mjs',import.meta.url)),root,root,'opus'],{stdio:'ignore',env:{...process.env,HERDR_CHAT_CLAUDE:mock}});let socket;
 try{
  for(let i=0;i<100&&!fs.existsSync(root+'/run.sock');i++)await delay(20);
  socket=net.createConnection(root+'/run.sock');const seen=[];let buffer='';socket.setEncoding('utf8');socket.on('data',s=>{buffer+=s;let n;while((n=buffer.indexOf('\n'))>=0){seen.push(JSON.parse(buffer.slice(0,n)));buffer=buffer.slice(n+1);}});socket.on('error',()=>{});
  const send=c=>socket.write(JSON.stringify(c)+'\n');const wait=async f=>{for(let i=0;i<150;i++){const e=seen.find(f);if(e)return e;await delay(20);}throw Error('control test timeout');};
  const hook=event=>new Promise((resolve,reject)=>{const h=net.createConnection(root+'/run.sock');h.on('error',reject);h.on('connect',()=>h.write(JSON.stringify({action:'hook',event})+'\n'));h.on('data',()=>{h.end();resolve();});});
  send({action:'subscribe',after:0});await wait(e=>e.type==='connected');const sid=seen.find(e=>e.type==='session').session_id;
  send({action:'configure',request_id:'config-before-turn',model:'opus',effort:'low'});await wait(e=>e.type==='ack'&&e.request_id==='config-before-turn');
  send({action:'send',request_id:'mock-test-prompt',text:'fixture'});
  await hook({hook_event_name:'SessionStart',session_id:sid,cwd:root,source:'startup'});await wait(e=>e.type==='state'&&e.status==='working');
  const offset=seen.length;send({action:'configure',request_id:'config-while-busy',model:'opus',effort:'high'});await wait(e=>e.type==='ack'&&e.request_id==='config-while-busy');assert(seen.slice(offset).some(e=>e.type==='notice'&&e.message.includes('Wait')));assert(!seen.slice(offset).some(e=>e.type==='settings'));
  await hook({hook_event_name:'PreToolUse',session_id:sid,cwd:root+'/subproject',tool_name:'Read',tool_use_id:'changed-cwd-tool',tool_input:{file_path:'fixture.txt'}});
  await wait(e=>e.type==='tool'&&e.id==='changed-cwd-tool');
  await hook({hook_event_name:'PreToolUse',session_id:'different-session',cwd:root+'/subproject',tool_name:'Read',tool_use_id:'wrong-owner',tool_input:{}});
  assert(!seen.some(e=>e.type==='tool'&&e.id==='wrong-owner'));
  await hook({hook_event_name:'Stop',session_id:sid,cwd:root+'/subproject'});await delay(50);
  send({action:'configure',request_id:'config-after-turn',model:'opus',effort:'high'});await wait(e=>e.type==='ack'&&e.request_id==='config-after-turn');assert(seen.some(e=>e.type==='settings'&&e.effort==='high'));
  const journal=fs.readFileSync(root+'/events.jsonl','utf8');assert(!journal.includes('config-while-busy\"'));assert(!journal.includes('wrong-owner'));
 }finally{socket?.destroy();child.kill();await new Promise(r=>child.once('exit',r));fs.rmSync(root,{recursive:true,force:true});}
});

test('new and resumed interactive runners always use the requested full tool access', {skip:process.platform==='win32'},async()=>{
 for(const resume of [false,true]){
  const root=fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'herdr-permission-')));
  const mock=root+'/claude';
  fs.writeFileSync(mock,'#!/usr/bin/env python3\nimport sys\nif "--version" in sys.argv: print("2.1.319"); sys.exit(0)\nimport json,os,sys\nopen(os.environ["HERDR_QA_ARGS"],"w").write(json.dumps(sys.argv[1:]))\n',{mode:0o755});
  if(resume)fs.writeFileSync(root+'/has-session','1');
  try{
   const child=spawn('python3',[fileURLToPath(new URL('./runner.py',import.meta.url)),root,root,'opus','00000000-0000-4000-8000-000000000000','high'],{stdio:['pipe','ignore','ignore'],env:{...process.env,HERDR_CHAT_CLAUDE:mock,HERDR_QA_ARGS:root+'/args.json'}});
   await new Promise((resolve,reject)=>{child.once('error',reject);child.once('exit',resolve);});
   const args=JSON.parse(fs.readFileSync(root+'/args.json','utf8'));
   assert(args.includes('--dangerously-skip-permissions'));
   assert(args.includes(resume?'--resume':'--session-id'));
   assert(!args.includes('default'));assert(!args.includes('-p'));
  }finally{fs.rmSync(root,{recursive:true,force:true});}
 }
});

test('a queued message survives a slow idle CLI restart', {skip:process.platform==='win32',timeout:20000},async()=>{
 const root=fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'herdr-switch-')));const mock=root+'/claude';
 fs.writeFileSync(mock,'#!/usr/bin/env python3\nimport sys\nif "--version" in sys.argv: print("2.1.319"); sys.exit(0)\nimport os,signal,time\nsignal.signal(signal.SIGTERM,signal.SIG_IGN)\nwith open(os.environ["HERDR_QA_PIDS"],"a") as f:f.write(str(os.getpid())+"\\n")\ntime.sleep(60)\n',{mode:0o700});
 const child=spawn(process.execPath,[fileURLToPath(new URL('./daemon.mjs',import.meta.url)),root,root,'opus'],{stdio:'ignore',env:{...process.env,HERDR_CHAT_CLAUDE:mock,HERDR_QA_PIDS:root+'/pids'}});let socket;const seen=[];
 const wait=async f=>{for(let i=0;i<500;i++){const e=seen.find(f);if(e)return e;await delay(20);}throw Error('switch test timeout');};
 const hook=event=>new Promise((resolve,reject)=>{const h=net.createConnection(root+'/run.sock');h.on('error',reject);h.on('connect',()=>h.write(JSON.stringify({action:'hook',event})+'\n'));h.on('data',()=>{h.end();resolve();});});
 try{
  for(let i=0;i<100&&!fs.existsSync(root+'/run.sock');i++)await delay(20);
  socket=net.createConnection(root+'/run.sock');let buffer='';socket.setEncoding('utf8');socket.on('data',chunk=>{buffer+=chunk;let n;while((n=buffer.indexOf('\n'))>=0){seen.push(JSON.parse(buffer.slice(0,n)));buffer=buffer.slice(n+1);}});socket.on('error',()=>{});
  const send=c=>socket.write(JSON.stringify(c)+'\n');send({action:'subscribe',after:0});await wait(e=>e.type==='connected');const sid=seen.find(e=>e.type==='session').session_id;
  send({action:'send',request_id:'before-restart',text:'first'});
  await hook({hook_event_name:'SessionStart',session_id:sid,cwd:root});await wait(e=>e.type==='state'&&e.status==='working');
  for(let i=0;i<250&&!fs.existsSync(root+'/pids');i++)await delay(20);
  assert(fs.existsSync(root+'/pids'),'old CLI must be running before testing its slow shutdown');
  await hook({hook_event_name:'Stop',session_id:sid,cwd:root});
  send({action:'configure',request_id:'restart-config',model:'sonnet',effort:'high'});await wait(e=>e.type==='ack'&&e.request_id==='restart-config');
  const offset=seen.length;send({action:'send',request_id:'during-restart',text:'second'});await wait(e=>e.type==='ack'&&e.request_id==='during-restart');
  await hook({hook_event_name:'StopFailure',session_id:sid,cwd:root,error:'server_error'});
  for(let i=0;i<500;i++){if(fs.existsSync(root+'/pids')&&fs.readFileSync(root+'/pids','utf8').trim().split('\n').length>=2)break;await delay(20);}
  assert.equal(fs.readFileSync(root+'/pids','utf8').trim().split('\n').length,2,JSON.stringify(seen.slice(offset)));
  await hook({hook_event_name:'SessionStart',session_id:sid,cwd:root});
  for(let i=0;i<100&&!seen.slice(offset).some(e=>e.type==='state'&&e.status==='working');i++)await delay(20);
  assert(seen.slice(offset).some(e=>e.type==='state'&&e.status==='working'),'queued second prompt must dispatch after restart');
 }finally{
  const pids=fs.existsSync(root+'/pids')?fs.readFileSync(root+'/pids','utf8').trim().split('\n').map(Number):[];
  socket?.destroy();child.kill();await new Promise(r=>child.once('exit',r));
  for(let i=0;i<300;i++){let alive=false;for(const pid of pids){try{process.kill(pid,0);alive=true;}catch{}}if(!alive)break;await delay(20);}
  fs.rmSync(root,{recursive:true,force:true});
 }
});
