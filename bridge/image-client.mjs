// Adapt uploads to older interactive services without restarting their Claude.
import fs from 'node:fs';import path from 'node:path';import crypto from 'node:crypto';
import {imagePaths}from './images.mjs';
const hash=s=>crypto.createHash('sha256').update(s).digest('hex');
const suffix=paths=>'\n\nAttached images (use Read to view these image files):\n'+paths.join('\n');
function recordPath(folder,id){if(typeof id!=='string'||!/^[a-zA-Z0-9_-]{8,100}$/.test(id))throw Error('request');return path.join(folder,'image-requests',id+'.json');}
export function imageCommand(folder,c,native){
 if(c.action!=='send'||!c.images?.length)return c;
 const paths=imagePaths(folder,c.images),extra=suffix(paths);
 if(typeof c.text!=='string'||Buffer.byteLength(c.text+extra)>256*1024)throw Error('message_size');
 const file=recordPath(folder,c.request_id),facts={images:c.images,length:c.text.length,hash:hash(c.text)};
 fs.mkdirSync(path.dirname(file),{recursive:true,mode:0o700});
 let metadata={...facts,mode:native?'native':'legacy'};
 try{fs.writeFileSync(file,JSON.stringify(metadata),{flag:'wx',mode:0o600});}catch(e){
  if(e.code!=='EEXIST')throw e;metadata=JSON.parse(fs.readFileSync(file,'utf8'));
  if(JSON.stringify(metadata.images)!==JSON.stringify(facts.images)||metadata.length!==facts.length||metadata.hash!==facts.hash)throw Error('request_id_conflict');
 }
 if(metadata.mode==='native'){if(!native)throw Error('pending_service_downgrade');return c;}
 const result={...c,text:c.text+extra};delete result.images;return result;
}
export function legacyImageCommand(folder,c){return imageCommand(folder,c,false);}
export function legacyImageDisplay(folder,e){
 if(e.type!=='user'||e.images?.length||typeof e.text!=='string')return e;
 try{
  const m=JSON.parse(fs.readFileSync(recordPath(folder,e.id),'utf8'));
  if(m.mode==='native'||!Array.isArray(m.images)||m.images.length>4||!m.images.every(id=>typeof id==='string'&&/^[a-f0-9]{64}$/.test(id))||!Number.isSafeInteger(m.length)||m.length<0)return e;
  const text=e.text.slice(0,m.length),paths=m.images.map(id=>path.join(fs.realpathSync(folder),'images',id+'.png'));
  if(hash(text)!==m.hash||e.text!==text+suffix(paths))return e;
  return {...e,text,images:m.images};
 }catch{return e;}
}
