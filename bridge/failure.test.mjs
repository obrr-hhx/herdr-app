import test from 'node:test';import assert from 'node:assert/strict';import fs from 'node:fs';import os from 'node:os';import net from 'node:net';import {spawn} from 'node:child_process';import {setTimeout as delay} from 'node:timers/promises';import {fileURLToPath} from 'node:url';
test('API failure becomes a visible error without retaining raw authentication details',async()=>{
 const root=fs.realpathSync(fs.mkdtempSync(os.tmpdir()+'/herdr-failure-'));const child=spawn(process.execPath,[fileURLToPath(new URL('./daemon.mjs',import.meta.url)),root,root,'opus'],{stdio:'ignore'});
 try{for(let i=0;i<100&&!fs.existsSync(root+'/run.sock');i++)await delay(20);
 const first=JSON.parse(fs.readFileSync(root+'/events.jsonl','utf8').split('\n')[0]);
 await new Promise((resolve,reject)=>{const s=net.createConnection(root+'/run.sock');s.setTimeout(2000,()=>s.destroy(Error('timeout')));s.on('error',reject);s.on('connect',()=>s.write(JSON.stringify({action:'hook',event:{hook_event_name:'StopFailure',session_id:first.session_id,cwd:root,error:'rate_limit',error_details:'sensitive-error-detail',last_assistant_message:'sensitive-error-detail'}})+'\n'));s.on('data',()=>{s.end();resolve();});});
 const text=fs.readFileSync(root+'/events.jsonl','utf8');const events=text.trim().split('\n').map(JSON.parse);assert(events.some(e=>e.type==='state'&&e.status==='error'));assert(events.some(e=>e.type==='notice'&&e.message.includes('rate_limit')));assert(!text.includes('sensitive-error-detail'));
 }finally{child.kill();await new Promise(r=>child.once('exit',r));fs.rmSync(root,{recursive:true,force:true});}
});
