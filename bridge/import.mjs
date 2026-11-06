import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
const uuid=/^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i;
export function configRoot(){return process.env.CLAUDE_CONFIG_DIR??path.join(os.homedir(),'.claude');}
export function assertInactive(id,root=configRoot()){
  if(!uuid.test(id))throw Error('invalid_session_id');
  const sessions=path.join(root,'sessions');
  if(!fs.existsSync(sessions))return;
  for(const name of fs.readdirSync(sessions)){
    if(!/^\d+\.json$/.test(name))continue;
    let m;try{m=JSON.parse(fs.readFileSync(path.join(sessions,name),'utf8'));}catch{continue;}
    if(m.sessionId!==id||!Number.isSafeInteger(m.pid)||m.pid<=0)continue;
    let alive=true;
    try{process.kill(m.pid,0);}catch(e){alive=e.code==='EPERM';}
    if(!alive)continue;
    try{const stat=fs.readFileSync(`/proc/${m.pid}/stat`,'utf8');const fields=stat.slice(stat.lastIndexOf(')')+1).trim().split(/\s+/);
      if(fields[0]==='Z'||m.procStart!==undefined&&String(m.procStart)!==fields[19])continue;
    }catch{}
    const error=Error('session_active');error.code='SESSION_ACTIVE';throw error;
  }
}
export function transcriptPath(id,root=configRoot()){
  if(!uuid.test(id))throw Error('invalid_session_id');
  const projects=path.join(root,'projects'),found=[];
  for(const d of fs.readdirSync(projects,{withFileTypes:true})){
    if(!d.isDirectory())continue;
    const file=path.join(projects,d.name,id+'.jsonl');if(fs.existsSync(file)&&fs.statSync(file).isFile())found.push(file);
  }
  if(found.length!==1)throw Error('ambiguous_or_missing_transcript');return found[0];
}
function clean(s){return s.replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g,'').replace(/\x1b\[[0-?]*[ -/]*[@-~]/g,'').replace(/[\x00-\x08\x0b-\x1f\x7f]/g,'').replace(/https?:\/\/[^\s<>"']*(?:oauth|authorize)[^\s<>"']*/gi,'[authorization URL redacted]').replace(/\bsk-[A-Za-z0-9_-]{16,}/g,'[credential redacted]').replace(/\b(?:gh[opusr]_|github_pat_)[A-Za-z0-9_]{20,}/g,'[credential redacted]').replace(/\bxox[abceoprs]-[A-Za-z0-9-]{10,}/g,'[credential redacted]').replace(/Bearer [A-Za-z0-9._~-]{8,}/gi,'Bearer [credential redacted]');}
function plain(content){if(typeof content==='string')return clean(content);if(!Array.isArray(content))return '';return content.map(b=>b.type==='text'?clean(b.text??''):b.type==='image'?'[image attachment retained in the original conversation]':'').filter(Boolean).join('\n');}
function safeInput(input){if(typeof input==='string')return clip(clean(input));if(Array.isArray(input))return input.map(safeInput);if(input&&typeof input==='object')return Object.fromEntries(Object.entries(input).map(([k,v])=>[k,/^(access_token|refresh_token|api[_-]?key|password|authorization)$/i.test(k)?'[credential redacted]':safeInput(v)]));return input??{};}
function clip(s){return s.length>120000?s.slice(0,120000)+'\n[Long tool output abbreviated; full output remains in the original transcript.]':s;}
export function transcriptEvents(text,onMalformed){
  const events=[];
  for(const [lineIndex,line]of text.split('\n').entries()){
    if(!line.trim())continue;let r;try{r=JSON.parse(line);}catch(e){if(!onMalformed)throw e;onMalformed(lineIndex+1);continue;}
    if(!['user','assistant'].includes(r.type)||r.isMeta||r.isSidechain)continue;
    const m=r.message;if(!m||!['user','assistant'].includes(m.role))continue;
    const content=typeof m.content==='string'?[{type:'text',text:m.content}]:m.content;
    if(!Array.isArray(content))continue;
    for(const [index,b]of content.entries()){
      const id=`import:${r.uuid??lineIndex}:${index}`;
      if(b.type==='text'&&typeof b.text==='string'&&b.text.trim()){
        const text=clean(b.text);
        // Keep all conversational text while bounding individual transport frames.
        for(let i=0;i<text.length;i+=120000){const part=text.slice(i,i+120000);
          events.push(m.role==='user'?{type:'user',id:id+(i?`:${i}`:''),text:part}:{type:'delta',id,index:i/120000,kind:'text_delta',text:part});}
      }
      if(b.type==='tool_use')events.push({type:'tool',id:b.id??id,name:b.name??'Tool',input:safeInput(b.input)});
      if(b.type==='tool_result')events.push({type:'tool_result',id:b.tool_use_id??id,content:clip(plain(b.content)),is_error:!!b.is_error});
      if(b.type==='image')events.push({type:'user',id,text:'[image attachment retained in the original conversation]'});
    }
  }
  return events;
}
export function importTranscript(journal,id,root=configRoot()){
  assertInactive(id,root);
  const previous=journal.events.find(e=>e.type==='session');if(previous&&previous.session_id!==id)throw Error('different_import_session');
  if(journal.events.some(e=>e.type==='import_complete')){fs.writeFileSync(path.join(path.dirname(journal.path),'has-session'),'1',{mode:0o600});return;}
  const skipped=[];
  const events=transcriptEvents(fs.readFileSync(transcriptPath(id,root),'utf8'),line=>skipped.push(line));
  if(skipped.length)events.push({type:'notice',id:'import:malformed-records',message:`History import skipped ${skipped.length} malformed source record(s). The original transcript is unchanged; some history may be missing.`});
  // An interrupted import is repaired by completing deterministic event IDs on retry.
  const seen=new Set(journal.events.map(e=>`${e.type}:${e.id}:${e.index??''}`));
  if(!journal.events.some(e=>e.type==='session'))journal.append('session',{session_id:id,backend:'interactive',resumed:true});
  const pending=events.filter(e=>!seen.has(`${e.type}:${e.id}:${e.index??''}`));
  pending.push({type:'import_complete',session_id:id,records:events.length,skipped_lines:skipped});
  journal.appendBatch(pending);
  fs.writeFileSync(path.join(path.dirname(journal.path),'has-session'),'1',{mode:0o600});
}
