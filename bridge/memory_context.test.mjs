import {test} from 'node:test';import assert from 'node:assert/strict';import fs from 'node:fs';import os from 'node:os';import path from 'node:path';import {spawnSync} from 'node:child_process';import {memoryContext,memorySnapshot,memoryAdditionalContext} from './memory_context.mjs';
test('stable summary keeps more than eight preferences for unrelated utterances; edits/deletes replace the entire snapshot',()=>{
 const home=fs.mkdtempSync(path.join(os.tmpdir(),'herdr-layer-memory-'));
 const edit=a=>{const r=spawnSync('python3',['bridge/memory_store.py','--manual',home],{input:JSON.stringify(a),encoding:'utf8'});assert.equal(r.status,0,r.stdout);};
 try{
 for(let i=0;i<12;i++)edit({action:'save',id:'preference.'+i,expected_revision:0,title:'协作偏好'+i,content:'核实原会话证据，主动跟进第'+i+'项'});
 const s=memorySnapshot(home);assert.equal(s.memories.length,12);assert.match(s.summary,/preference.11/);assert.match(s.summary,/preference.0/);
 assert.equal(memoryContext(home,'继续'),memoryContext(home,'这个完全无关的词'));assert.equal(memoryContext(home,'/help'),s.summary);
 assert.equal(memoryAdditionalContext(s).herdr_collaboration_memory.kind,'application');
 edit({action:'save',id:'preference.0',expected_revision:1,title:'更新偏好',content:'新的协作偏好'});
 const updated=memorySnapshot(home);assert.notEqual(updated.version,s.version);assert.match(updated.summary,/新的协作偏好/);
 edit({action:'delete',id:'preference.0',expected_revision:2});const deleted=memorySnapshot(home);assert.doesNotMatch(deleted.summary,/新的协作偏好|更新偏好/);
 // A stale derivative file cannot resurrect deleted data.
 fs.writeFileSync(path.join(home,'data/super-agent/memories.json'),JSON.stringify(s));assert.equal(memorySnapshot(home).version,deleted.version);
 assert.match(memoryAdditionalContext(null).herdr_collaboration_memory.value,/unavailable/);
 }finally{fs.rmSync(home,{recursive:true,force:true});}
});
