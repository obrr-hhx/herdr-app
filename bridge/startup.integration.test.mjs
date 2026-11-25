import test from 'node:test';import assert from 'node:assert/strict';import fs from 'node:fs';import {spawn}from'node:child_process';import {fileURLToPath}from'node:url';import {setTimeout as delay}from'node:timers/promises';import {Journal}from'./journal.mjs';
for(const resumed of [false,true])test(`${resumed?'An existing':'A cold'} Claude startup resumes only its never-submitted queue and does not repeat it after restart`,{skip:process.platform==='win32',timeout:20000},async()=>{
 const root=fs.realpathSync(fs.mkdtempSync('/tmp/hboot-')),sid='10000000-0000-4000-8000-000000000001',base=fileURLToPath(new URL('.',import.meta.url));
 const j=new Journal(root);j.append('session',{session_id:sid,backend:'interactive'});j.append('state',{status:'idle'});
 if(resumed){fs.writeFileSync(root+'/has-session','1');j.append('user',{id:'import:historical',text:'do not replay imported history'});}
 for(const id of ['queued-original','queued-hi']){const c={action:'send',request_id:id,text:id,images:[],mode:'queue'};j.accept(c);j.append('user',{id,text:c.text,images:[],mode:'queue'});}
 const mock=root+'/claude';fs.writeFileSync(mock,`#!/usr/bin/env python3
import os,sys,json,subprocess,tty
if '--version' in sys.argv:print('2.1.332');sys.exit(0)
root=os.environ['BOOT_FIXTURE'];sid=sys.argv[sys.argv.index('--session-id')+1] if '--session-id' in sys.argv else sys.argv[sys.argv.index('--resume')+1]
def hook(name,**extra):
 e={'hook_event_name':name,'session_id':sid,'cwd':root,**extra}
 subprocess.run(['python3',os.environ['BOOT_HOOK'],root],input=json.dumps(e),text=True,stdout=subprocess.DEVNULL,check=True)
tty.setraw(0);hook('SessionStart');data=b''
while True:
 data+=os.read(0,65536)
 if data.endswith(b'\\r') and b'\\x1b[201~' in data:
  text=data.split(b'\\x1b[200~',1)[1].split(b'\\x1b[201~',1)[0].decode();data=b''
  with open(root+'/received','a') as f:f.write(text+'\\n')
  hook('UserPromptSubmit',prompt=text);hook('Stop')
`,{mode:0o700});
 let child;const read=()=>fs.readFileSync(root+'/events.jsonl','utf8').trim().split('\n').map(JSON.parse);
 try{
  for(let run=0;run<2;run++){
   child=spawn(process.execPath,[base+'daemon.mjs',root,root,'opus'],{stdio:'ignore',env:{...process.env,HERDR_CHAT_CLAUDE:mock,BOOT_FIXTURE:root,BOOT_HOOK:base+'hook.py'}});
   for(let n=0;n<300;n++){await delay(30);const es=read();if(run===0?es.filter(e=>e.type==='prompt_confirmed').length===2:es.some(e=>e.type==='hook'&&e.seq>j.events.length&&e.event?.hook_event_name==='SessionStart'))break;}
   if(run===0){for(let n=0;n<100&&read().at(-1)?.status!=='idle';n++)await delay(30);const before=fs.readFileSync(root+'/events.jsonl','utf8');const duplicate=spawn(process.execPath,[base+'daemon.mjs',root,root,'opus'],{stdio:'ignore'});assert.equal(await new Promise(r=>duplicate.once('exit',r)),1);assert.equal(fs.readFileSync(root+'/events.jsonl','utf8'),before);const es=read();assert.equal(es.filter(e=>e.type==='prompt_confirmed').length,2);assert.deepEqual(es.find(e=>e.type==='startup_queue_restored').request_ids,['queued-original','queued-hi']);}
   else await delay(1200);
   child.kill('SIGTERM');await new Promise(r=>child.once('exit',r));child=null;
  }
  assert.deepEqual(fs.readFileSync(root+'/received','utf8').trim().split('\n'),['queued-original','queued-hi']);
  assert.equal(read().filter(e=>e.type==='startup_queue_restored').length,1);
 }finally{if(child){child.kill('SIGTERM');await new Promise(r=>child.once('exit',r));}fs.rmSync(root,{recursive:true,force:true});}
});
