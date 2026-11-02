import net from 'node:net';import fs from 'node:fs';import os from 'node:os';import path from 'node:path';import {spawn,execFileSync} from 'node:child_process';import {setTimeout as delay} from 'node:timers/promises';import {fileURLToPath} from 'node:url';
const [id,cwd,model='opus']=process.argv.slice(2);if(!/^[a-f0-9]{16,64}$/.test(id??'')||!path.isAbsolute(cwd??''))process.exit(2);
const folder=path.join(os.homedir(),'.local/share/herdr-chat/conversations',id);fs.mkdirSync(folder,{recursive:true,mode:0o700});const sock=folder+'/run.sock';
async function connect(){return new Promise((resolve,reject)=>{const s=net.createConnection(sock);s.once('connect',()=>resolve(s));s.once('error',reject);});}
let socket;try{socket=await connect();}catch{
 const lock=folder+'/starting';let owner=false;
 try{fs.mkdirSync(lock);owner=true;}catch{if(Date.now()-fs.statSync(lock).mtimeMs>30000){fs.rmSync(lock,{recursive:true,force:true});fs.mkdirSync(lock);owner=true;}}
 if(owner){try{let claude=process.env.HERDR_CHAT_CLAUDE;if(!claude){try{claude=execFileSync('/bin/sh',['-c','command -v claude'],{encoding:'utf8'}).trim();}catch{}}
 const child=spawn(process.execPath,[fileURLToPath(new URL('./daemon.mjs',import.meta.url)),folder,cwd,model],{detached:true,stdio:'ignore',env:{...process.env,...(claude?{HERDR_CHAT_CLAUDE:claude}:{})}});child.unref();
 for(let i=0;i<100&&!socket;i++){try{socket=await connect();}catch{await delay(100);}}
 }finally{fs.rmSync(lock,{recursive:true,force:true});}}
 else{for(let i=0;i<100&&!socket;i++){try{socket=await connect();}catch{await delay(100);}}}
}
if(!socket)process.exit(1);socket.on('error',()=>process.exit(1));socket.on('close',()=>process.exit(0));process.stdin.pipe(socket);socket.pipe(process.stdout);process.stdin.on('end',()=>socket.end());
