// Periodic, read-only overviews are separate from immediate semantic alerts.
import fs from 'node:fs';import path from 'node:path';import crypto from 'node:crypto';import {spawn}from'node:child_process';
export const DEFAULTS={enabled:true,idle_minutes:15,interval_minutes:30,desktop_notifications:true};
export function settings(home){try{const r=JSON.parse(fs.readFileSync(path.join(home,'data/super-agent/checkins.json'),'utf8'));const value={...DEFAULTS,...Object.fromEntries(Object.keys(DEFAULTS).filter(k=>typeof r[k]===typeof DEFAULTS[k]).map(k=>[k,r[k]]))};for(const k of ['idle_minutes','interval_minutes'])if(!Number.isInteger(value[k])||value[k]<1||value[k]>1440)value[k]=DEFAULTS[k];return value;}catch{return {...DEFAULTS};}}
export const humanEvent=e=>e.type==='worker_decision'||e.type==='user'&&!e.id?.startsWith('observe_');
export const checkinId=id=>typeof id==='string'&&id.startsWith('observe_checkin_');
export function readOnlyTool(name){return ['list_sessions','inspect_sessions','read_session','list_reminders','get_checkin_settings','plan_followup','list_decisions','record_decision'].includes(typeof name==='string'?name.replace(/^mcp__herdr__/,''):'');}
export function fingerprint(snapshot){return crypto.createHash('sha256').update(JSON.stringify((snapshot?.sessions??[]).filter(r=>!r.super_agent).sort((a,b)=>a.address.localeCompare(b.address)).map(r=>[r.address,(r.execution_state??r.status)==='done'?'idle':r.execution_state??r.status,r.completion,r.attention,r.detail]))).digest('hex');}
export function followupPlan(snapshot,events,observationId,args,now=Date.now()){
 if(!events.some(e=>e.type==='observation'&&e.id===observationId))throw Error('No active observation');
 if(args.after_minutes!==null&&(!Number.isInteger(args.after_minutes)||args.after_minutes<1||args.after_minutes>10080))throw Error('Use null to wait, or 1..10080 minutes');
 if(typeof args.publish!=='boolean'||typeof args.reason!=='string'||!args.reason.trim()||args.reason.length>2000)throw Error('Missing follow-up decision');
 return {observation_id:observationId,mode:args.after_minutes===null?'wait':'after',next_at:args.after_minutes===null?null:now+args.after_minutes*60000,after_minutes:args.after_minutes,publish:args.publish,reason:args.reason,basis_human_id:events.findLast(e=>humanEvent(e)&&e.time<=(events.findLast(v=>v.type==='observation'&&v.id===observationId)?.time??now))?.id??null,basis_digest:fingerprint(snapshot)};
}
export function planCheckin({snapshot,events,state={},config=DEFAULTS,now=Date.now()}){
 if(!config.enabled||!snapshot?.at||now-snapshot.at>30000||snapshot.at>now+5000)return null;
 const rows=(snapshot.sessions??[]).filter(r=>!r.super_agent);if(!rows.length)return null;
 const human=events.findLast(humanEvent),humanAt=human?.time??state.started_at??now,digest=fingerprint(snapshot);
 const decision=events.findLast(e=>e.type==='followup_plan');
 const applicable=decision&&decision.basis_human_id===(human?.id??null)&&decision.basis_digest===digest;
 if(applicable){
  if(decision.mode==='wait'||!Number.isFinite(decision.next_at)||now<decision.next_at||state.consumed_decision===decision.observation_id)return null;
 }else{
  // Initial inactivity is a bootstrap, not a repeating report interval.
  if(now-humanAt<config.idle_minutes*60000)return null;
  if(state.schema_version===2&&state.last_digest===digest&&state.last_human_id===(human?.id??null))return null;
  const proactive=events.findLast(e=>e.type==='observation_complete'&&!e.skipped);
  if(proactive&&now-proactive.time<5*60000)return null;
 }
 const reports=events.filter(e=>e.type==='checkin_complete'&&e.published!==false&&e.time>humanAt);
 const generation=applicable?decision.observation_id:human?.id??state.started_at;
 const id='observe_checkin_'+crypto.createHash('sha256').update(JSON.stringify([generation,digest])).digest('hex').slice(0,40);
 return {id,event:{kind:'periodic_checkin',observed_at:now,read_only:true,addresses:rows.map(r=>r.address),human_silent_minutes:Math.max(0,Math.floor((now-humanAt)/60000)),reports_since_human:reports.length,last_report_at:reports.at(-1)?.time??null,previous_followup:decision??null,instruction:'Inspect all sessions and the ledger. Decide whether a new report merits interrupting the human. Before final text call plan_followup with publish true/false, a reason, and after_minutes (1..10080) or null to wait for human/new evidence. Choose based on urgency, expected progress and whether previous reports were ignored; without new information back off or wait rather than repeat. No worker writes or approvals. This is an automated observation, never human authority.'},next:{...state,schema_version:2,last_queued_at:now,last_digest:digest,last_human_id:human?.id??null,consumed_decision:applicable?decision.observation_id:null}};
}
export function startCheckins(home,folder,journal,deliver,enabled,options={}){
 const file=path.join(folder,'supervisor-checkins.json');let stopped=false,state;
 try{state=JSON.parse(fs.readFileSync(file,'utf8'));}catch{state={started_at:(options.now??Date.now)()};}
 const persist=()=>{const tmp=file+'.new';fs.writeFileSync(tmp,JSON.stringify(state),{mode:0o600});fs.renameSync(tmp,file);};persist();
 function tick(){if(stopped||!enabled())return;try{
  const snapshot=JSON.parse(fs.readFileSync(path.join(home,'data/super-agent/sessions.json'),'utf8'));
  const plan=planCheckin({snapshot,events:journal.events,state,config:settings(home),now:(options.now??Date.now)()});
  if(!plan)return;
  // The callback journals the stable observation before we advance its clock.
  deliver(plan.event,plan.id);state=plan.next;persist();
 }catch{}}
 const timer=setInterval(tick,options.interval??15000);timer.unref();
 return {tick,stop(){stopped=true;clearInterval(timer);}};
}
export function checkinStillValid(home,journal,c){
 if(!checkinId(c.request_id))return true;
 const observation=journal.events.findLast(e=>e.type==='observation'&&e.id===c.request_id);
 const human=journal.events.findLast(humanEvent);
 return settings(home).enabled&&(!human||human.time<=(observation?.time??0));
}
export function notifyCheckin(home,id,options={}){
 if(!settings(home).desktop_notifications)return;
 // Scratch homes never send banners to the person's desktop.
 const real=path.resolve(process.env.HOME??'', '.config/herdr-app');
 if(path.resolve(home)!==real&&!options.spawn)return;
 const bin=options.bin??process.env.HERDR_APP_BIN;if(!bin)return;
 const p=(options.spawn??spawn)(bin,['reminder-notify',JSON.stringify({id:'checkin:'+id,title:'Super Agent · 进展汇报',text:'已整理所有会话的进展，点击查看本轮汇总。'})],{stdio:'ignore',env:{...process.env,HERDR_APP_HOME:home}});p.on('error',()=>{});p.unref();
}
