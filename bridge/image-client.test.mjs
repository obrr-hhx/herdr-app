import test from 'node:test';import assert from 'node:assert/strict';import fs from 'node:fs';import net from 'node:net';import path from 'node:path';import crypto from 'node:crypto';import {spawn}from 'node:child_process';import {fileURLToPath}from 'node:url';import {setTimeout as delay}from 'node:timers/promises';
test('client waits for capabilities and projects legacy images through reconnect history',{skip:process.platform==='win32'},async()=>{
 const home=fs.mkdtempSync('/tmp/hc-'),id='a'.repeat(32),folder=path.join(home,'.local/share/herdr-chat/conversations',id);fs.mkdirSync(folder+'/images',{recursive:true});
 const png=Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j4l8AAAAASUVORK5CYII=','base64'),image=crypto.createHash('sha256').update(png).digest('hex');fs.writeFileSync(folder+'/images/'+image+'.png',png);
 // Legacy bridge sockets must fit both Linux and macOS sockaddr_un limits.
 assert(Buffer.byteLength(folder+'/run.sock')<100);
 const server=net.createServer();await new Promise(r=>server.listen(folder+'/run.sock',r));let child,peer,output='',wire='',commands=[];
 try{
  const connected=new Promise(r=>server.once('connection',s=>{peer=s;s.setEncoding('utf8');s.on('data',b=>{wire+=b;let n;while((n=wire.indexOf('\n'))>=0){commands.push(JSON.parse(wire.slice(0,n)));wire=wire.slice(n+1);}});r();}));
  child=spawn(process.execPath,[fileURLToPath(new URL('./client.mjs',import.meta.url)),id,home,'opus'],{env:{...process.env,HOME:home},stdio:['pipe','pipe','pipe']});child.stdout.setEncoding('utf8');child.stdout.on('data',b=>output+=b);
  child.stdin.write(JSON.stringify({action:'subscribe',after:0})+'\n'+JSON.stringify({action:'send',request_id:'image-client-request',text:'look',images:[image]})+'\n');
  await connected;await delay(100);assert.equal(commands.length,1);
  peer.write(JSON.stringify({type:'connected',status:'idle',capabilities:['workspace']})+'\n');
  for(let i=0;i<100&&commands.length<2;i++)await delay(10);
  assert.equal(commands.length,2);assert(!commands[1].images);assert(commands[1].text.includes(image+'.png'));
  peer.write(JSON.stringify({type:'user',seq:1,id:'image-client-request',text:commands[1].text})+'\n');await delay(50);
  const records=output.trim().split('\n').map(JSON.parse);assert(records.find(e=>e.type==='connected').capabilities.includes('images'));const user=records.find(e=>e.type==='user');assert.equal(user.text,'look');assert.deepEqual(user.images,[image]);
 }finally{peer?.destroy();child?.kill();server.close();fs.rmSync(home,{recursive:true,force:true});}
});
