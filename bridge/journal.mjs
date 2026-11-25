import fs from 'node:fs';
import crypto from 'node:crypto';
import path from 'node:path';
import {Worker,isMainThread,parentPort,workerData} from 'node:worker_threads';
// A persisted working state precedes every stdin write. Only a fresh Claude
// runtime with no such state can prove a queued prompt was never submitted.
export function unstartedPrompts(events){
 const queued=new Map(),accepted=new Set();
 for(const e of events){
  if(e.type==='accepted'&&e.action==='send')accepted.add(e.request_id);
  if(e.type==='user'&&accepted.has(e.id)&&e.mode!=='steer'&&typeof e.id==='string'&&typeof e.text==='string')queued.set(e.id,{action:'send',request_id:e.id,text:e.text,images:e.images??[],mode:'queue'});
  if(e.type==='state'&&e.status==='working'){
   if(e.active_turn)queued.delete(e.active_turn);else queued.clear();
  }
  if(e.type==='prompt_confirmed'||e.type==='dispatch_receipt')queued.delete(e.id);
  if(e.type==='accepted'&&e.action==='interrupt'||e.type==='state'&&['error','interrupted','interrupting'].includes(e.status))queued.clear();
 }
 return [...queued.values()];
}
export class Journal {
  constructor(folder,{async=false,onCommit=()=>{},onError=()=>{},createWorker}={}) {
    fs.mkdirSync(folder, {recursive:true,mode:0o700});
    this.path=folder+'/events.jsonl';
    this.events=[];this.requests=new Map();
    const data=fs.existsSync(this.path)?fs.readFileSync(this.path,'utf8'):'';
    const complete=data.endsWith('\n')?data:data.slice(0,data.lastIndexOf('\n')+1);
    for(const line of complete.split('\n')) {
      if(!line)continue;
      const e=JSON.parse(line);
      if(e.seq!==this.events.length+1)throw Error('journal_corruption');
      this.events.push(e);
      if(e.type==='accepted')this.requests.set(e.request_id,e.digest);
    }
    // Only an incomplete final append can be discarded. Never truncate a
    // complete malformed line or a sequence gap and silently lose history.
    let retained=complete;
    // A torn batch may end after acceptance but before the send payload. Such
    // a tail could never have crossed dispatch's durable barrier. Keep the
    // caller's original request retryable instead of replaying a false receipt.
    if(async&&this.events.at(-1)?.type==='accepted'&&this.events.at(-1)?.action==='send'){
      const orphan=this.events.pop();this.requests.delete(orphan.request_id);
      retained=complete.slice(0,complete.lastIndexOf('\n',complete.length-2)+1);
    }
    if(retained!==data)fs.truncateSync(this.path,Buffer.byteLength(retained));
    this.durableSeq=this.events.length;this.async=async;this.onCommit=onCommit;this.onError=onError;
    this.waiters=[];this.pending=[];this.pendingBytes=0;this.inflight=null;this.failure=null;this.closing=false;
    if(async){
      const data={journalPath:this.path};
      this.writer=createWorker?createWorker(data):new Worker(new URL(import.meta.url),{workerData:data});
      this.writer.on('message',message=>{
        if(message.error){this.fail(Error('journal_write_failed'));return;}
        const batch=this.inflight;
        if(!batch||message.seq!==batch.at(-1).seq){this.fail(Error('journal_writer_protocol'));return;}
        this.inflight=null;this.durableSeq=message.seq;
        this.onCommit(batch);
        const ready=this.waiters.filter(w=>w.seq<=this.durableSeq);this.waiters=this.waiters.filter(w=>w.seq>this.durableSeq);
        for(const w of ready)w.resolve();
        this.pump();
      });
      this.writer.on('error',error=>this.fail(error));
      this.writer.on('exit',()=>{if(!this.closing)this.fail(Error('journal_writer_stopped'));});
    }
  }
  append(type,data={}) {return this.appendBatch([{...data,type}])[0];}
  appendBatch(events){
    if(this.failure)throw this.failure;
    if(this.closing)throw Error('journal_closed');
    const records=events.map((e,i)=>({...e,seq:this.events.length+i+1,time:Date.now()}));
    if(!records.length)return records;
    if(this.async){
      for(const e of records)this.pending.push(e);this.pendingBytes+=Buffer.byteLength(records.map(e=>JSON.stringify(e)+'\n').join(''));
      if(!this.timer&&!this.inflight)this.timer=setTimeout(()=>{this.timer=null;this.pump();},10);
    }else{
      const data=Buffer.from(records.map(e=>JSON.stringify(e)+'\n').join(''));
      const fd=fs.openSync(this.path,'a',0o600);
      try{for(let offset=0;offset<data.length;){const n=fs.writeSync(fd,data,offset,data.length-offset);if(!n)throw Error('journal_short_write');offset+=n;}fs.fsyncSync(fd);}finally{fs.closeSync(fd);}
      this.durableSeq=records.at(-1).seq;
    }
    for(const e of records)this.events.push(e);return records;
  }
  pump(){
    if(this.failure||this.inflight||!this.pending.length)return;
    this.inflight=this.pending;this.pending=[];this.pendingBytes=0;
    try{this.writer.postMessage({records:this.inflight});}catch(error){this.fail(error);}
  }
  fail(error){
    if(this.failure)return;
    this.failure=error;clearTimeout(this.timer);this.writer?.terminate();
    for(const w of this.waiters)w.reject(error);this.waiters=[];
    this.onError(error);
  }
  flush(seq=this.events.length){
    if(this.failure)return Promise.reject(this.failure);
    if(seq<=this.durableSeq)return Promise.resolve();
    return new Promise((resolve,reject)=>this.waiters.push({seq,resolve,reject}));
  }
  async close(){await this.flush();this.closing=true;if(this.writer)await this.writer.terminate();}
  accept(command) {
    const digest=crypto.createHash('sha256').update(JSON.stringify(command)).digest('hex');
    if(this.requests.has(command.request_id)) {
      if(this.requests.get(command.request_id)!==digest)throw Error('request_id_conflict');
      return false;
    }
    this.append('accepted',{request_id:command.request_id,action:command.action,digest});
    this.requests.set(command.request_id,digest);return true;
  }
  after(cursor){return this.events.filter(e=>e.seq>cursor&&e.seq<=this.durableSeq);}
}

