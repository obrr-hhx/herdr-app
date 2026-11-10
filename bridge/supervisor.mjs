// Only semantic changes wake the manager. Heartbeats and UI seen/idle do not.
import crypto from 'node:crypto';import fs from 'node:fs';import path from 'node:path';
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
// Only actionable state changes wake continuation; fresh timestamps/summaries do not.
export function goalVersion(goals){return crypto.createHash('sha256').update(JSON.stringify(goals.map(g=>[g.id,g.enabled,g.mandate,g.plan,g.phase,g.needs_user,Object.entries(g.milestones??{}).map(([key,value])=>[key,value.achieved]).sort()]))).digest('hex');}
export function quietGoalWorkers(before,goals,rows,now=Date.now()){
 const next={},events=[];const addresses=new Map();
 for(const g of goals)if(g.enabled&&!g.needs_user&&g.phase!=='observed_complete')for(const step of g.plan??[])if(!addresses.has(step.address))addresses.set(step.address,{goal_id:g.id,step_id:step.id,binding:step.binding});
 for(const [address,ref] of addresses){const row=rows.find(r=>r.address===address);if(!row||row.super_agent||row.attention||!['idle','done'].includes(row.execution_state??row.status)||/reconnect|disconnect|connecting|error|interrupted/i.test(row.detail??''))continue;
 const binding={conversation:row.conversation??null,host:row.host??null,read_identity:row.read_identity??null};if(!['conversation','host','read_identity'].every(k=>JSON.stringify(binding[k])===JSON.stringify(ref.binding?.[k]??null)))continue;
 const prev=before[address],completion=row.completion??0;const same=prev&&JSON.stringify(prev.binding)===JSON.stringify(binding)&&prev.completion===completion;
 const value={binding,completion,since:same?prev.since:now,last_check:same?prev.last_check??0:0};
 if(now-value.since>=15*60000&&now-value.last_check>=30*60000){events.push({kind:'goal_session_quiet',address,...ref,quiet_since:value.since,instruction:'Read fresh evidence for the original worker, then ask_goal_session(goal_id,step_id) if it is still quiet and needs no human approval. This is a progress question, not a replacement task or a grant. Reuse the original session; do not resend unknown deliveries. Update the goal from its reply; if it needs human input, surface the full decision.'});value.last_check=now;}
 next[address]=value;
 }
 return {next,events};
}
export function supervise(folder,deliver,enabled){
 const app=process.env.HERDR_APP_HOME;if(!app)return;
 const stateFile=path.join(folder,'supervisor-cursors.json');let state={};
 try{state=JSON.parse(fs.readFileSync(stateFile,'utf8'));}catch{}
 const timer=setInterval(()=>{
  const canDeliver=enabled();
  try{
   const snapshot=JSON.parse(fs.readFileSync(path.join(app,'data/super-agent/sessions.json'),'utf8'));
   if(Date.now()-snapshot.at>30000)return;
   const {next,events}=changes(state,snapshot.sessions);
   try{const goals=JSON.parse(fs.readFileSync(path.join(app,'data/super-agent/goals.json'),'utf8')).goals??[];
    const quiet=quietGoalWorkers(state.__quiet??{},goals,snapshot.sessions);next.__quiet=quiet.next;if(!canDeliver)for(const [a,v] of Object.entries(quiet.next))v.last_check=state.__quiet?.[a]?.last_check??0;events.push(...quiet.events);const watch=path.join(app,'data/super-agent/goal-session-watch.json');fs.writeFileSync(watch+'.new',JSON.stringify({at:Date.now(),workers:quiet.next}),{mode:0o600});fs.renameSync(watch+'.new',watch);
    if(goals.length){const version=goalVersion(goals);if(version!==state.__goals){events.push({kind:'goal_plan_changed',version});next.__goals=version;}}
   }catch{}
   if(!canDeliver){state.__quiet=next.__quiet??state.__quiet;const tmp=stateFile+'.new';fs.writeFileSync(tmp,JSON.stringify(state),{mode:0o600});fs.renameSync(tmp,stateFile);return;}
   if(events.length){
    // The callback durably queues this before advancing the source cursor.
    deliver(events);
   }
   const tmp=stateFile+'.new';fs.writeFileSync(tmp,JSON.stringify(next),{mode:0o600});fs.renameSync(tmp,stateFile);state=next;
  }catch{}
 },5000);timer.unref();return timer;
}
