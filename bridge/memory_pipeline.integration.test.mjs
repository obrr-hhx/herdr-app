import test from 'node:test';import assert from 'node:assert/strict';import fs from 'node:fs';import path from 'node:path';import {spawn} from 'node:child_process';import {setTimeout as delay} from 'node:timers/promises';import {fileURLToPath} from 'node:url';
const base=path.dirname(fileURLToPath(import.meta.url));
test('background extraction and consolidation persist sources quietly, without worker actions or follow-up-clock changes',async()=>{
 const home=fs.mkdtempSync('/tmp/hmemory-pipeline-'),id='73757065726167656e74000000000001',folder=home+'/data/local-chat/'+id,cache=home+'/data/forge-chat/'+id;fs.mkdirSync(folder,{recursive:true});fs.mkdirSync(cache,{recursive:true});
 fs.writeFileSync(cache+'/events.jsonl',JSON.stringify({type:'user',id:'human-memory-feedback',text:'以后先给结论和证据，不要只报告中间状态',time:Date.now()-180000})+'\n');
 const fake=home+'/codex';fs.writeFileSync(fake,`#!/usr/bin/env node
const fs=require('fs');let stage=0,next=100;const emit=v=>process.stdout.write(JSON.stringify(v)+'\\n'),call=(tool,args)=>emit({id:++next,method:'item/tool/call',params:{threadId:'pipeline',callId:'call-'+next,tool,arguments:args}});
require('readline').createInterface({input:process.stdin}).on('line',l=>{const m=JSON.parse(l),reply=result=>emit({id:m.id,result});
if(m.method==='initialize')reply({});if(m.method==='config/read')reply({config:{}});if(m.method==='model/list')reply({data:[],nextCursor:null});if(m.method==='thread/start'||m.method==='thread/resume')reply({thread:{id:'pipeline'},model:'fixture'});
if(m.method==='turn/start'){stage=0;reply({turn:{id:'pipeline-turn'}});call('review_memories',{});}
if(m.id>100&&m.result){let result={};try{result=JSON.parse(m.result.contentItems[0].text);}catch{}if(stage===0){stage=1;if(!result.memories.length)call('record_memory',{topic:'report.results',expected_revision:0,title:'先给结论与证据',content:'先给结论与证据，持续推进到有结果',kind:'workflow',confidence:'explicit',importance:5,sources:[{id:'human-memory-feedback',quote:'先给结论和证据，不要只报告中间状态'}]});else call('review_memories',{});}
else if(stage===1){stage=2;call('review_memories',{});}
else if(stage===2){stage=3;call('consolidate_memories',{expected_version:result.version,reviewed_source_ids:result.pending_human_feedback.map(r=>r.id),profile:[],preferences:result.memories.map(r=>({text:r.content,memory_ids:[r.id]})),topics:[]});}
else if(stage===3){stage=4;call('send_message',{address:'t1',text:'MUST NOT SEND',request_id:'forbidden-memory-send'});}
else{fs.appendFileSync(process.env.QA_TRACE,JSON.stringify({forbidden_call_blocked:m.result.success===false})+'\\n');emit({method:'item/agentMessage/delta',params:{threadId:'pipeline',itemId:'maintenance-report',delta:'Routine memory work must stay hidden'}});emit({method:'turn/completed',params:{turn:{id:'pipeline-turn',status:'completed'}}});}}
});`,{mode:0o700});
 let child;try{for(let run=0;run<2;run++){
 const records=fs.existsSync(folder+'/events.jsonl')?fs.readFileSync(folder+'/events.jsonl','utf8').trim().split('\n').map(JSON.parse):[{type:'settings',provider:'codex',seq:1,time:Date.now()}];
 const request='observe_memory_pipeline_'+run;records.push({type:'observation',id:request,events:[{kind:'memory_maintenance'}],seq:records.length+1,time:Date.now()});fs.writeFileSync(folder+'/events.jsonl',records.map(JSON.stringify).join('\n')+'\n');
 child=spawn(process.execPath,[base+'/daemon.mjs',folder,home,'opus'],{env:{...process.env,HERDR_APP_HOME:home,HERDR_APP_BIN:home+'/unused',HERDR_CODEX_BIN:fake,HERDR_SUPER_AGENT:'1',HERDR_CHAT_LOCAL:'1',HERDR_CODEX_CONTROL_SOCKET:home+'/no.sock',HERDR_CODEX_WORKER_SOCKET:home+'/workers.sock',QA_TRACE:home+'/trace'},stdio:'ignore'});
 let events=[];for(let i=0;i<400;i++){await delay(25);events=fs.readFileSync(folder+'/events.jsonl','utf8').trim().split('\n').map(JSON.parse);if(events.some(e=>e.type==='observation_complete'&&e.id===request))break;}
 assert(events.some(e=>e.type==='observation_complete'&&e.id===request),'maintenance did not complete');assert(!events.some(e=>e.type==='delta'||e.type==='followup_plan'));
 child.kill('SIGTERM');await new Promise(r=>child.once('exit',r));child=null;
 }
 const state=JSON.parse(fs.readFileSync(home+'/data/super-agent/memories.json'));assert.equal(state.compiled,true);assert.equal(state.memories.length,1);assert.equal(state.memories[0].sources[0].id,'human-memory-feedback');assert.match(state.summary,/先给结论与证据/);assert(state.reviewed_through>0);
 const traces=fs.readFileSync(home+'/trace','utf8').trim().split('\n').map(JSON.parse);assert.equal(traces.length,2);assert(traces.every(r=>r.forbidden_call_blocked));
 }finally{if(child){child.kill('SIGTERM');await new Promise(r=>child.once('exit',r));}fs.rmSync(home,{recursive:true,force:true});}
});
