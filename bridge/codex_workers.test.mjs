import test from 'node:test';import assert from 'node:assert/strict';import fs from 'node:fs';import os from 'node:os';import path from 'node:path';import {EventEmitter}from'node:events';import {startCodexWorkers}from'./codex_workers.mjs';
test('worker questions replay into manager, survive disconnect and resolve only with live proof',()=>{
 const home=fs.mkdtempSync(path.join(os.tmpdir(),'hcodex-test-'));const file=path.join(home,'data/super-agent/codex-workers.json');fs.mkdirSync(path.dirname(file),{recursive:true});const events=[];let child;const deps={spawn(){child=new EventEmitter();child.stdout=new EventEmitter();child.kill=()=>{};return child;}};
 const q={id:'codexq_test',name:'AskUserQuestion',reply_supported:true,input:{questions:[{question:'Complete question',options:[{label:'B',description:'Full description'}]}]}};
 const write=(row)=>fs.writeFileSync(file,JSON.stringify({at:Date.now(),sessions:{t2:{title:'Task',...row}}}));
 write({binding_live:true,connected:true,pending_questions:{[q.id]:q}});const worker=startCodexWorkers(home,(type,value)=>events.push({type,...value}),{events:[]},deps);
 try{
  assert.equal(events[0].input.questions[0].options[0].description,'Full description');assert.deepEqual(worker.ids(),['codexq_test']);worker.sync();assert.equal(events.length,1);
  write({binding_live:false,connected:false,pending_questions:{}});child.stdout.emit('data','updated');assert.equal(events.length,1);assert(worker.has(q.id));
  write({binding_live:true,connected:true,pending_questions:{}});worker.sync();assert.equal(events.at(-1).type,'permission_resolved');assert.equal(worker.ids().length,0);
 }finally{worker.stop();fs.rmSync(home,{recursive:true,force:true});}
});
