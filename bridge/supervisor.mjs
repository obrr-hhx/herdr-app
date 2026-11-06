// Only semantic changes wake the manager. Heartbeats and UI seen/idle do not.
import fs from 'node:fs';import path from 'node:path';
export function changes(before,rows,now=Date.now()){
 const next={},events=[];
 for(const a of rows){if(a.super_agent)continue;
  const prev=before[a.address];
  const reported=a.execution_state??a.status;const state=reported==='done'?'idle':reported;
  const connection=/reconnect|connecting|disconnected|SSH/i.test(a.detail??'');
  const issue=connection?'connection':a.detail==='interrupted'||a.detail==='error'?a.detail:null;
  const value={state,attention:a.attention??null,completion:a.completion??0,issue,title:a.title,issue_since:prev?.issue===issue?prev.issue_since??now:now,reported:prev?.issue===issue?prev.reported:null};next[a.address]=value;
  if(connection&&now-value.issue_since>=30000&&value.reported!=='connection'){events.push({kind:'connection_lost',address:a.address,title:a.title,epoch:value.issue_since});value.reported='connection';}
  else if(!connection&&prev?.issue==='connection'&&prev.reported==='connection')events.push({kind:'connection_restored',address:a.address,title:a.title,epoch:prev.issue_since});
  if(!prev)events.push({kind:'discovered',address:a.address,title:a.title,status:a.status});
  else if(state==='idle'&&(value.completion>prev.completion||prev.state==='working'))events.push({kind:'result',address:a.address,title:a.title,completion:value.completion,seq:a.seq});
  else if(state==='blocked'&&(prev.state!=='blocked'||value.attention!==prev.attention))events.push({kind:'needs_user',address:a.address,title:a.title,attention:value.attention,seq:a.seq});
  else if(value.issue && value.issue!=='connection' && prev.issue!==value.issue)events.push({kind:'execution_issue',address:a.address,title:a.title,detail:value.issue,seq:a.seq});
 }
 // A missing row can be a temporarily offline machine, so retain its cursor.
 return {next:{...before,...next},events};
}
export function supervise(folder,deliver,enabled){
 const app=process.env.HERDR_APP_HOME;if(!app)return;
 const stateFile=path.join(folder,'supervisor-cursors.json');let state={};
 try{state=JSON.parse(fs.readFileSync(stateFile,'utf8'));}catch{}
 const timer=setInterval(()=>{
  if(!enabled())return;
  try{
   const snapshot=JSON.parse(fs.readFileSync(path.join(app,'data/super-agent/sessions.json'),'utf8'));
   if(Date.now()-snapshot.at>30000)return;
   const {next,events}=changes(state,snapshot.sessions);
   if(events.length){
    // The callback durably queues this before advancing the source cursor.
    deliver(events);
   }
   const tmp=stateFile+'.new';fs.writeFileSync(tmp,JSON.stringify(next),{mode:0o600});fs.renameSync(tmp,stateFile);state=next;
  }catch{}
 },5000);timer.unref();return timer;
}
