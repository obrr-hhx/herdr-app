import test from 'node:test';import assert from 'node:assert/strict';import fs from 'node:fs';import path from 'node:path';import net from 'node:net';import {spawn} from 'node:child_process';import {fileURLToPath} from 'node:url';import {setTimeout as delay} from 'node:timers/promises';
const base=path.dirname(fileURLToPath(import.meta.url));
test('manager learns via real tool process then retrieves after daemon restart without changing the human message',async()=>{
 const root=fs.mkdtempSync('/tmp/hmem-'),home=root+'/home',id='73757065726167656e74000000000001',folder=home+'/data/local-chat/'+id,cache=home+'/data/forge-chat/'+id;
 fs.mkdirSync(folder,{recursive:true});fs.mkdirSync(cache,{recursive:true});
 fs.writeFileSync(folder+'/events.jsonl',JSON.stringify({type:'settings',provider:'codex',model:'fixture',seq:1,time:Date.now()})+'\n');
 const fake=root+'/codex';fs.writeFileSync(fake,`#!/usr/bin/env node
const fs=require('fs');const emit=v=>process.stdout.write(JSON.stringify(v)+'\\n');
require('readline').createInterface({input:process.stdin}).on('line',l=>{const m=JSON.parse(l),reply=result=>emit({id:m.id,result});
if(m.method==='initialize')reply({});if(m.method==='config/read')reply({config:{}});if(m.method==='model/list')reply({data:[{model:'fixture',hidden:false,supportedReasoningEfforts:[]}],nextCursor:null});if(m.method==='thread/start'||m.method==='thread/resume')reply({thread:{id:'memory-thread'},model:'fixture'});
if(m.method==='turn/start'){fs.appendFileSync(process.env.QA_TRACE,JSON.stringify(m.params)+'\\n');reply({turn:{id:'memory-turn'}});
if(m.params.clientUserMessageId==='human-learn')emit({id:999,method:'item/tool/call',params:{threadId:'memory-thread',callId:'memory-write',tool:'record_memory',arguments:{topic:'report.evidence',expected_revision:0,title:'先给结论',content:'先给结论和证据',kind:'workflow',confidence:'explicit',importance:5,sources:[{id:'human-learn',quote:'先给结论和证据'}]}}});
else emit({method:'turn/completed',params:{turn:{id:'memory-turn',status:'completed'}}});}
if(m.id===999&&m.result){if(!m.result.success)process.exit(2);emit({method:'turn/completed',params:{turn:{id:'memory-turn',status:'completed'}}});}
});`,{mode:0o700});
 let child,socket;const env={...process.env,HERDR_APP_HOME:home,HERDR_APP_BIN:root+'/unused',HERDR_CODEX_BIN:fake,HERDR_CHAT_LOCAL:'1',HERDR_SUPER_AGENT:'1',HERDR_CODEX_CONTROL_SOCKET:root+'/no.sock',HERDR_CODEX_WORKER_SOCKET:root+'/workers.sock',QA_TRACE:root+'/trace'};
 const stop=async()=>{socket?.destroy();socket=null;if(child){child.kill('SIGTERM');await new Promise(r=>child.once('exit',r));child=null;}};
 try{
 for(let run=0;run<2;run++){
 child=spawn(process.execPath,[base+'/daemon.mjs',folder,root,'opus'],{env,stdio:'ignore'});
 for(let i=0;i<150&&!fs.existsSync(folder+'/run.sock');i++)await delay(30);
 socket=net.connect(folder+'/run.sock');socket.on('data',()=>{});await new Promise((res,rej)=>{socket.once('connect',res);socket.once('error',rej);});
 const request=run?'human-next':'human-learn',text=run?'现在汇报一下':'以后请先给结论和证据';
 fs.appendFileSync(cache+'/events.jsonl',JSON.stringify({type:'user',id:request,text,time:Date.now()})+'\n');
 socket.write(JSON.stringify({action:'subscribe',after:0})+'\n');
 socket.write(JSON.stringify({action:'send',request_id:request,text,images:[]})+'\n');
 let events=[];for(let i=0;i<200;i++){await delay(30);events=fs.readFileSync(folder+'/events.jsonl','utf8').trim().split('\n').map(JSON.parse);if(events.some(e=>e.type==='prompt_confirmed'&&e.id===request)&&events.findLast(e=>e.type==='state')?.status==='idle')break;}
 assert(events.some(e=>e.type==='prompt_confirmed'&&e.id===request));assert.equal(events.findLast(e=>e.type==='state')?.status,'idle');
 await stop();
 }
 const turns=fs.readFileSync(root+'/trace','utf8').trim().split('\n').map(JSON.parse);assert.equal(turns.length,2);
 assert.equal(turns[1].input[0].text,'现在汇报一下');assert.equal(turns[1].input.length,1);assert.match(turns[1].additionalContext.herdr_collaboration_memory.value,/先给结论和证据/);assert.match(turns[1].additionalContext.herdr_collaboration_memory.value,/report.evidence/);
 const memory=JSON.parse(fs.readFileSync(home+'/data/super-agent/memories.json')).memories[0];assert.equal(memory.authority,false);assert.equal(memory.revision,1);
 }finally{await stop();fs.rmSync(root,{recursive:true,force:true});}
});
