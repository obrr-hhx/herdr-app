import test from 'node:test';import assert from 'node:assert/strict';import fs from 'node:fs';import os from 'node:os';import path from 'node:path';import {spawnSync}from 'node:child_process';import {fileURLToPath}from 'node:url';
import {normalizeStatus,ensureStatusLine,readStatusLine}from './statusline.mjs';
test('status telemetry forwards only bounded known fields and keeps missing values unknown',()=>{
 const s=normalizeStatus({model:'Opus',context_pct:0,cost_usd:0,five_hour:{used_pct:30,resets_at:1234},api_key:'must-not-forward',context_size:-1,output_tokens:'123'});
 assert.equal(s.context_pct,0);assert.equal(s.cost_usd,0);assert.equal(s.seven_day.used_pct,null);assert.equal(s.context_size,null);assert.equal(s.output_tokens,null);assert(!JSON.stringify(s).includes('must-not-forward'));
 assert.equal(normalizeStatus({context_pct:101}).context_pct,null);
});
test('adding a status line preserves hooks and only rewrites configuration when needed',()=>{
 const dir=fs.mkdtempSync(path.join(os.tmpdir(),"herdr-status 'qa-"));try{
  const file=path.join(dir,'settings.json');fs.writeFileSync(file,JSON.stringify({hooks:{Stop:[{hooks:[]}]},permissions:{defaultMode:'bypassPermissions'}}));ensureStatusLine(dir);const first=fs.statSync(file).mtimeMs;ensureStatusLine(dir);
  assert.equal(fs.statSync(file).mtimeMs,first);const config=JSON.parse(fs.readFileSync(file,'utf8'));assert(config.hooks.Stop);assert.equal(config.permissions.defaultMode,'bypassPermissions');assert(config.statusLine.command.includes('statusline.py'));assert.equal(config.statusLine.refreshInterval,30);
  fs.writeFileSync(path.join(dir,'statusline.json'),'{partial');assert.equal(readStatusLine(dir),null);
 }finally{fs.rmSync(dir,{recursive:true,force:true});}
});
test('native collector rejects a different session and never stores unrelated CLI data',()=>{
 const dir=fs.mkdtempSync(path.join(os.tmpdir(),'herdr-native-status-'));try{
  fs.writeFileSync(path.join(dir,'events.jsonl'),JSON.stringify({type:'session',session_id:'test-session'})+'\n');
  const input={session_id:'test-session',model:{display_name:'Opus 5.5'},context_window:{used_percentage:12,total_input_tokens:24000,context_window_size:200000},rate_limits:{five_hour:{used_percentage:30,resets_at:2000000000}},cost:{total_cost_usd:.27},api_key:'must-not-store',transcript_path:'/private/transcript'};
  const run=data=>spawnSync('python3',[fileURLToPath(new URL('./statusline.py',import.meta.url)),dir],{input:JSON.stringify(data),encoding:'utf8'});
  assert.equal(run(input).status,0);const file=path.join(dir,'statusline.json'),before=fs.readFileSync(file,'utf8'),s=JSON.parse(before);assert.equal(s.context_pct,12);assert.equal(s.five_hour.used_pct,30);assert.equal(s.seven_day.used_pct,null);assert(!before.includes('must-not-store'));assert(!before.includes('transcript'));assert.equal(fs.statSync(file).mode&0o777,0o600);
  run({...input,session_id:'other-session',context_window:{used_percentage:99}});assert.equal(fs.readFileSync(file,'utf8'),before);
 }finally{fs.rmSync(dir,{recursive:true,force:true});}
});