// A dedicated thread owns blocking disk operations. Only a successful fsync
// advances the durable cursor; a failed/partial batch is never acknowledged.
if(!isMainThread&&workerData?.journalPath){
 let failed=false;
 parentPort.on('message',({records})=>{
  if(failed)return;
  let fd;
  try{
   const fresh=!fs.existsSync(workerData.journalPath);
   fd=fs.openSync(workerData.journalPath,'a',0o600);
   const data=Buffer.from(records.map(e=>JSON.stringify(e)+'\n').join(''));
   for(let offset=0;offset<data.length;){const n=fs.writeSync(fd,data,offset,data.length-offset);if(!n)throw Error('journal_short_write');offset+=n;}
   const marker=path.join(path.dirname(workerData.journalPath),'has-session');
   const started=records.some(e=>e.type==='hook'&&e.event?.hook_event_name==='UserPromptSubmit');
   const newMarker=started&&!fs.existsSync(marker);
   if(newMarker){const m=fs.openSync(marker,'w',0o600);try{fs.writeSync(m,'1');fs.fsyncSync(m);}finally{fs.closeSync(m);}}
   fs.fsyncSync(fd);
   // Windows does not expose directory fsync through Node's file API.
   if((fresh||newMarker)&&process.platform!=='win32'){
    const dir=fs.openSync(path.dirname(workerData.journalPath),'r');
    try{fs.fsyncSync(dir);}finally{fs.closeSync(dir);}
   }
   parentPort.postMessage({seq:records.at(-1).seq});
  }catch{failed=true;parentPort.postMessage({error:true});}
  finally{if(fd!==undefined)fs.closeSync(fd);}
 });
}
