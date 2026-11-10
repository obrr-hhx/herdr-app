import {startMemoryMaintenance,memoryTurn} from './memory_maintenance.mjs';
import {memorySnapshot} from './memory_context.mjs';
import {Steering,unconfirmedSteering} from './steering.mjs';
import {codexRunner} from './codex.mjs';
import {startCheckins,checkinId,checkinStillValid,readOnlyTool,manualReviewId,readOnlyTurn,notifyCheckin,followupPlan} from './checkins.mjs';
import {supervise} from './supervisor.mjs';
import {startScheduler,scheduleCall} from './scheduler.mjs';
import {startCodexWorkers,workerAnswer} from './codex_workers.mjs';
import net from 'node:net';
import fs from 'node:fs';
import path from 'node:path';
import readline from 'node:readline';
import crypto from 'node:crypto';
import {spawn} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {imagePaths} from './images.mjs';
import {Journal} from './journal.mjs';
import {Display} from './display.mjs';
import {importTranscript,assertInactive} from './import.mjs';
import {workspace,validSettings} from './workspace.mjs';
import {statusLineConfig} from './statusline.mjs';
const [folder,requestedCwd,model='opus',resume='']=process.argv.slice(2);
const cwd=fs.realpathSync(requestedCwd);
if(!folder||!path.isAbsolute(cwd))process.exit(2);
const journal=new Journal(folder);const existing=journal.events.length>0;
if(resume)importTranscript(journal,resume);
const peers=new Set(),pending=new Map();
const session=journal.events.findLast(e=>e.type==='session')?.session_id??crypto.randomUUID();
let lastProgress=Date.now(),phase='starting',lastTool='';
let permissionMode=process.env.HERDR_CHAT_LOCAL==='1'?'configured':'bypassPermissions';
const steering=new Steering();let steerSupported=false;
let cli=null,ready=false,busy=false,status='starting',current=null,turnRequestId=null,queue=[],restarting=false,shuttingDown=false;
let codexModels=[];let goalReviewTools=new Set();
const markReviewTool=name=>goalReviewTools.add(String(name??'').replace(/^mcp__herdr__/,''));
let runtime=journal.events.findLast(e=>e.type==='settings')??{model,effort:''};
const manager=process.env.HERDR_SUPER_AGENT==='1';
let claudeMemoryVersion=null;
if(manager&&!runtime.provider)runtime={model:'default',effort:'',provider:'codex'};
let reminderClock=null,codexWorkers=null,checkinClock=null,memoryClock=null,checkinDecision=null,checkinOutput=[];
const emit=(type,data={})=>{if(memoryTurn(turnRequestId)&&['delta','message_complete','tool','tool_result'].includes(type))return null;if(checkinId(turnRequestId)&&['delta','message_complete','tool','tool_result'].includes(type)){if(checkinDecision?.publish===false)return null;if(!checkinDecision){checkinOutput.push([type,data]);return null;}}const e=journal.append(type,data),line=JSON.stringify(e)+'\n';for(const p of peers){if(p.writableLength>8*1024*1024)p.destroy();else p.write(line);}return e;};
if(existing)emit('state',{status:'interrupted',message:'Service restarted. Previously accepted prompts were not automatically submitted again.'});
if(existing)finishReviews('interrupted');
if(existing)for(const id of unconfirmedSteering(journal.events))emit('dispatch_receipt',{id,mode:'steer',stage:'unknown',reason:'Bridge restarted before Claude confirmed this input. Inspect the same session; it was not replayed.'});
emit('session',{session_id:session,model,backend:'interactive'});
const normalized=new Set(journal.events.filter(e=>e.type==='delta').map(e=>`${e.id}:${e.index}`));
const complete=new Set(journal.events.filter(e=>e.type==='message_complete').map(e=>e.id));
const display=new Display((type,data)=>{
 const key=`${data.id}:${data.index}`;
 if(type==='delta'){if(normalized.has(key))return;normalized.add(key);}
 if(type==='message_complete'){if(complete.has(data.id))return;complete.add(data.id);}
 emit(type,data);
});
const oldPending=new Set();
for(const e of [...journal.events]){
 if(e.type==='permission'&&e.source!=='codex_worker')oldPending.add(e.id);
 if(e.type==='permission_resolved')oldPending.delete(e.id);
 if(e.type==='hook'&&e.event.hook_event_name==='MessageDisplay')display.accept(e.event);
}
for(const id of oldPending)emit('permission_resolved',{id,decision:'cancelled'});
function capabilities(){return ['workspace','settings','compact','images',...(!manager&&steerSupported?['steer']:[]),...(manager?['session_review']:[])];}
function activity(){return {active_turn:turnRequestId,phase,last_progress:lastProgress,tool:lastTool,permission_mode:manager?'managementTools':permissionMode};}
function progress(next,tool=''){lastProgress=Date.now();phase=next;lastTool=tool;}
function finishReviews(outcome){for(const e of journal.events.filter(e=>e.type==='review_requested'))if(!journal.events.some(r=>r.type==='review_complete'&&r.id===e.id))emit('review_complete',{id:e.id,outcome});}
function state(s){status=s;if(['error','interrupted','interrupting'].includes(s))finishReviews(s);if(s!=='working')progress(s);if((memoryTurn(turnRequestId)||checkinId(turnRequestId)&&!checkinDecision?.publish)&&s==='working')return;emit('state',{status:s,active_turn:turnRequestId});}
const activityTimer=setInterval(()=>{for(const p of peers)if(p.writableLength<1024*1024)p.write(JSON.stringify({type:'activity',...activity()})+'\n');},5000);activityTimer.unref();
function cancelPermissions(){for(const [id,p]of pending){p.reply('deny');clearTimeout(p.timer);emit('permission_resolved',{id,decision:'cancelled'});}pending.clear();}
function dispatch(){if(!cli||!ready||busy||!queue.length)return;while(queue.length&&(!checkinStillValid(process.env.HERDR_APP_HOME??'',journal,queue[0])||memoryTurn(queue[0].request_id)&&journal.events.some(e=>e.type==='user'&&e.time>(journal.events.findLast(v=>v.type==='observation'&&v.id===queue[0].request_id)?.time??0)))){const skipped=queue.shift();emit('observation_complete',{id:skipped.request_id,skipped:'newer_human_or_disabled'});}if(!queue.length)return;current=queue.shift();turnRequestId=current.request_id;if(manualReviewId(turnRequestId))emit('review_started',{id:turnRequestId});checkinDecision=null;checkinOutput=[];goalReviewTools.clear();busy=true;progress('submitting');state('working');if(manager){const dir=path.join(process.env.HERDR_APP_HOME,'data/super-agent');fs.mkdirSync(dir,{recursive:true});fs.writeFileSync(path.join(dir,'active-observation.json'),JSON.stringify({id:turnRequestId,socket:folder+'/run.sock',cursor:journal.events.length}),{mode:0o600});}if(manager)current.memory_snapshot=memorySnapshot(process.env.HERDR_APP_HOME);cli.stdin.write(JSON.stringify({...current,action:'send',image_paths:imagePaths(folder,current.images)})+'\n');}
function commitFollowup(args,allowGap=false){
 if(isGoalReview(turnRequestId)&&!allowGap&&(!goalReviewTools.has('list_goals')||!['inspect_sessions','read_session'].some(t=>goalReviewTools.has(t))))throw Error('Before publishing a goal review, read list_goals and attempt fresh inspect_sessions/read_session evidence. Report any observation gaps explicitly.');
 let snapshot={sessions:[]};try{snapshot=JSON.parse(fs.readFileSync(path.join(process.env.HERDR_APP_HOME,'data/super-agent/sessions.json'),'utf8'));}catch{}
 const plan=followupPlan(snapshot,journal.events,turnRequestId,args);
 const recorded=emit('followup_plan',plan);checkinDecision=recorded;if(checkinId(turnRequestId)&&plan.publish)emit('state',{status:'working'});
 const buffered=checkinOutput;checkinOutput=[];if(plan.publish)for(const [type,data]of buffered)emit(type,data);
 return recorded;
}
function isGoalReview(id){return journal.events.some(e=>e.type==='observation'&&e.id===id&&e.events?.some(v=>v.kind==='goal_review'));}
function completeObservation(){if(memoryTurn(turnRequestId)){emit('observation_complete',{id:turnRequestId});turnRequestId=null;checkinOutput=[];return;}if(manualReviewId(turnRequestId))emit('review_complete',{id:turnRequestId,outcome:'completed'});if(turnRequestId?.startsWith('observe_')){
 if(!journal.events.some(e=>e.type==='followup_plan'&&e.observation_id===turnRequestId)){const required=isGoalReview(turnRequestId);if(required)checkinOutput.push(['delta',{id:'goal-review-gap-'+turnRequestId,index:0,kind:'text_delta',text:'本轮目标检查未提交完整核验结论；以上内容可能不完整，不能视为目标已完成。未完成目标会继续定期复查。'}]);commitFollowup({after_minutes:null,publish:required,reason:required?'Goal review ended without an explicit checked report; publish the observation gap and retain the next mandatory review.':'No explicit follow-up decision was produced; wait for the human or new evidence instead of repeating automatically.'},true);}
 if(isGoalReview(turnRequestId))emit('goal_review_complete',{id:turnRequestId});
 emit('observation_complete',{id:turnRequestId});if(checkinId(turnRequestId)){emit('checkin_complete',{id:turnRequestId,published:checkinDecision?.publish===true});if(checkinDecision?.publish&&checkinStillValid(process.env.HERDR_APP_HOME,journal,{request_id:turnRequestId}))notifyCheckin(process.env.HERDR_APP_HOME,turnRequestId);}}
 turnRequestId=null;checkinOutput=[];
}