test('client telemetry cannot split a daemon JSON frame', {skip:process.platform==='win32'},async()=>{
 const {default:net}=await import('node:net');const {spawn}=await import('node:child_process');const {setTimeout:delay}=await import('node:timers/promises');
 const home=fs.mkdtempSync(process.platform==='darwin'?'/tmp/hc-':path.join(os.tmpdir(),'herdr-frame-')),id='a'.repeat(32),folder=path.join(home,'.local/share/herdr-chat/conversations',id);fs.mkdirSync(folder,{recursive:true});fs.writeFileSync(folder+'/settings.json','{"hooks":{}}');fs.writeFileSync(folder+'/statusline.json','{"context_pct":1,"updated_at":1}');
 const server=net.createServer();await new Promise(r=>server.listen(folder+'/run.sock',r));let child,peer;
 try{
  const accepted=new Promise(r=>server.once('connection',s=>{peer=s;r(s);}));
  child=spawn(process.execPath,[fileURLToPath(new URL('./client.mjs',import.meta.url)),id,home,'opus'],{env:{...process.env,HOME:home},stdio:['pipe','pipe','pipe']});let output='';child.stdout.setEncoding('utf8');child.stdout.on('data',s=>output+=s);
  await accepted;const frame=JSON.stringify({type:'state',seq:1,status:'idle'});peer.write(frame.slice(0,15));
  fs.writeFileSync(folder+'/statusline.json','{"context_pct":2,"updated_at":2}');await delay(1200);peer.write(frame.slice(15)+'\n');await delay(100);peer.end();await new Promise(r=>child.once('exit',r));
  const records=output.trim().split('\n').map(JSON.parse);assert(records.some(e=>e.type==='state'&&e.seq===1));assert(records.some(e=>e.type==='statusline'&&e.context_pct===2));
 }finally{peer?.destroy();child?.kill();server.close();fs.rmSync(home,{recursive:true,force:true});}
});

test('global dispatcher preserves the original formatter and only registers managed sessions',async()=>{
 const home=fs.mkdtempSync(path.join(os.tmpdir(),'herdr-dispatch-')),base=path.join(home,'.local/share/herdr-chat'),folder=path.join(base,'conversations','a'.repeat(32)),sid='11111111-1111-4111-8111-111111111111';
 fs.mkdirSync(folder,{recursive:true});fs.mkdirSync(path.join(home,'.claude'));fs.copyFileSync(fileURLToPath(new URL('./statusline.mjs',import.meta.url)),base+'/statusline.mjs');fs.copyFileSync(fileURLToPath(new URL('./statusline.py',import.meta.url)),base+'/statusline.py');
 try{
  const original=path.join(home,'.claude/original.sh');fs.writeFileSync(original,'#!/bin/sh\ncat >/dev/null\nprintf ORIGINAL_FORMATTER\n',{mode:0o755});const settings=path.join(home,'.claude/settings.json');fs.writeFileSync(settings,JSON.stringify({statusLine:{type:'command',command:original},permissions:{defaultMode:'default'}}));fs.writeFileSync(folder+'/events.jsonl',JSON.stringify({type:'session',session_id:sid})+'\n');
  const {installStatusDispatcher}=await import('file://'+base+'/statusline.mjs');installStatusDispatcher(folder,home);const cfg=JSON.parse(fs.readFileSync(settings,'utf8'));assert(cfg.statusLine.command.includes('--dispatch'));assert.equal(cfg.permissions.defaultMode,'default');
  const input={session_id:sid,model:{display_name:'Opus'},rate_limits:{five_hour:{used_percentage:42}}};
  const run=data=>spawnSync('python3',[base+'/statusline.py','--dispatch'],{input:JSON.stringify(data),encoding:'utf8'});
  assert.equal(run(input).stdout,'ORIGINAL_FORMATTER');assert.equal(JSON.parse(fs.readFileSync(folder+'/statusline.json')).five_hour.used_pct,42);
  assert.equal(run({...input,session_id:'22222222-2222-4222-8222-222222222222'}).stdout,'ORIGINAL_FORMATTER');
  const saved=fs.readFileSync(base+'/statusline-original.json','utf8');installStatusDispatcher(folder,home);assert.equal(fs.readFileSync(base+'/statusline-original.json','utf8'),saved);
  fs.writeFileSync(settings,JSON.stringify({statusLine:{type:'command',command:original}}));installStatusDispatcher(folder,home);assert.equal(JSON.parse(fs.readFileSync(settings,'utf8')).statusLine.command,original);
 }finally{fs.rmSync(home,{recursive:true,force:true});}
});
