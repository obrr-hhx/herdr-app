import fs from 'node:fs';import path from 'node:path';import crypto from 'node:crypto';
export function imagePaths(folder,images){
 if(images===undefined)return [];
 if(!Array.isArray(images)||images.length>4||new Set(images).size!==images.length)throw Error('invalid_images');
 return images.map(id=>{
  if(typeof id!=='string'||!/^[a-f0-9]{64}$/.test(id))throw Error('invalid_images');
  const p=path.join(fs.realpathSync(folder),'images',id+'.png');
  if(fs.realpathSync(p)!==p||!fs.statSync(p).isFile()||fs.statSync(p).size>8*1024*1024)throw Error('invalid_images');
  const b=fs.readFileSync(p);if(crypto.createHash('sha256').update(b).digest('hex')!==id)throw Error('invalid_images');
  return p;
 });
}
