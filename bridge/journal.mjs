import fs from 'node:fs';
import crypto from 'node:crypto';
export class Journal {
  constructor(folder) {
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
    if(complete!==data)fs.writeFileSync(this.path,complete,{mode:0o600});
  }
  append(type,data={}) {
    const e={...data,type,seq:this.events.length+1,time:Date.now()};
    const fd=fs.openSync(this.path,'a',0o600);
    try {fs.writeSync(fd,JSON.stringify(e)+'\n');fs.fsyncSync(fd);}finally{fs.closeSync(fd);}
    this.events.push(e);return e;
  }
  appendBatch(events){
    const records=events.map((e,i)=>({...e,seq:this.events.length+i+1,time:Date.now()}));
    const data=Buffer.from(records.map(e=>JSON.stringify(e)+'\n').join(''));
    const fd=fs.openSync(this.path,'a',0o600);
    try{for(let offset=0;offset<data.length;){offset+=fs.writeSync(fd,data,offset,data.length-offset);}fs.fsyncSync(fd);}finally{fs.closeSync(fd);}
    for(const record of records)this.events.push(record);return records;
  }
  accept(command) {
    const digest=crypto.createHash('sha256').update(JSON.stringify(command)).digest('hex');
    if(this.requests.has(command.request_id)) {
      if(this.requests.get(command.request_id)!==digest)throw Error('request_id_conflict');
      return false;
    }
    this.append('accepted',{request_id:command.request_id,action:command.action,digest});
    this.requests.set(command.request_id,digest);return true;
  }
  after(cursor){return this.events.filter(e=>e.seq>cursor);}
}
