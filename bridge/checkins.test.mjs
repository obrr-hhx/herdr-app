import test from 'node:test';import assert from 'node:assert/strict';import fs from 'node:fs';import os from 'node:os';import path from 'node:path';import {EventEmitter}from'node:events';
import {DEFAULTS,planCheckin,startCheckins,checkinStillValid,readOnlyTool,notifyCheckin,settings,followupPlan,fingerprint}from'./checkins.mjs';
const minute=60000,now=2000000000;
const worker={address:'t2',status:'working',execution_state:'working',completion:1,title:'Worker'};
const snapshot={at:now,sessions:[worker,{address:'manager',super_agent:true}]};
const events=[{type:'user',id:'human',time:now-16*minute}];
function fixture(){const home=fs.mkdtempSync(path.join(os.tmpdir(),'checkin-test-'));fs.mkdirSync(path.join(home,'data/super-agent'),{recursive:true});fs.writeFileSync(path.join(home,'data/super-agent/sessions.json'),JSON.stringify(snapshot));return home;}
test('inactivity starts a bounded all-session read-only report, never a human request',()=>{
 assert.equal(planCheckin({snapshot,events:[{...events[0],time:now-14*minute}],now}),null);
 const plan=planCheckin({snapshot,events,now});assert(plan.id.startsWith('observe_checkin_'));assert.equal(plan.event.read_only,true);assert.deepEqual(plan.event.addresses,['t2']);assert.equal(plan.event.human_silent_minutes,16);assert.equal(plan.event.reports_since_human,0);
 assert.equal(planCheckin({snapshot,events,state:plan.next,now:now+minute}),null);
 assert.equal(planCheckin({snapshot:{...snapshot,at:now+31*minute},events,state:plan.next,now:now+31*minute}),null,'No fixed fallback repeats without an agent decision');
});
test('human replies, recent proactive summaries, disabled reporting and stale inventories defer reports',()=>{
 for(const extra of [{type:'user',id:'reply',time:now},{type:'worker_decision',id:'button',time:now},{type:'observation_complete',time:now}])assert.equal(planCheckin({snapshot,events:[...events,extra],now}),null);

 assert.equal(planCheckin({snapshot:{...snapshot,at:now-31000},events,now}),null);
 assert.equal(planCheckin({snapshot,events,config:{...DEFAULTS,enabled:false},now}),null);
 assert.equal(planCheckin({snapshot:{at:now,sessions:[]},events,now}),null);
});
test('unchanged quiet idle inventories stay quiet; a new result still gets reported',()=>{
 const idle={...snapshot,sessions:[{...worker,status:'idle',execution_state:'idle'}]},first=planCheckin({snapshot:idle,events,now});assert(first);
 const later=now+31*minute;assert.equal(planCheckin({snapshot:{...idle,at:later},events,state:first.next,now:later}),null);
 assert(planCheckin({snapshot:{...idle,at:later,sessions:[{...idle.sessions[0],completion:2}]},events,state:first.next,now:later}));
});
test('busy manager defers, durable restart coalesces missed intervals and callbacks precede clock advancement',()=>{
 const home=fixture(),folder=path.join(home,'conversation');fs.mkdirSync(folder);const journal={events};let busy=true,calls=[];let clock=startCheckins(home,folder,journal,(event,id)=>calls.push({event,id}),()=>!busy,{now:()=>now,interval:100000});
 try{clock.tick();assert.equal(calls.length,0);busy=false;clock.tick();assert.equal(calls.length,1);clock.stop();clock=startCheckins(home,folder,journal,(e,id)=>calls.push({id}),()=>true,{now:()=>now+minute,interval:100000});clock.tick();assert.equal(calls.length,1);}finally{clock.stop();fs.rmSync(home,{recursive:true,force:true});}
});
test('an unjournaled callback failure can retry without losing the due check-in',()=>{
 const home=fixture(),folder=path.join(home,'conversation');fs.mkdirSync(folder);let attempts=0;const clock=startCheckins(home,folder,{events},()=>{if(++attempts===1)throw Error('disk');},()=>true,{now:()=>now,interval:100000});try{clock.tick();clock.tick();assert.equal(attempts,2);}finally{clock.stop();fs.rmSync(home,{recursive:true,force:true});}
});
test('stale queued check-ins yield to newer human requests and mutating tools are never allowed',()=>{
 const home=fixture();try{const command={request_id:'observe_checkin_qa'},observation={type:'observation',id:command.request_id,time:now};assert(checkinStillValid(home,{events:[observation]},command));assert(!checkinStillValid(home,{events:[observation,{type:'user',id:'new',time:now+1}]},command));
 for(const tool of ['send_message','reply_to_question','create_session','record_task','schedule_reminder','configure_checkins','Bash',undefined])assert.equal(readOnlyTool(tool),false);
 assert(readOnlyTool('mcp__herdr__inspect_sessions'));assert(readOnlyTool('list_sessions'));
 }finally{fs.rmSync(home,{recursive:true,force:true});}
});
test('banners use Herdr identity, settings can disable them and corrupt intervals cannot spam',()=>{
 const home=fixture(),calls=[];try{notifyCheckin(home,'qa',{bin:'/Herdr',spawn:(...args)=>{calls.push(args);const p=new EventEmitter();p.unref=()=>{};return p;}});assert.equal(calls[0][1][0],'reminder-notify');assert.equal(JSON.parse(calls[0][1][1]).title,'Super Agent · 进展汇报');fs.writeFileSync(path.join(home,'data/super-agent/checkins.json'),JSON.stringify({desktop_notifications:false,idle_minutes:-10,interval_minutes:0}));notifyCheckin(home,'qa',{spawn:()=>assert.fail('disabled')});assert.equal(settings(home).idle_minutes,15);assert.equal(settings(home).interval_minutes,30);}finally{fs.rmSync(home,{recursive:true,force:true});}
});

