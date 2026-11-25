import test from'node:test';import assert from'node:assert/strict';import fs from'node:fs';import path from'node:path';import net from'node:net';import{spawn}from'node:child_process';import{fileURLToPath}from'node:url';import{setTimeout as delay}from'node:timers/promises';
const base=fileURLToPath(new URL('.',import.meta.url));
async function until(fn){for(let n=0;n<600;n++){if(fn())return;await delay(20);}throw Error('slow-disk fixture timed out');}
test('blocked disk cannot block subscriptions or heartbeats, acknowledge early, or execute an undurable/duplicate prompt',{skip:process.platform==='win32',timeout:30000},async()=>{
 const root=fs.realpathSync(fs.mkdtempSync('/tmp/hslow-')),service=root+'/service',folder=root+'/conversation';fs.mkdirSync(service);fs.mkdirSync(folder);
 for(const name of fs.readdirSync(base))if(/\.(mjs|py|md)$/.test(name)&&!name.endsWith('.test.mjs'))fs.copyFileSync(base+name,service+'/'+name);
 const journal=service+'/journal.mjs';fs.writeFileSync(journal,fs.readFileSync(journal,'utf8').replace('let failed=false;',`let failed=false;const sync=fs.fsyncSync;fs.fsyncSync=fd=>{if(fs.existsSync(path.dirname(workerData.journalPath)+'/failed'))throw Error('injected disk failure');const gate=path.dirname(workerData.journalPath)+'/blocked';if(fs.existsSync(gate)){fs.writeFileSync(gate+'.entered','1');while(fs.existsSync(gate))Atomics.wait(new Int32Array(new SharedArrayBuffer(4)),0,0,10);}return sync(fd);};`));
 const fake=root+'/claude';fs.writeFileSync(fake,`#!/usr/bin/env python3
import os,sys,json,subprocess,tty
if '--version' in sys.argv:print('2.1.332');sys.exit(0)
root=os.environ['SLOW_FOLDER'];sid=sys.argv[sys.argv.index('--session-id')+1] if '--session-id' in sys.argv else sys.argv[sys.argv.index('--resume')+1]
def hook(name,**extra):
 subprocess.run(['python3',os.environ['SLOW_HOOK'],root],input=json.dumps({'hook_event_name':name,'session_id':sid,'cwd':root,**extra}),text=True,check=True,stdout=subprocess.DEVNULL)
tty.setraw(0);hook('SessionStart');data=b''
while True:
 data+=os.read(0,65536)
 if data.endswith(b'\\r') and b'\\x1b[201~' in data:
  text=data.split(b'\\x1b[200~',1)[1].split(b'\\x1b[201~',1)[0].decode();data=b''
  with open(root+'/received','a') as f:f.write(text+'\\n')
  hook('UserPromptSubmit',prompt=text);hook('Stop')
`,{mode:0o700});
 let child,socket;const events=[];
 async function start(){child=spawn(process.execPath,[service+'/daemon.mjs',folder,folder,'opus'],{stdio:'ignore',env:{...process.env,HERDR_CHAT_CLAUDE:fake,SLOW_FOLDER:folder,SLOW_HOOK:service+'/hook.py'}});await until(()=>fs.existsSync(folder+'/bridge-protocol.json'));socket=net.createConnection(folder+'/run.sock');socket.on('error',()=>{});let wire='';socket.on('data',b=>{wire+=b;let n;while((n=wire.indexOf('\n'))>=0){events.push(JSON.parse(wire.slice(0,n)));wire=wire.slice(n+1);}});await new Promise(r=>socket.once('connect',r));socket.write('{"action":"subscribe","after":0}\n');await until(()=>events.some(e=>e.type==='connected'));}
 const send=id=>socket.write(JSON.stringify({action:'send',request_id:id,text:id,images:[],mode:'queue'})+'\n');
 const confirmed=id=>events.some(e=>e.type==='prompt_confirmed'&&e.id===id);
 try{
  await start();send('warmup-request');await until(()=>confirmed('warmup-request')&&events.at(-1)?.status==='idle');
  fs.writeFileSync(folder+'/blocked','1');send('slow-original-1');send('slow-original-2');await until(()=>fs.existsSync(folder+'/blocked.entered'));
  let connected=false,exposed=false;const probe=net.createConnection(folder+'/run.sock');let data='';probe.on('data',b=>{data+=b;for(const line of data.trim().split('\n')){try{const e=JSON.parse(line);if(e.type==='connected')connected=true;if(e.request_id?.startsWith('slow-original')||e.id?.startsWith('slow-original'))exposed=true;}catch{}}});await new Promise(r=>probe.once('connect',r));probe.write('{"action":"subscribe","after":0}\n');await until(()=>connected);assert.equal(exposed,false);probe.destroy();
  const count=events.filter(e=>e.type==='activity').length;await until(()=>events.filter(e=>e.type==='activity').length>count);
  assert(!events.some(e=>e.type==='ack'&&e.request_id?.startsWith('slow-original')));assert(!confirmed('slow-original-1'));assert.equal(fs.readFileSync(folder+'/received','utf8'),'warmup-request\n');
  fs.unlinkSync(folder+'/blocked');await until(()=>confirmed('slow-original-2')&&events.at(-1)?.status==='idle');
  for(const id of ['slow-original-1','slow-original-2'])send(id);await delay(200);assert.deepEqual(fs.readFileSync(folder+'/received','utf8').trim().split('\n'),['warmup-request','slow-original-1','slow-original-2']);
  socket.destroy();child.kill('SIGTERM');await new Promise(r=>child.once('exit',r));child=null;
  const saved=fs.readFileSync(folder+'/events.jsonl','utf8').trim().split('\n').map(JSON.parse);assert(saved.every((e,i)=>e.seq===i+1));assert.equal(saved.filter(e=>e.type==='prompt_confirmed').length,3);
  fs.rmSync(folder+'/bridge-protocol.json');events.length=0;await start();send('slow-original-1');await delay(1000);assert.deepEqual(fs.readFileSync(folder+'/received','utf8').trim().split('\n'),['warmup-request','slow-original-1','slow-original-2']);
  fs.writeFileSync(folder+'/failed','1');send('failed-original');await until(()=>events.some(e=>e.type==='notice'&&e.message.includes('未能可靠保存')));
  assert(!events.some(e=>e.type==='ack'&&e.request_id==='failed-original'));assert(!confirmed('failed-original'));assert(!fs.readFileSync(folder+'/received','utf8').includes('failed-original'));
 }finally{fs.rmSync(folder+'/blocked',{force:true});socket?.destroy();if(child){child.kill('SIGTERM');await new Promise(r=>child.once('exit',r));}fs.rmSync(root,{recursive:true,force:true});}
});
