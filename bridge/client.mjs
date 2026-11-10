import {imageCommand,legacyImageDisplay} from './image-client.mjs';
import {ensureStatusLine,readStatusLine,installStatusDispatcher} from './statusline.mjs';
import net from 'node:net';import fs from 'node:fs';import os from 'node:os';import path from 'node:path';import {spawn,execFileSync} from 'node:child_process';import {setTimeout as delay} from 'node:timers/promises';import {fileURLToPath} from 'node:url';import {assertInactive,transcriptPath} from './import.mjs';
const [id,cwd,model='opus',resume='']=process.argv.slice(2);if(!/^[a-f0-9]{16,64}$/.test(id??'')||!path.isAbsolute(cwd??''))process.exit(2);
const folder=path.join(process.env.HERDR_CHAT_HOME??path.join(os.homedir(),'.local/share/herdr-chat'),'conversations',id);fs.mkdirSync(folder,{recursive:true,mode:0o700});const sock=folder+'/run.sock';
if(fs.existsSync(folder+'/runtime-released.json')){process.stdout.write(JSON.stringify({type:'command_error',message:'This runtime was explicitly released. Its history is preserved; create a new chat to run again.'})+'\n');process.exit(0);}
async function connect(){return new Promise((resolve,reject)=>{const s=net.createConnection(sock);s.once('connect',()=>resolve(s));s.once('error',reject);});}
let socket;try{socket=await connect();}catch{
 if(resume){try{assertInactive(resume);transcriptPath(resume);}catch(e){process.stdout.write(JSON.stringify({type:'command_error',message:e.code==='SESSION_ACTIVE'?'Original Claude is still running. Finish handover before resuming.':'Saved conversation could not be imported from this machine.'})+'\n');process.exit(2);}}
 const lock=folder+'/starting';let owner=false;
 try{fs.mkdirSync(lock);owner=true;}catch{if(Date.now()-fs.statSync(lock).mtimeMs>30000){fs.rmSync(lock,{recursive:true,force:true});fs.mkdirSync(lock);owner=true;}}
 if(owner){try{if(fs.existsSync(folder+'/runtime-released.json'))process.exit(0);let claude=process.env.HERDR_CHAT_CLAUDE;if(!claude){try{claude=execFileSync('/bin/sh',['-c','command -v claude'],{encoding:'utf8'}).trim();}catch{}}
 const child=spawn(process.execPath,[fileURLToPath(new URL('./daemon.mjs',import.meta.url)),folder,cwd,model,resume],{detached:true,stdio:'ignore',env:{...process.env,...(claude?{HERDR_CHAT_CLAUDE:claude}:{})}});child.unref();
 for(let i=0;i<100&&!socket;i++){try{socket=await connect();}catch{await delay(100);}}
 }finally{fs.rmSync(lock,{recursive:true,force:true});}}
 else{for(let i=0;i<100&&!socket;i++){try{socket=await connect();}catch{await delay(100);}}}
}
if(!socket)process.exit(1);socket.on('error',()=>process.exit(1));socket.on('close',()=>process.exit(0));// Frame daemon output before merging telemetry: a status update must never land
// in the middle of a journal JSON record split across socket chunks.
let nativeImages=null,queued=[],incoming='';
let legacy=false,remoteStatus='',upgrading=false;
const upgradeTimer=setInterval(tryUpgrade,5000);upgradeTimer.unref();
function tryUpgrade(){
 if(!legacy||remoteStatus!=='idle'||upgrading||process.env.HERDR_SUPER_AGENT==='1')return;
 upgrading=true;const child=spawn('python3',[fileURLToPath(new URL('./upgrade_idle.py',import.meta.url)),folder],{stdio:['ignore','pipe','ignore']});let result='';child.stdout.on('data',b=>result+=b);child.on('error',finish);child.on('exit',finish);let done=false;
 function finish(){if(done)return;done=true;let outcome;try{outcome=JSON.parse(result);}catch{}if(outcome?.upgraded){socket.destroy();process.exit(0);return;}upgrading=false;const held=queued;queued=[];for(const c of held)forward(c);}
}

function writeCommand(c){if(!socket.write(JSON.stringify(c)+'\n'))process.stdin.pause();}
socket.on('drain',()=>process.stdin.resume());
function forward(c){
 if(c.action==='subscribe'){writeCommand(c);return;}
 if(nativeImages===null||upgrading){if(queued.length>=1024)throw Error('queue_limit');queued.push(c);return;}
 writeCommand(imageCommand(folder,c,nativeImages));
}
let wire='';socket.setEncoding('utf8');socket.on('data',chunk=>{wire+=chunk;let n;while((n=wire.indexOf('\n'))>=0){
 const line=wire.slice(0,n);wire=wire.slice(n+1);let e;try{e=legacyImageDisplay(folder,JSON.parse(line));}catch{process.exit(1);}
 if(e.type==='connected'){legacy=e.bridge_protocol!==2;remoteStatus=e.status;tryUpgrade();nativeImages=(e.capabilities??[]).includes('images');e.capabilities=[...new Set([...(e.capabilities??[]),'images'])];}
 if(e.type==='state')remoteStatus=e.status;
 if(!process.stdout.write(JSON.stringify(e)+'\n'))socket.pause();
 if(nativeImages!==null&&!upgrading&&queued.length){const pending=queued;queued=[];for(const c of pending){try{forward(c);}catch{process.stdout.write(JSON.stringify({type:'notice',message:'Could not prepare images. The queued message is kept for retry.'})+'\n');}}}
 }if(Buffer.byteLength(wire)>1024*1024)process.exit(1);});
process.stdout.on('drain',()=>socket.resume());process.stdin.setEncoding('utf8');process.stdin.on('data',chunk=>{incoming+=chunk;let n;while((n=incoming.indexOf('\n'))>=0){const line=incoming.slice(0,n);incoming=incoming.slice(n+1);try{forward(JSON.parse(line));}catch{process.stdout.write(JSON.stringify({type:'notice',message:'Could not prepare images. The queued message is kept for retry.'})+'\n');}}if(Buffer.byteLength(incoming)>1024*1024)process.exit(1);});process.stdin.on('end',()=>socket.end());
installStatusDispatcher(folder);
let previous='';function publishStatus(){if(process.env.HERDR_SUPER_AGENT==='1'){try{if(fs.readFileSync(folder+'/manager-provider','utf8')==='codex')return;}catch{}}ensureStatusLine(folder);const status=readStatusLine(folder);if(!status)return;const data=JSON.stringify(status);if(data!==previous){previous=data;if(!process.stdout.write(data+'\n'))socket.pause();}}
publishStatus();const timer=setInterval(publishStatus,1000);timer.unref();socket.on('close',()=>{clearInterval(timer);clearInterval(upgradeTimer);});
