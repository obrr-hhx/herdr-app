import test from 'node:test';import assert from 'node:assert/strict';import fs from 'node:fs';import path from 'node:path';import {spawnSync} from 'node:child_process';import {startMemoryMaintenance} from './memory_maintenance.mjs';import {readOnlyTool} from './checkins.mjs';
test('quiet maintenance waits for human inactivity, coalesces, and cannot dispatch workers or change follow-up timing',()=>{
 const home=fs.mkdtempSync('/tmp/hmemory-maint-'),folder=path.join(home,'service');fs.mkdirSync(folder);const now=Date.now();let clock=now,enabled=true,calls=[];
 const p=path.join(home,'data/forge-chat/73757065726167656e74000000000001');fs.mkdirSync(p,{recursive:true});fs.writeFileSync(path.join(p,'events.jsonl'),JSON.stringify({type:'user',id:'human-test',time:now,text:'以后请少打断我'})+'\n');
 const r=spawnSync('python3',['bridge/memory_store.py','--manual',home],{input:JSON.stringify({action:'save',id:'personal.fixture',expected_revision:0,title:'少打断','content':'持续推进后给结果'}),encoding:'utf8'});assert.equal(r.status,0,r.stdout);
 const timer=startMemoryMaintenance(home,folder,(...a)=>calls.push(a),()=>enabled,{now:()=>clock});
 try{timer.tick();assert.equal(calls.length,0);clock+=121000;enabled=false;timer.tick();assert.equal(calls.length,0);enabled=true;timer.tick();assert.equal(calls.length,1);assert.match(calls[0][1],/^observe_memory_/);timer.tick();assert.equal(calls.length,1);
 for(const name of ['send_message','create_session','release_session','advance_goal','record_goal','plan_followup','reply_to_question','forget_memory'])assert.equal(readOnlyTool(name,calls[0][1]),false,name);
 for(const name of ['list_memories','read_memory','review_memories','record_memory','consolidate_memories'])assert.equal(readOnlyTool(name,calls[0][1]),true,name);
 assert.equal(readOnlyTool('record_memory','observe_regular_foo'),false);
 }finally{timer.stop();fs.rmSync(home,{recursive:true,force:true});}
});
