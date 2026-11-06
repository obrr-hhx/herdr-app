import {test} from 'node:test';import assert from 'node:assert/strict';import fs from 'node:fs';import os from 'node:os';import path from 'node:path';import {codexRunner,errorLabel} from './codex.mjs';
test('Codex streams structured messages, serves bounded tools and resumes the same thread',async()=>{
 const root=fs.mkdtempSync(path.join(os.tmpdir(),'herdr-codex-test-'));
 const saved={...process.env};
 const fake=path.join(root,'codex');const app=path.join(root,'app');const trace=path.join(root,'trace');
 fs.writeFileSync(app,'#!/usr/bin/env node\nconsole.log(JSON.stringify({sessions:[],machines:[]}));',{mode:0o700});
 fs.writeFileSync(fake,`#!/usr/bin/env node
const fs=require('fs'),rl=require('readline').createInterface({input:process.stdin});
const write=v=>process.stdout.write(JSON.stringify(v)+'\\n');
rl.on('line',l=>{const m=JSON.parse(l);fs.appendFileSync(process.env.TEST_TRACE,JSON.stringify(m)+'\\n');
if(m.method==='initialize')write({id:m.id,result:{}});
if(m.method==='config/read')write({id:m.id,result:{config:{mcp_servers:{legacy:{enabled:true}}}}});
if(m.method==='model/list')write({id:m.id,result:{data:[{model:'test-model',displayName:'Test model',description:'Fixture',hidden:false,defaultReasoningEffort:'medium',supportedReasoningEfforts:[{reasoningEffort:'high',description:'More reasoning'}]}],nextCursor:null}});
if(m.method==='thread/start'||m.method==='thread/resume')write({id:m.id,result:{thread:{id:'thread-test'},model:'test-model'}});
if(m.method==='turn/start'){write({id:m.id,result:{turn:{id:'turn-test'}}});write({method:'item/tool/call',id:999,params:{threadId:'thread-test',turnId:'turn-test',callId:'call-test',tool:m.params.input?.[0]?.text==='checkin-guard'?'send_message':'list_sessions',arguments:{}}});}
if(m.id===999&&m.result){for(const delta of ['hello',' world'])write({method:'item/agentMessage/delta',params:{threadId:'thread-test',itemId:'message-test',delta}});write({method:'item/completed',params:{item:{type:'agentMessage',id:'message-test'}}});write({method:'turn/completed',params:{turn:{id:'turn-test',status:'completed'}}});}
});`,{mode:0o700});
 Object.assign(process.env,{HERDR_CODEX_BIN:fake,HERDR_APP_BIN:app,HERDR_APP_HOME:root,TEST_TRACE:trace});
 const folder=path.join(root,'conversation');fs.mkdirSync(folder);
 try{
  for(let run=0;run<3;run++){
   const settings={model:'test-model',effort:'high'};const chunks=[];let turns=0;let runner;let callbacks;
   await new Promise((resolve,reject)=>{
    const timer=setTimeout(()=>{runner?.kill();reject(Error('test timeout'));},10000);
    callbacks={models:models=>{assert.equal(models[0].model,'test-model');assert.equal(models[0].supportedReasoningEfforts[0].reasoningEffort,'high');},ready:model=>{assert.equal(model,'test-model');runner.stdin.write(JSON.stringify({action:'send',request_id:run===2?'observe_checkin_qa':'test-request',text:run===2?'checkin-guard':'test'}));},confirmed:id=>assert.equal(id,run===2?'observe_checkin_qa':'test-request'),delta:(_id,_idx,t)=>chunks.push(t),complete(){},tool(){assert.notEqual(run,2,'Periodic writes must not reach tool execution');},toolResult(_id,value,error){assert.equal(error,false);assert.ok(Array.isArray(value.sessions));},usage(){},failure:reason=>{clearTimeout(timer);runner?.kill();reject(Error(reason));},done(){if(run===0&&++turns===1){settings.effort='';runner.stdin.write(JSON.stringify({action:'send',request_id:'test-request',text:'test default effort'}));return;}clearTimeout(timer);runner.kill();resolve();}};
    runner=codexRunner(folder,root,callbacks,settings);
   });
   assert.equal(chunks.join(''),run===0?'hello worldhello world':'hello world');
  }
  const records=fs.readFileSync(trace,'utf8').trim().split('\n').map(JSON.parse);
  assert.equal(records.filter(r=>r.id===999&&r.result?.success===false).length,1,'Check-in mutation was rejected through the real Codex adapter');
  const start=records.find(r=>r.method==='thread/start');assert.deepEqual(start.params.environments,[]);assert.equal(start.params.config['mcp_servers.legacy.enabled'],false);assert.equal(start.params.sandbox,'read-only');assert.ok(start.params.dynamicTools.some(t=>t.name==='create_session'));
  assert.equal(records.find(r=>r.method==='thread/resume').params.threadId,'thread-test');
  assert.equal(start.params.model,'test-model');assert.equal(start.params.config.model_reasoning_effort,'high');
  const turns=records.filter(r=>r.method==='turn/start');assert.deepEqual(turns.map(t=>t.params.effort),['high','medium','high','high']);for(const t of turns)assert.equal(t.params.model,'test-model');
 }finally{for(const k of Object.keys(process.env))if(!(k in saved))delete process.env[k];Object.assign(process.env,saved);fs.rmSync(root,{recursive:true,force:true});}
});

test('error labels only expose known classes and numeric status, never credentials',()=>{assert.equal(errorLabel({responseTooManyFailedAttempts:{httpStatusCode:502}}),'responseTooManyFailedAttempts HTTP 502');assert.equal(errorLabel('Bearer fake-sensitive-value'),'other');assert.equal(errorLabel({unknown:'secret'}),'other');});