function start(){
 if(cli||shuttingDown)return;
 if(manager&&runtime.provider==='codex'){
  cli=codexRunner(folder,cwd,{
   models:models=>{codexModels=models;emit('models',{provider:'codex',models});},
   ready:model=>{Object.assign(runtime,{model});ready=true;emit('settings',{...runtime,applied:true});state('idle');dispatch();},
   notice:message=>emit('notice',{message}),
   confirmed:id=>{emit('prompt_confirmed',{id});if(current?.request_id===id)current=null;},
   delta:(id,index,text)=>{progress('responding');emit('delta',{id,index,text,kind:'text_delta'});},
   complete:id=>emit('message_complete',{id}),
   tool:(id,name,input)=>{markReviewTool(name);progress('tool',name);emit('tool',{id,name,input});},
   toolResult:(id,content,is_error)=>emit('tool_result',{id,content,is_error}),
   done:()=>{completeObservation();busy=false;current=null;state('idle');setTimeout(dispatch,300);},
   failure:(message,fatal=false)=>{ready=!fatal;busy=false;current=null;queue=[];state('error');emit('notice',{message});},
   usage:u=>emit('statusline',{model:runtime.model,updated_at:Date.now(),input_tokens:u.last.inputTokens,output_tokens:u.last.outputTokens,cache_read_tokens:u.last.cachedInputTokens,context_size:u.modelContextWindow,context_pct:u.modelContextWindow?100*u.last.totalTokens/u.modelContextWindow:null}),
  },runtime);
  cli.on('exit',()=>{cli=null;ready=false;busy=false;current=null;if(shuttingDown)return;if(restarting){restarting=false;start();}else {queue=[];state('interrupted');emit('notice',{message:'Codex stopped. Send a new prompt to resume; previous prompts were not automatically retried.'});}});return;
 }
 try{assertInactive(session);}catch{queue=[];state('error');emit('notice',{message:'This Claude conversation is still running in another terminal. No prompt was submitted; finish the handover before retrying.'});return;}
 ready=false;
 const command=`python3 ${quote(fileURLToPath(new URL('./hook.py',import.meta.url)))} ${quote(folder)}`;
 const hooks=Object.fromEntries(['PreCompact','SessionStart','UserPromptSubmit','MessageDisplay','PreToolUse','PermissionRequest','PostToolUse','PostToolUseFailure','Stop','StopFailure','SessionEnd'].map(e=>[e,[{hooks:[{type:'command',command,timeout:['PermissionRequest','PreToolUse'].includes(e)?600:5}]}]]));
 fs.writeFileSync(folder+'/settings.json',JSON.stringify({hooks,statusLine:statusLineConfig(folder)}),{mode:0o600});
 cli=spawn('python3',[fileURLToPath(new URL('./runner.py',import.meta.url)),folder,cwd,runtime.model,session,runtime.effort??''],{stdio:['pipe','pipe','ignore']});
 cli.on('error',()=>{cli=null;state('error');});
 cli.stdin.on('error',()=>{ready=false;state('error');});
 readline.createInterface({input:cli.stdout}).on('line',line=>{try{const e=JSON.parse(line);if(e.type==='capabilities'){steerSupported=e.steer===true;emit('capabilities',{capabilities:capabilities()});}if(e.type==='authentication_failed')emit('notice',{message:'Local Claude subscription token is unavailable. Check the configured SSH auth host or sign in with Claude Code.'});if(e.type==='rejected')emit('notice',{message:'Prompt contains unsupported terminal control characters.'});if(e.type==='command_failed'){if(current){emit('prompt_confirmed',{id:current.request_id});current=null;}busy=false;queue=[];state('idle');emit('notice',{message:e.reason==='slash_unavailable'?'Claude did not recognize this project command. Refresh the command list or check its installation.':'Claude could not compact this conversation. Its existing history is kept.'});}}catch{}});
 cli.on('exit',()=>{cli=null;ready=false;busy=false;cancelPermissions();if(shuttingDown)return;if(restarting){restarting=false;start();return;}queue=[];state('interrupted');emit('notice',{message:'Interactive Claude stopped. Connect and send a new prompt to resume; earlier prompts are not automatically retried.'});});
}
function quote(s){return "'"+s.replaceAll("'","'\\''")+"'";}
function hook(c,socket){
 // cwd follows the interactive CLI's cd commands; the session ID and private
 // socket identify its owner, while the launch directory remains unchanged.
 const e=c.event;if(!e||e.session_id!==session||typeof e.cwd!=='string'||!path.isAbsolute(e.cwd))throw Error('wrong_session');
 const reply=output=>{if(!socket.destroyed)socket.end(JSON.stringify({type:'hook_ack',...(output?{output}:{})})+'\n');};
 const name=e.hook_event_name;
 if(name==='PreToolUse')markReviewTool(e.tool_name);
 if(!manager&&['default','plan','acceptEdits','bypassPermissions','auto'].includes(e.permission_mode))permissionMode=e.permission_mode;
 // Late hooks from the exiting backend cannot clear the replacement's queue.
 if(restarting||shuttingDown){
  if(name==='PermissionRequest')reply({hookSpecificOutput:{hookEventName:name,decision:{behavior:'deny',message:'Execution service is stopping.'}}});
  else if(name==='PreToolUse')reply({hookSpecificOutput:{hookEventName:name,permissionDecision:'deny',permissionDecisionReason:'Execution service is stopping.'}});
  else reply();return;
 }
 if(manager&&readOnlyTurn(turnRequestId)&&name==='PreToolUse'&&!readOnlyTool(e.tool_name,turnRequestId)){reply({hookSpecificOutput:{hookEventName:name,permissionDecision:'deny',permissionDecisionReason:'Session inspections are read-only; wait for an explicit human instruction.'}});return;}
 // Record raw structured events before acknowledging, for crash recovery and audit.
 const recorded={...e};if(name==='StopFailure'){delete recorded.error_details;delete recorded.last_assistant_message;}
 emit('hook',{event:recorded});
 if(name==='MessageDisplay'){progress('responding');display.accept(e);reply();return;}
 if(name==='PreCompact'){cli?.stdin.write(JSON.stringify({action:'prompt_confirmed'})+'\n');state('working');reply();return;}
 if(name==='SessionStart'){claudeMemoryVersion=null;if(current&&/^\/compact(?:\s|$)/.test(current.text??'')){emit('prompt_confirmed',{id:current.request_id});current=null;busy=false;}reply();setTimeout(()=>{ready=true;emit('settings',{...runtime,applied:true});state('idle');dispatch();},400);return;}
 if(name==='UserPromptSubmit'){const memory=current?.memory_snapshot??(manager?memorySnapshot(process.env.HERDR_APP_HOME):null);const steered=steering.confirm(e.prompt);if(steered)emit('prompt_confirmed',{id:steered.request_id,mode:'steer'});progress('thinking');cli?.stdin.write(JSON.stringify({action:'prompt_confirmed'})+'\n');fs.writeFileSync(folder+'/has-session','1',{mode:0o600});if(current){emit('prompt_confirmed',{id:current.request_id});current=null;}if(manager){const ver=memory?.version??'unavailable';if(claudeMemoryVersion!==ver){claudeMemoryVersion=ver;reply({hookSpecificOutput:{hookEventName:name,additionalContext:memory?.summary??'Memory unavailable. Verify old preferences with list_memories/read_memory before using them.'}});return;}}reply();return;}
 if(name==='PermissionRequest'||name==='PreToolUse'&&e.tool_name==='AskUserQuestion'){
   progress(name==='PermissionRequest'?'permission':'question',e.tool_name);
   const id=crypto.randomUUID();
   const p={reply:(decision,answers)=>{
     let output;
     if(name==='PermissionRequest')output={hookSpecificOutput:{hookEventName:name,decision:{behavior:decision,...(decision==='deny'?{message:'Declined in Herdr'}:{})}}};
     // AskUserQuestion is a terminal-only tool in interactive mode. A denied
     // call with the user's answer supplies context without pretending the tool ran.
     else output={hookSpecificOutput:{hookEventName:name,permissionDecision:'deny',permissionDecisionReason:decision==='allow'?`User answered in Herdr: ${JSON.stringify(answers??{})}`:'User declined to answer in Herdr'}};
     reply(output);
   }};
   p.timer=setTimeout(()=>{if(pending.delete(id)){p.reply('deny');emit('permission_resolved',{id,decision:'expired'});}},590000);
   pending.set(id,p);emit('permission',{id,name:e.tool_name,input:e.tool_input});
   socket.on('close',()=>{if(pending.delete(id)){clearTimeout(p.timer);emit('permission_resolved',{id,decision:'cancelled'});}});return;
 }
 if(name==='PreToolUse'){progress('tool',e.tool_name);emit('tool',{id:e.tool_use_id,name:e.tool_name,input:e.tool_input});}
 if(name==='PostToolUse'||name==='PostToolUseFailure'){progress('thinking');emit('tool_result',{id:e.tool_use_id,content:name==='PostToolUse'&&e.tool_name==='Read'&&/\.(png|jpe?g|webp|gif)$/i.test(e.tool_input?.file_path??'')?'Image loaded by Claude.':e.tool_response??e.error,is_error:name==='PostToolUseFailure'});}
 if(name==='StopFailure'){const uncertain=steering.cancel();if(uncertain)emit('dispatch_receipt',{id:uncertain.request_id,mode:'steer',stage:'unknown',reason:'Claude ended before confirming the steering input.'});busy=false;current=null;queue=[];cancelPermissions();state('error');const code=['rate_limit','overloaded','authentication_failed','oauth_org_not_allowed','account_on_hold','billing_error','invalid_request','model_not_found','server_error','max_output_tokens','cloud_credential_error'].includes(e.error)?e.error:'unknown';emit('notice',{message:`Claude ended the turn: ${code}. Earlier prompts were not automatically retried.`});}
 if(name==='Stop'&&steering.pending){state('working');reply();return;}
 if(name==='Stop'){completeObservation();busy=false;current=null;state('idle');setTimeout(dispatch,300);}
 if(name==='SessionEnd'){ready=false;cancelPermissions();}
 reply();
}
if(manager){
 const completed=new Set(journal.events.filter(e=>e.type==='observation_complete').map(e=>e.id));
 for(const e of journal.events.filter(e=>e.type==='observation'&&!completed.has(e.id)))queue.push({action:'send',request_id:e.id,text:'[Session events]\n'+JSON.stringify(e.events),images:[]});
}
const server=net.createServer(socket=>{
 let subscribed=false,buffer='';socket.setEncoding('utf8');
 const handle=async line=>{let c;try{
   c=JSON.parse(line);
   if(c.action==='hook'){hook(c,socket);return;}
   if(c.action==='subscribe'){
     if(!Number.isSafeInteger(c.after)||c.after<0||c.after>journal.events.length)throw Error('cursor');
     for(const e of journal.after(c.after))socket.write(JSON.stringify(e)+'\n');peers.add(socket);subscribed=true;
     socket.write(JSON.stringify({type:'connected',bridge_protocol:2,status:memoryTurn(turnRequestId)||checkinId(turnRequestId)&&!checkinDecision?.publish?'idle':status,head:journal.events.length,pending:[...pending.keys(),...(codexWorkers?.ids()??[])],...activity(),capabilities:capabilities()})+'\n');return;
   }
   if(!subscribed||typeof c.request_id!=='string'||! /^[a-zA-Z0-9_-]{8,100}$/.test(c.request_id))throw Error('invalid_command');
   if(c.action==='plan_followup'&&manager){
     if(!busy||!turnRequestId?.startsWith('observe_')||c.observation_id!==turnRequestId)throw Error('inactive_observation');
     const existing=journal.events.findLast(e=>e.type==='followup_plan'&&e.observation_id===turnRequestId);
     let plan;
     if(existing){if(existing.after_minutes!==c.after_minutes||existing.publish!==c.publish||existing.reason!==c.reason)throw Error('already_planned');plan=existing;}
     else plan=commitFollowup(c);
     socket.write(JSON.stringify({type:'ack',request_id:c.request_id,plan})+'\n');return;
   }
   if(c.action==='permission'&&manager&&typeof c.id==='string'&&c.id.startsWith('codexq_')){
     if(!['allow','deny'].includes(c.decision))throw Error('decision');
     const q=codexWorkers?.get(c.id);
     if(!q){emit('notice',{message:'Codex question is synchronizing or already resolved. No decision was sent.'});socket.write(JSON.stringify({type:'ack',request_id:c.request_id})+'\n');return;}
     const result=await workerAnswer({address:q.address,question_id:c.id,answers:c.answers??{},decision:c.decision,ui:true,request_id:c.request_id});
     emit('worker_decision',{id:c.request_id,address:q.address,question_id:c.id,decision:c.decision,answers:c.answers??{}});
     emit('notice',{message:result.request_resolved?'Decision delivered to '+q.address+'; request resolved.':'Decision submitted to '+q.address+'; confirmation is pending.'});
     codexWorkers.sync();socket.write(JSON.stringify({type:'ack',request_id:c.request_id})+'\n');return;
   }
   if(c.action==='reminder_ack'&&manager){
     if(!Array.isArray(c.ids)||c.ids.length>128||!c.ids.every(id=>typeof id==='string'&&id.length<180))throw Error('reminder_ids');
     journal.accept(c);await scheduleCall('--ack',process.env.HERDR_APP_HOME,{ids:c.ids});await reminderClock?.tick();
     socket.write(JSON.stringify({type:'ack',request_id:c.request_id})+'\n');return;
   }
   if(c.action==='provider'&&manager){
     if(!['codex','claude'].includes(c.provider))throw Error('provider');
     if(busy||pending.size||queue.length||cli&&!ready){socket.write(JSON.stringify({type:'notice',message:'Finish or stop the current turn before switching manager.'})+'\n');socket.write(JSON.stringify({type:'ack',request_id:c.request_id})+'\n');return;}
     if(journal.accept(c)){
      const previous=journal.events.findLast(e=>e.type==='settings'&&e.provider===c.provider);runtime={model:previous?.model??(c.provider==='codex'?'default':'opus'),effort:previous?.effort??'',provider:c.provider};fs.writeFileSync(folder+'/manager-provider',c.provider,{mode:0o600});emit('settings',{...runtime,applied:true});
      emit('notice',{message:'Manager changed. Task ledger and worker sessions are retained; each backend keeps its own conversation context.'});
      if(cli){restarting=true;ready=false;state('starting');cli.kill('SIGTERM');}
     }
     socket.write(JSON.stringify({type:'ack',request_id:c.request_id})+'\n');return;
   }
   if(!['send','interrupt','permission','workspace','configure'].includes(c.action))throw Error('unknown_action');
   if(c.action==='send'&&(typeof c.text!=='string'||(!c.text.trim()&&!c.images?.length)||Buffer.byteLength(c.text)>256*1024||/[\x00-\x08\x0b-\x1f\x7f]/.test(c.text)))throw Error('invalid_message');
   if(c.action==='send'&&manualReviewId(c.request_id)&&!manager)throw Error('manager_only');
   if(c.action==='send'){imagePaths(folder,c.images);if(c.mode&&!['queue','steer'].includes(c.mode))throw Error('invalid_mode');}
   if(c.action==='permission'&&!['allow','deny'].includes(c.decision))throw Error('invalid_decision');
   if(c.action==='workspace'){
     try{socket.write(JSON.stringify({type:'workspace',request_id:c.request_id,...await workspace(cwd,undefined,typeof c.query==='string'?c.query.slice(0,4096):'')})+'\n');}catch{socket.write(JSON.stringify({type:'workspace',request_id:c.request_id,files:[],commands:[],error:'Could not list remote project files.'})+'\n');}
     socket.write(JSON.stringify({type:'ack',request_id:c.request_id})+'\n');return;
   }
   if(c.action==='configure'){
     if(!(manager&&runtime.provider==='codex'?codexModels.some(m=>m.model===c.model&&(!c.effort||m.supportedReasoningEfforts?.some(e=>e.reasoningEffort===c.effort))):validSettings(c))){socket.write(JSON.stringify({type:'notice',message:'Unsupported model or reasoning effort.'})+'\n');socket.write(JSON.stringify({type:'ack',request_id:c.request_id})+'\n');return;}
     if(busy||pending.size||queue.length||cli&&!ready){socket.write(JSON.stringify({type:'notice',message:'Wait for the current turn to finish before changing model or effort.'})+'\n');socket.write(JSON.stringify({type:'ack',request_id:c.request_id})+'\n');return;}
   }
   if(c.action==='send'&&c.mode==='steer'&&!journal.requests.has(c.request_id)){
     const reason=steering.refusal(c,{manager,supported:steerSupported,busy,ready:ready&&!current,approval:pending.size>0,turn:turnRequestId});
     if(reason){journal.accept(c);emit('user',{id:c.request_id,text:c.text,images:c.images??[],mode:'steer'});emit('dispatch_receipt',{id:c.request_id,mode:'steer',stage:'refused',reason});socket.write(JSON.stringify({type:'ack',request_id:c.request_id,rejected:true})+'\n');return;}
   }
   if(journal.accept(c)){
     const accepted=journal.events.at(-1);for(const p of peers)p.write(JSON.stringify(accepted)+'\n');
     if(c.action==='configure'){const same=runtime.model===c.model&&(runtime.effort??'')===c.effort;Object.assign(runtime,{model:c.model,effort:c.effort});emit('settings',{...runtime,applied:runtime.provider==='codex'||!cli||same});if(cli&&!same&&runtime.provider!=='codex'){restarting=true;ready=false;state('starting');cli.kill('SIGTERM');}}
     if(c.action==='send'){
       if(manualReviewId(c.request_id)&&(manualReviewId(turnRequestId)||queue.some(q=>manualReviewId(q.request_id))))emit('review_complete',{id:c.request_id,outcome:'coalesced'});
       else{emit(manualReviewId(c.request_id)?'review_requested':'user',{id:c.request_id,text:manualReviewId(c.request_id)?'检查全部会话并同步最新进展。':c.text,images:c.images??[],mode:c.mode??'queue'});
        if(c.mode==='steer'){
          const paths=imagePaths(folder,c.images);const prompt=c.text+(paths.length?'\n\nAttached images (use Read to view these image files):\n'+paths.join('\n'):'');
          steering.begin(c,prompt);emit('dispatch_receipt',{id:c.request_id,mode:'steer',stage:'submitted'});
          cli.stdin.write(JSON.stringify({...c,action:'send',image_paths:paths})+'\n');
          const timer=setTimeout(()=>{if(steering.pending?.command.request_id===c.request_id)emit('dispatch_receipt',{id:c.request_id,mode:'steer',stage:'unknown',reason:'Claude has not confirmed this steering message. Inspect this session; do not resend automatically.'});},15000);timer.unref();
        }else{queue.push(c);start();dispatch();}}
     }
     if(c.action==='permission'){const p=pending.get(c.id);if(p){pending.delete(c.id);clearTimeout(p.timer);progress('tool',lastTool);p.reply(c.decision,c.answers);emit('permission_resolved',{id:c.id,decision:c.decision});}else emit('notice',{message:'This approval request has ended.'});}
     if(c.action==='interrupt'){const cancelled=steering.cancel();if(cancelled)emit('dispatch_receipt',{id:cancelled.request_id,mode:'steer',stage:'cancelled'});queue=[];cancelPermissions();cli?.stdin.write(JSON.stringify({action:'interrupt'})+'\n');busy=false;current=null;state('interrupting');emit('notice',{message:'Stop requested. Waiting for Claude to finish interrupting.'});}
   }
   socket.write(JSON.stringify({type:'ack',request_id:c.request_id})+'\n');
 }catch{socket.write(JSON.stringify({type:'command_error',request_id:c?.request_id??'',message:'Command could not be accepted. Your outgoing command is retained; inspect the connection before retrying.'})+'\n');}};
 socket.on('data',data=>{buffer+=data;if(Buffer.byteLength(buffer)>1024*1024){socket.destroy();return;}let n;while((n=buffer.indexOf('\n'))>=0){const line=buffer.slice(0,n);buffer=buffer.slice(n+1);handle(line);}});
 socket.on('error',()=>{});socket.on('close',()=>peers.delete(socket));
});
const sock=folder+'/run.sock';try{fs.unlinkSync(sock);}catch{}
server.listen(sock,()=>{if(manager){fs.writeFileSync(folder+'/manager-provider',runtime.provider,{mode:0o600});emit('settings',{...runtime,applied:true});}fs.chmodSync(sock,0o600);fs.writeFileSync(folder+'/daemon.pid',String(process.pid),{mode:0o600});fs.writeFileSync(folder+'/bridge-protocol.json',JSON.stringify({version:2,pid:process.pid}),{mode:0o600});if(resume||manager&&runtime.provider==='codex'||!manager&&existing&&fs.existsSync(folder+'/has-session'))start();else state('idle');if(manager&&queue.length)start();});
function quit(){memoryClock?.stop();checkinClock?.stop();codexWorkers?.stop();reminderClock?.stop();shuttingDown=true;restarting=false;cancelPermissions();server.close();if(!cli){process.exit(0);return;}cli.once('exit',()=>process.exit(0));cli.kill('SIGTERM');setTimeout(()=>process.exit(0),6500);}process.on('SIGTERM',quit);process.on('SIGINT',quit);

