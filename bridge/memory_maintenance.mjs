// Separate, quiet maintenance turns. Never wakes or dispatches workers.
import fs from 'node:fs';import path from 'node:path';import crypto from 'node:crypto';import {memorySnapshot} from './memory_context.mjs';
export const memoryTurn=id=>typeof id==='string'&&id.startsWith('observe_memory_');
export function startMemoryMaintenance(home,folder,deliver,enabled,options={}){
 let stopped=false,lastAttempt=0;const file=path.join(folder,'memory-maintenance.json');
 try{lastAttempt=JSON.parse(fs.readFileSync(file,'utf8')).lastAttempt??0;}catch{}
 function tick(){
  if(stopped||!enabled())return;
  const now=(options.now??Date.now)();if(now-lastAttempt<10*60000)return;
  const snapshot=memorySnapshot(home);if(!snapshot)return;
  let latestHuman=0;
  try{const text=fs.readFileSync(path.join(home,'data/forge-chat/73757065726167656e74000000000001/events.jsonl'),'utf8');
   for(const line of text.trim().split('\n')){try{const e=JSON.parse(line);if(e.type==='user'&&!e.id?.startsWith('observe_'))latestHuman=Math.max(latestHuman,e.time??0);}catch{}}
  }catch{}
  if(latestHuman&&now-latestHuman<120000)return;
  const feedback=latestHuman>snapshot.reviewed_through;
  if(snapshot.compiled&&!feedback||!snapshot.memories.length&&!feedback)return;
  const id='observe_memory_'+crypto.createHash('sha256').update(snapshot.version+':'+latestHuman+':'+Math.floor(now/600000)).digest('hex').slice(0,28);
  deliver({kind:'memory_maintenance',version:snapshot.version,pending_feedback:feedback},id);
  lastAttempt=now;fs.writeFileSync(file,JSON.stringify({lastAttempt}),{mode:0o600});
 }
 const timer=setInterval(tick,options.interval??60000);timer.unref();
 return {tick,stop(){stopped=true;clearInterval(timer);}};
}
