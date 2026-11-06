import {execFile} from 'node:child_process';
import {promisify} from 'node:util';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
const exec=promisify(execFile);
export const MODELS=['opus','sonnet','haiku'];
export const EFFORTS=['','low','medium','high','xhigh','max'];
export function validSettings(c){return MODELS.includes(c.model)&&EFFORTS.includes(c.effort);}
export async function workspace(cwd,home=os.homedir(),query=''){
 let names;
 try{names=(await exec('git',['ls-files','--cached','--recurse-submodules','-z'],{cwd,timeout:8000,maxBuffer:32*1024*1024})).stdout.split('\0');try{for(const p of (await exec('git',['ls-files','--others','--exclude-standard','-z'],{cwd,timeout:8000,maxBuffer:8*1024*1024})).stdout.split('\0'))names.push(p);}catch{}}
 catch{names=[];const ignored=new Set(['.git','node_modules','target','dist','.venv','vendor']);let visited=0;
  const walk=(dir,prefix='')=>{if(visited>16000)return;let rows;try{rows=fs.readdirSync(dir,{withFileTypes:true});}catch{return;}for(const d of rows){if(++visited>16000)return;if(ignored.has(d.name)||d.name==='.env'||d.name.startsWith('.env.'))continue;const relative=prefix+d.name;if(d.isDirectory())walk(path.join(dir,d.name),relative+'/');else if(d.isFile())names.push(relative);}};walk(cwd);
 }
 const all=[...new Set(names)].filter(p=>p&&p.toLowerCase().includes(query.toLowerCase())&&!path.isAbsolute(p)&&!p.split('/').includes('..')&&!p.split('/').some(n=>n==='.env'||n.startsWith('.env.')));
 const files=[],commands=[];let bytes=0;for(const p of all){bytes+=Buffer.byteLength(JSON.stringify(p))+1;if(files.length>=12000||bytes>600000)break;files.push(p);}
 for(const [base,scope]of [[path.join(home,'.claude'),'User'],[path.join(cwd,'.claude'),'Project']]){
  const walk=(dir,prefix='')=>{let rows;try{rows=fs.readdirSync(dir,{withFileTypes:true});}catch{return;}for(const d of rows){const relative=prefix+d.name;if(d.isDirectory())walk(path.join(dir,d.name),relative+'/');else if(d.isFile()&&relative.endsWith('.md')&&commands.length<200){const name=relative.slice(0,-3).replaceAll('/',':');if(/^[\p{L}\p{N}_:-]+$/u.test(name))commands.push({name,description:`${scope} command`});}}};
  walk(path.join(base,'commands'));
  try{for(const d of fs.readdirSync(path.join(base,'skills'),{withFileTypes:true})){if(fs.existsSync(path.join(base,'skills',d.name,'SKILL.md'))&&/^[\p{L}\p{N}_:-]+$/u.test(d.name))commands.push({name:d.name,description:`${scope} skill`});}}catch{}
 }
 const enabled=new Map();
 const readJson=file=>{try{return JSON.parse(fs.readFileSync(file,'utf8'));}catch{return {};}};
 const dirs=[];for(let dir=cwd;;dir=path.dirname(dir)){dirs.unshift(dir);if(path.dirname(dir)===dir)break;}
 for(const file of [path.join(home,'.claude/settings.json'),...dirs.flatMap(d=>[path.join(d,'.claude/settings.json'),path.join(d,'.claude/settings.local.json')])])for(const [name,on]of Object.entries(readJson(file).enabledPlugins??{}))enabled.set(name,on);
 const installed=readJson(path.join(home,'.claude/plugins/installed_plugins.json')).plugins??{};
 for(const [key,versions]of Object.entries(installed)){
  if(enabled.get(key)!==true||!Array.isArray(versions))continue;
  const plugin=versions.find(v=>typeof v.installPath==='string'&&(!v.projectPath||cwd===v.projectPath||cwd.startsWith(v.projectPath+'/')));if(!plugin)continue;
  const ns=key.split('@')[0];if(!/^[\p{L}\p{N}_-]+$/u.test(ns))continue;
  try{for(const d of fs.readdirSync(path.join(plugin.installPath,'commands'),{withFileTypes:true})){if(d.isFile()&&d.name.endsWith('.md'))commands.push({name:ns+':'+d.name.slice(0,-3),description:'Plugin command'});}}catch{}
  try{for(const d of fs.readdirSync(path.join(plugin.installPath,'skills'),{withFileTypes:true})){if(d.isDirectory()&&fs.existsSync(path.join(plugin.installPath,'skills',d.name,'SKILL.md')))commands.push({name:ns+':'+d.name,description:'Plugin skill'});}}catch{}
 }
 return {query,files,commands:[...new Map(commands.map(c=>[c.name,c])).values()].slice(0,200),truncated:all.length>files.length};
}