if(process.env.HERDR_SUPER_AGENT==='1')supervise(folder,events=>{
 const c={action:'send',request_id:'observe_'+crypto.createHash('sha256').update(JSON.stringify(events)).digest('hex').slice(0,40),text:'[Session events]\n'+JSON.stringify(events),images:[]};
 if(journal.accept(c)){emit('observation',{id:c.request_id,events});queue.push(c);start();dispatch();}
},()=>((runtime.provider==='codex'&&fs.existsSync(folder+'/codex-thread.json'))||fs.existsSync(folder+'/has-session'))&&!shuttingDown&&!busy&&!queue.length&&pending.size===0);

if(manager&&process.env.HERDR_APP_HOME)reminderClock=startScheduler(process.env.HERDR_APP_HOME,journal,emit);

if(manager&&process.env.HERDR_APP_HOME)codexWorkers=startCodexWorkers(process.env.HERDR_APP_HOME,emit,journal);

if(manager&&process.env.HERDR_APP_HOME)checkinClock=startCheckins(process.env.HERDR_APP_HOME,folder,journal,(event,id)=>{
 const c={action:'send',request_id:id,text:'[Periodic check-in]\n'+JSON.stringify(event),images:[]};
 if(journal.accept(c)){emit('observation',{id,events:[event]});queue.push(c);start();dispatch();}
},()=>ready&&!shuttingDown&&!busy&&!queue.length&&pending.size===0);

if(manager&&process.env.HERDR_APP_HOME)memoryClock=startMemoryMaintenance(process.env.HERDR_APP_HOME,folder,(event,id)=>{
 const c={action:'send',request_id:id,text:'[Memory maintenance]\nReview pending HUMAN feedback with review_memories. Extract only durable cooperation lessons with exact human source quotes, search first and update stable topics; preserve manual/newer notes. Then reread review_memories and consolidate_memories into a deduplicated stable profile, preferences and topic index, covering every current note. Acknowledge the exact feedback batch even when it contains nothing worth remembering. Never dispatch workers, answer questions, change goals, approve, or report routine maintenance to the human. '+JSON.stringify(event),images:[]};
 if(journal.accept(c)){emit('observation',{id,events:[event]});queue.push(c);start();dispatch();}
},()=>ready&&!shuttingDown&&!busy&&!queue.length&&pending.size===0);
