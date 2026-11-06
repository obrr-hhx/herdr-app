import test from 'node:test';import assert from 'node:assert/strict';import fs from 'node:fs';import os from 'node:os';import path from 'node:path';import crypto from 'node:crypto';import {spawnSync}from 'node:child_process';import {fileURLToPath}from 'node:url';import {imagePaths}from './images.mjs';
const png=Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j4l8AAAAASUVORK5CYII=','base64');
const id=crypto.createHash('sha256').update(png).digest('hex');
import {imageCommand,legacyImageDisplay}from './image-client.mjs';
test('old services receive images without restart; replay survives a service upgrade',()=>{
 const root=fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'herdr-image-wire-')));fs.mkdirSync(root+'/images');fs.writeFileSync(root+'/images/'+id+'.png',png);
 try{
  const c={action:'send',request_id:'image-request-001',text:'看看 🐈',images:[id]};
  const wire=imageCommand(root,c,false);assert(!wire.images);assert(wire.text.includes(root+'/images/'+id+'.png'));
  assert.deepEqual(imageCommand(root,c,true),wire);
  const display=legacyImageDisplay(root,{type:'user',id:c.request_id,text:wire.text,images:[]});assert.equal(display.text,c.text);assert.deepEqual(display.images,[id]);
  assert.throws(()=>imageCommand(root,{...c,text:'changed'},false));
  assert(!fs.readFileSync(root+'/image-requests/'+c.request_id+'.json','utf8').includes(c.text));
  const only={...c,request_id:'image-only-request',text:''};const onlyWire=imageCommand(root,only,false);assert(onlyWire.text.trim());assert.equal(legacyImageDisplay(root,{type:'user',id:only.request_id,text:onlyWire.text}).text,'');
  const native={...c,request_id:'native-request-001'};assert.deepEqual(imageCommand(root,native,true),native);assert.throws(()=>imageCommand(root,native,false));
 }finally{fs.rmSync(root,{recursive:true,force:true});}
});
test('upload is private, bounded, idempotent and cannot escape the conversation',()=>{
 const home=fs.mkdtempSync(path.join(os.tmpdir(),'herdr-images-')),conv='a'.repeat(32);
 const code=`import importlib.util,json,pathlib,sys\ns=importlib.util.spec_from_file_location('images',sys.argv[1]);m=importlib.util.module_from_spec(s);s.loader.exec_module(m)\ntry:\n print(json.dumps(m.receive(pathlib.Path(sys.argv[2]),sys.argv[3],json.load(sys.stdin))))\nexcept Exception:sys.exit(1)`;
 const upload=(images,identity=conv)=>spawnSync('python3',['-c',code,fileURLToPath(new URL('./images.py',import.meta.url)),home,identity],{input:JSON.stringify({images}),encoding:'utf8'});
 try{
  const entry={id,data:png.toString('base64')};assert.equal(upload([entry]).status,0);assert.equal(upload([entry]).status,0);
  const root=path.join(home,'.local/share/herdr-chat/conversations',conv),p=path.join(root,'images',id+'.png');
  assert.equal(fs.statSync(p).mode&0o777,0o600);assert.deepEqual(imagePaths(root,[id]),[fs.realpathSync(p)]);
  assert.notEqual(upload([entry],'../../elsewhere').status,0);assert.notEqual(upload(Array(5).fill(entry)).status,0);assert.notEqual(upload([{...entry,id:'0'.repeat(64)}]).status,0);
  assert.throws(()=>imagePaths(root,[id,id]));assert.throws(()=>imagePaths(root,['../private']));assert.throws(()=>imagePaths(root,['0'.repeat(64)]));
  fs.writeFileSync(p,'changed');assert.throws(()=>imagePaths(root,[id]));
  fs.unlinkSync(p);fs.symlinkSync('/etc/hosts',p);assert.notEqual(upload([entry]).status,0);assert.throws(()=>imagePaths(root,[id]));
 }finally{fs.rmSync(home,{recursive:true,force:true});}
});
test('image content does not ride the durable command wire',()=>{
 const command={action:'send',text:'look',images:[id]};assert(JSON.stringify(command).length<256);assert(!JSON.stringify(command).includes(png.toString('base64')));assert.deepEqual(imagePaths('/unused',undefined),[]);
});
