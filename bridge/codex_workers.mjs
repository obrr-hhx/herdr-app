import fs from 'node:fs';import path from 'node:path';import {spawn} from 'node:child_process';import {fileURLToPath} from 'node:url';
const source=fileURLToPath(new URL('./codex_workers.py',import.meta.url));
export function workerAnswer(args){return new Promise((resolve,reject)=>{
 const p=spawn('python3',['-c',"import json,sys;from codex_workers import control;print(json.dumps(control('answer',json.load(sys.stdin)),ensure_ascii=False))"],{cwd:path.dirname(source),stdio:['pipe','pipe','ignore']});let out='';const timer=setTimeout(()=>{p.kill();reject(Error('Codex decision delivery unconfirmed; read the worker before retrying'));},13000);
 p.on('error',()=>{clearTimeout(timer);reject(Error('Codex session bridge unavailable'));});p.stdout.on('data',b=>out+=b);p.on('exit',code=>{clearTimeout(timer);try{const r=JSON.parse(out);if(code||r.error)throw Error('reply');resolve(r);}catch{reject(Error('Codex question changed, disconnected or already resolved. Nothing was automatically retried.'));}});p.stdin.on('error',()=>{});p.stdin.end(JSON.stringify(args));
});}
export function startCodexWorkers(home,emit,journal={events:[]},deps={spawn}){
 let child=null,stopped=false;const displayed=new Map();
 for(const e of journal.events){if(e.type==='permission'&&e.source==='codex_worker')displayed.set(e.id,e);if(e.type==='permission_resolved')displayed.delete(e.id);}
 function start(){if(stopped||child)return;child=deps.spawn('python3',[source],{stdio:['ignore','pipe','ignore']});child.stdout.on('data',()=>sync());child.on('error',()=>{});child.on('exit',()=>{child=null;});}
 function sync(){
  if(stopped)return;start();
  try{
   const state=JSON.parse(fs.readFileSync(path.join(home,'data/super-agent/codex-workers.json'),'utf8'));
   if(Date.now()-state.at>15000)return;
   const current=new Map();
   for(const [address,row]of Object.entries(state.sessions??{})){
    for(const q of Object.values(row.pending_questions??{})){
     if(!q.reply_supported)continue;current.set(q.id,{...q,address,source:'codex_worker',source_label:`${address} · ${row.title??'Codex'}`});
    }
   }
   for(const [id,q]of current)if(!displayed.has(id)){emit('permission',q);displayed.set(id,q);}
   for(const [id,old]of displayed){const row=state.sessions?.[old.address];if(!current.has(id)&&row?.binding_live&&row.connected){emit('permission_resolved',{id,decision:'resolved_by_runtime'});displayed.delete(id);}}
  }catch{}
 }
 const timer=setInterval(sync,1500);timer.unref();sync();
 return {sync,ids:()=>[...displayed.keys()],has:id=>displayed.has(id),get:id=>displayed.get(id),stop(){stopped=true;clearInterval(timer);child?.kill();}};
}
