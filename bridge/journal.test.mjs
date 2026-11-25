import test from 'node:test';import assert from 'node:assert/strict';import fs from 'node:fs';import os from 'node:os';import {Journal,unstartedPrompts} from './journal.mjs';
test('reconnect replay is ordered and duplicate sends do not reexecute',()=>{const p=fs.mkdtempSync(os.tmpdir()+'/herdr-chat-test-');try{let j=new Journal(p);const c={action:'send',request_id:'test-command-1',text:'hello'};assert.equal(j.accept(c),true);j.append('user',{id:c.request_id,text:c.text});j=new Journal(p);assert.equal(j.accept(c),false);assert.throws(()=>j.accept({...c,text:'different'}));assert.deepEqual(j.after(1).map(e=>e.seq),[2]);fs.appendFileSync(p+'/events.jsonl','{"partial":');j=new Journal(p);assert.equal(j.events.length,2);assert.equal(j.append('state',{status:'idle'}).seq,3);}finally{fs.rmSync(p,{recursive:true,force:true});}});
test('complete corrupt records never cause silent history truncation',()=>{const p=fs.mkdtempSync(os.tmpdir()+'/herdr-chat-corrupt-');try{const j=new Journal(p);j.append('state',{status:'idle'});fs.appendFileSync(p+'/events.jsonl','invalid\n');const before=fs.readFileSync(p+'/events.jsonl');assert.throws(()=>new Journal(p));assert.deepEqual(fs.readFileSync(p+'/events.jsonl'),before);}finally{fs.rmSync(p,{recursive:true,force:true});}});

test('startup recovery restores only prompts provably not dispatched',()=>{
 const restore=events=>unstartedPrompts(events.flatMap(e=>e.type==='user'?[{type:'accepted',action:'send',request_id:e.id},e]:[e]));
 const user=id=>({type:'user',id,text:id,images:[],mode:'queue'});
 const events=[user('never-sent'),user('submitted'),{type:'state',status:'working',active_turn:'submitted'},user('confirmed'),{type:'prompt_confirmed',id:'confirmed'},user('steered')];events.at(-1).mode='steer';
 assert.deepEqual(unstartedPrompts([user('unaccepted'),{...user('import:historic'),mode:undefined}]),[]);
 assert.deepEqual(restore(events).map(c=>c.request_id),['never-sent']);
 for(const end of [{type:'accepted',action:'interrupt'},{type:'state',status:'error'},{type:'state',status:'interrupted'},{type:'state',status:'working'}])assert.deepEqual(restore([user('cancelled-or-unknown'),end]),[]);
 assert.deepEqual(restore([{type:'state',status:'interrupted'},user('new-unsent')]).map(c=>c.request_id),['new-unsent']);
});