test('agent chooses a specific follow-up time, then may wait indefinitely; human/new evidence invalidates waiting',()=>{
 const obs={type:'observation',id:'observe_checkin_first',time:now};const es=[...events,obs];
 const decision={type:'followup_plan',...followupPlan(snapshot,es,obs.id,{after_minutes:7,publish:true,reason:'A run should finish shortly'},now)};
 const state={last_digest:fingerprint(snapshot),last_human_id:'human'};
 assert.equal(planCheckin({snapshot:{...snapshot,at:now+6*minute},events:[...es,decision],state,now:now+6*minute}),null);
 const due=planCheckin({snapshot:{...snapshot,at:now+7*minute},events:[...es,decision],state,now:now+7*minute});assert(due);
 assert.equal(planCheckin({snapshot:{...snapshot,at:now+8*minute},events:[...es,decision],state:due.next,now:now+8*minute}),null,'A decision is consumed once');
 const waiting={type:'followup_plan',...followupPlan(snapshot,es,obs.id,{after_minutes:null,publish:false,reason:'Only waiting for the human; do not nag'},now)};
 assert.equal(planCheckin({snapshot:{...snapshot,at:now+1440*minute},events:[...es,waiting],state,now:now+1440*minute}),null);
 assert(planCheckin({snapshot:{...snapshot,at:now+20*minute,sessions:[{...worker,completion:2}]},events:[...es,waiting],state,now:now+20*minute}));
 assert(planCheckin({snapshot:{...snapshot,at:now+20*minute},events:[...es,waiting,{type:'user',id:'new-human',time:now+minute}],state,now:now+20*minute}));
 assert(readOnlyTool('plan_followup'));assert(readOnlyTool('record_decision'));assert(readOnlyTool('curate_decision'));assert(readOnlyTool('list_decisions'));assert(!readOnlyTool('resolve_decision'));
});
test('late decisions retain the original human basis; invalid plans are rejected and view-only Done changes do not count as evidence',()=>{
 const obs={type:'observation',id:'observe_checkin_first',time:now};const es=[...events,obs,{type:'user',id:'new-human',time:now+1}];
 assert.equal(followupPlan(snapshot,es,obs.id,{after_minutes:null,publish:false,reason:'wait'},now+2).basis_human_id,'human');
 for(const args of [{after_minutes:0,publish:true,reason:'bad'},{after_minutes:10081,publish:false,reason:'bad'},{after_minutes:1,publish:true,reason:''}])assert.throws(()=>followupPlan(snapshot,es,obs.id,args,now));
 assert.equal(fingerprint({sessions:[{...worker,status:'done',execution_state:'done'}]}),fingerprint({sessions:[{...worker,status:'idle',execution_state:'idle'}]}));
});

