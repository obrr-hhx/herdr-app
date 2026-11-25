import test from 'node:test';import assert from 'node:assert/strict';import fs from 'node:fs';import os from 'node:os';import {Worker}from'node:worker_threads';import {setTimeout as delay}from'node:timers/promises';import {Journal}from'./journal.mjs';
const url=new URL('./journal.mjs',import.meta.url).href;
async function until(check){for(let n=0;n<200;n++){if(check())return;await delay(10);}throw Error('fixture timed out');}
test('slow fsync keeps the event loop alive, hides uncommitted events and groups writes in order',async()=>{
 const root=fs.mkdtempSync(os.tmpdir()+'/hjournal-');fs.writeFileSync(root+'/events.jsonl','');const shared=new SharedArrayBuffer(8),gate=new Int32Array(shared);let ticks=0;const committed=[];
 const j=new Journal(root,{async:true,onCommit:es=>committed.push(...es),createWorker:data=>new Worker(`import fs from 'node:fs';import{workerData}from'node:worker_threads';const a=new Int32Array(workerData.shared),sync=fs.fsyncSync;fs.fsyncSync=fd=>{Atomics.add(a,1,1);Atomics.wait(a,0,0);return sync(fd);};await import(${JSON.stringify(url)});`,{eval:true,workerData:{...data,shared}})});
 const timer=setInterval(()=>ticks++,5);
 try{
  for(let n=0;n<100;n++)j.append('delta',{text:String(n)});
  let flushed=false;const first=j.flush().then(()=>flushed=true);
  await until(()=>Atomics.load(gate,1)===1);await delay(40);
  assert(ticks>=4);assert.equal(flushed,false);assert.equal(committed.length,0);assert.deepEqual(j.after(0),[]);
  for(let n=0;n<100;n++)j.append('delta',{text:String(n+100)});
  const all=j.flush();Atomics.store(gate,0,1);Atomics.notify(gate,0);
  await Promise.all([first,all]);assert.equal(Atomics.load(gate,1),2);assert.equal(j.durableSeq,200);
  assert.deepEqual(committed.map(e=>e.seq),Array.from({length:200},(_,i)=>i+1));
  await j.close();const reopened=new Journal(root);assert.equal(reopened.events.length,200);
 }finally{clearInterval(timer);Atomics.store(gate,0,1);Atomics.notify(gate,0);await j.writer.terminate();fs.rmSync(root,{recursive:true,force:true});}
});
test('disk failure rejects barriers and never publishes a successful receipt',async()=>{
 const root=fs.mkdtempSync(os.tmpdir()+'/hjournal-fail-');const published=[];let failures=0;
 const j=new Journal(root,{async:true,onCommit:es=>published.push(...es),onError:()=>failures++,createWorker:data=>new Worker(`import fs from 'node:fs';fs.fsyncSync=()=>{throw Error('injected disk failure')};await import(${JSON.stringify(url)});`,{eval:true,workerData:data})});
 try{j.accept({action:'send',request_id:'disk-fail-request',text:'keep me'});j.append('user',{id:'disk-fail-request',text:'keep me'});await assert.rejects(j.flush());assert.equal(j.durableSeq,0);assert.equal(published.length,0);assert.equal(failures,1);assert.throws(()=>j.append('state',{status:'working'}));await assert.rejects(j.flush());}finally{await j.writer.terminate();fs.rmSync(root,{recursive:true,force:true});}
});
test('a torn acceptance without its payload remains retryable with the same ID',async()=>{
 const root=fs.mkdtempSync(os.tmpdir()+'/hjournal-torn-');const c={action:'send',request_id:'torn-request',text:'original text'};
 let j;try{const old=new Journal(root);old.append('state',{status:'idle'});old.accept(c);fs.appendFileSync(root+'/events.jsonl','{"type":"user"');j=new Journal(root,{async:true});assert.equal(j.events.length,1);assert.equal(j.accept(c),true);j.append('user',{id:c.request_id,text:c.text});await j.flush();await j.close();const read=new Journal(root);assert.equal(read.accept(c),false);assert.equal(read.events.filter(e=>e.type==='user').length,1);}finally{if(j?.writer)await j.writer.terminate();fs.rmSync(root,{recursive:true,force:true});}
});
