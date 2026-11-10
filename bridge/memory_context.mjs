// Summary is independent of the latest utterance. Detailed memory stays behind tools.
import {spawnSync} from 'node:child_process';import path from 'node:path';import {fileURLToPath} from 'node:url';
const base=path.dirname(fileURLToPath(import.meta.url));
export function memorySnapshot(home){
 if(!home)return null;
 const r=spawnSync('python3',[path.join(base,'memory_store.py'),'--snapshot',home],{encoding:'utf8',timeout:5000,maxBuffer:4*1024*1024});
 if(r.status!==0)return null;
 try{const s=JSON.parse(r.stdout);return typeof s.version==='string'&&typeof s.summary==='string'?s:null;}catch{return null;}
}
export function memoryContext(home){return memorySnapshot(home)?.summary??'';}
export function memoryAdditionalContext(snapshot){
 return {herdr_collaboration_memory:{kind:'application',value:snapshot?.summary??'Collaboration memory is currently unavailable. Do not treat previous snapshots as current; use list_memories/read_memory to verify before applying old notes.'}};
}
