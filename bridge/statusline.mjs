import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const quote=s=>"'"+s.replaceAll("'","'\\''")+"'";
export function statusLineConfig(folder){return {type:'command',command:`python3 ${quote(fileURLToPath(new URL('./statusline.py',import.meta.url)))} ${quote(folder)}`,refreshInterval:30};}
export function ensureStatusLine(folder){
 try{
  const file=path.join(folder,'settings.json'),settings=JSON.parse(fs.readFileSync(file,'utf8')),next=statusLineConfig(folder);
  if(JSON.stringify(settings.statusLine)===JSON.stringify(next))return;
  settings.statusLine=next;const tmp=file+'.statusline-'+process.pid;
  fs.writeFileSync(tmp,JSON.stringify(settings),{mode:0o600});fs.renameSync(tmp,file);
 }catch{}
}
const numeric=(n,max=Number.MAX_SAFE_INTEGER)=>typeof n==='number'&&Number.isFinite(n)&&n>=0&&n<=max?n:null;
const text=s=>typeof s==='string'?s.replace(/[\x00-\x1f\x7f]/g,'').slice(0,100):'';
export function normalizeStatus(s){
 const out={type:'statusline',updated_at:numeric(s.updated_at),model:text(s.model),branch:text(s.branch)};
 for(const key of ['context_size','input_tokens','output_tokens','cache_read_tokens','cost_usd','duration_ms','api_duration_ms','lines_added','lines_removed'])out[key]=numeric(s[key]);
 out.context_pct=numeric(s.context_pct,100);
 for(const k of ['five_hour','seven_day'])out[k]={used_pct:numeric(s[k]?.used_pct,100),resets_at:numeric(s[k]?.resets_at)};
 return out;
}
export function readStatusLine(folder){try{const file=path.join(folder,'statusline.json');if(fs.statSync(file).size>16384)return null;return normalizeStatus(JSON.parse(fs.readFileSync(file,'utf8')));}catch{return null;}}

// Older interactive processes watch user settings, but not their --settings file.
// A global dispatcher preserves the user's original formatter and only collects
// for sessions registered by this app. Installation is once, not an override loop.
export function installStatusDispatcher(folder, home=process.env.HOME){
 if(!home)return;
 const base=path.join(home,'.local/share/herdr-chat');
 try{if(fs.realpathSync(path.dirname(fileURLToPath(import.meta.url)))!==fs.realpathSync(base))return;}catch{return;}
 let lock=false;
 try{
  const first=fs.openSync(path.join(folder,'events.jsonl'),'r'),buffer=Buffer.alloc(4096);let length;
  try{length=fs.readSync(first,buffer);}finally{fs.closeSync(first);}
  const e=JSON.parse(buffer.subarray(0,length).toString().split('\n')[0]);
  if(e.type!=='session'||!/^[a-f0-9-]{36}$/i.test(e.session_id))return;
  const registry=path.join(base,'statusline-sessions');fs.mkdirSync(registry,{recursive:true,mode:0o700});
  const record=path.join(registry,e.session_id+'.json');
  if(!fs.existsSync(record)||JSON.parse(fs.readFileSync(record,'utf8')).folder!==folder){const tmp=record+'.'+process.pid;fs.writeFileSync(tmp,JSON.stringify({folder}),{mode:0o600});fs.renameSync(tmp,record);}
  const marker=path.join(base,'statusline-dispatcher-installed');if(fs.existsSync(marker))return;
  fs.mkdirSync(marker+'.lock');lock=true;
  const file=path.join(home,'.claude/settings.json'),settings=JSON.parse(fs.readFileSync(file,'utf8'));
  const command=`python3 ${quote(path.join(base,'statusline.py'))} --dispatch`;
  if(settings.statusLine?.command!==command){
   const original=settings.statusLine??null;
   // Do not copy inline credentials or authorization links into a new file.
   if(/(?:sk-[A-Za-z0-9_-]{12,}|Bearer\s|https?:\/\/\S*(?:oauth|authorize))/i.test(original?.command??''))return;
   if(original?.command&&!/^(?:(?:bash|sh|python3)\s+)?(?:~\/|\/)[A-Za-z0-9_./-]+$/.test(original.command))return;
   const saved=path.join(base,'statusline-original.json');
   if(!fs.existsSync(saved))fs.writeFileSync(saved,JSON.stringify(original),{mode:0o600});
   settings.statusLine={...original,type:'command',command,refreshInterval:original?.refreshInterval??30};
   const tmp=file+'.herdr-'+process.pid;fs.writeFileSync(tmp,JSON.stringify(settings,null,2)+'\n',{mode:0o600});fs.renameSync(tmp,file);
  }
  fs.writeFileSync(marker,'1',{mode:0o600});
 }catch{}finally{if(lock)try{fs.rmdirSync(path.join(base,'statusline-dispatcher-installed.lock'));}catch{}}
}