test('a persisted wait decision survives clock restart and silent viewing does not wake it',()=>{
 const home=fixture(),folder=path.join(home,'conversation');fs.mkdirSync(folder);const obs={type:'observation',id:'observe_previous',time:now};const journal={events:[...events,obs]};journal.events.push({type:'followup_plan',...followupPlan(snapshot,journal.events,obs.id,{after_minutes:null,publish:false,reason:'Awaiting human review'},now)});
 const later=now+2*24*60*minute;fs.writeFileSync(path.join(home,'data/super-agent/sessions.json'),JSON.stringify({...snapshot,at:later}));let calls=0;const clock=startCheckins(home,folder,journal,()=>calls++,()=>true,{now:()=>later,interval:100000});try{clock.tick();assert.equal(calls,0);}finally{clock.stop();fs.rmSync(home,{recursive:true,force:true});}
});

test('legacy fixed-interval state gets one adaptive decision instead of becoming permanently stuck',()=>{
 const legacy={last_digest:fingerprint(snapshot),last_human_id:'human',last_queued_at:now-31*minute};
 const bootstrap=planCheckin({snapshot,events,state:legacy,now});assert(bootstrap);assert.equal(bootstrap.next.schema_version,2);
 assert.equal(planCheckin({snapshot:{...snapshot,at:now+60*minute},events,state:bootstrap.next,now:now+60*minute}),null);
});

test('background goals use only bounded continuation and manual review stays observational',async()=>{const {readOnlyTurn,readOnlyTool}=await import('./checkins.mjs');assert(readOnlyTurn('observe_session_result'));assert(readOnlyTool('advance_goal','observe_checkin_qa'));assert(!readOnlyTool('advance_goal','manual_review_qa'));assert(readOnlyTool('record_goal_progress','observe_result'));for(const name of ['send_message','create_session','reply_to_question','close_session','release_session'])assert(!readOnlyTool(name,'observe_result'));});

test('unfinished goals have a periodic review even after adaptive wait and unchanged sessions',async()=>{
 const {planCheckin,followupPlan}=await import('./checkins.mjs');const now=Date.now();
 const snapshot={at:now,sessions:[],goals:[{id:'goal',phase:'active',enabled:true}]};
 const events=[{type:'user',id:'human',time:now-7200000},{type:'followup_plan',observation_id:'old',mode:'wait',basis_human_id:'human',time:now-7100000}];
 const plan=planCheckin({snapshot,events,state:{started_at:now-7200000},now});assert.equal(plan.event.kind,'goal_review');assert.equal(plan.event.mandatory_report,true);assert.deepEqual(plan.event.goal_ids,['goal']);
 const observed=[...events,{type:'observation',id:plan.id,time:now,events:[plan.event]}];
 assert.throws(()=>followupPlan(snapshot,observed,plan.id,{after_minutes:null,publish:false,reason:'nothing changed'},now),/must publish/);
 assert.equal(followupPlan(snapshot,observed,plan.id,{after_minutes:null,publish:true,reason:'verified goal blockers'},now).mode,'wait');
 assert.equal(planCheckin({snapshot,events:[...observed,{type:'goal_review_complete',id:plan.id,time:now}],state:plan.next,now}),null);
 assert.equal(planCheckin({snapshot:{...snapshot,at:now+3600001},events:[...observed,{type:'goal_review_complete',id:plan.id,time:now}],state:plan.next,now:now+3600001}).event.kind,'goal_review');
 assert.equal(planCheckin({snapshot,events,state:{started_at:now-7200000},config:{enabled:false},now}),null);
});
