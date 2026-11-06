// Deliver persisted reminders without a model request or a live Forge connection.
import {spawn} from 'node:child_process';import {fileURLToPath} from 'node:url';
const store=fileURLToPath(new URL('./schedule_store.py',import.meta.url));
export function scheduleCall(mode,home,args={}){
 return new Promise((resolve,reject)=>{
  const p=spawn('python3',[store,mode,home],{stdio:['pipe','pipe','ignore']});let out='';
  const timer=setTimeout(()=>{p.kill();reject(Error('reminder_store_timeout'));},15000);
  p.on('error',()=>{clearTimeout(timer);reject(Error('reminder_store_unavailable'));});
  p.stdout.on('data',b=>{out+=b;if(Buffer.byteLength(out)>2*1024*1024){p.kill();reject(Error('reminder_store_frame'));}});
  p.on('exit',code=>{clearTimeout(timer);try{if(code)throw Error('store');resolve(JSON.parse(out));}catch{reject(Error('reminder_store_unavailable'));}});
  p.stdin.on('error',()=>{});p.stdin.end(JSON.stringify(args));
 });
}
export function startScheduler(home,journal,emit,options={}){
 const known=new Set(),acknowledged=new Set();let busy=false,stopped=false,failed=false;
 for(const e of journal.events){if(e.type==='reminder')known.add(e.id);if(e.type==='reminder_ack'){known.delete(e.id);acknowledged.add(e.id);}}
 const call=options.call??scheduleCall;
 async function tick(){
  if(busy||stopped)return;busy=true;
  try{
   const r=await call('--poll',home,{known:[...known]});
   if(stopped)return;
   for(const d of r.deliveries??[]){if(known.has(d.delivery_id)||acknowledged.has(d.delivery_id))continue;emit('reminder',{...d,id:d.delivery_id});known.add(d.delivery_id);}
   for(const id of r.acknowledged??[]){if(!known.has(id))continue;emit('reminder_ack',{id});known.delete(id);acknowledged.add(id);}
   failed=false;
  }catch{if(!failed&&!stopped){failed=true;emit('notice',{message:'Reminder clock temporarily unavailable. Stored reminders will retry; they have not been discarded.'});}}
  finally{busy=false;}
 }
 const timer=setInterval(tick,options.interval??5000);timer.unref();tick();
 return {tick,stop(){stopped=true;clearInterval(timer);}};
}
