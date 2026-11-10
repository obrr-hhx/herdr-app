import test from 'node:test';import assert from 'node:assert/strict';import fs from 'node:fs';import path from 'node:path';import {spawn,execFileSync}from'node:child_process';import {fileURLToPath}from'node:url';import {setTimeout as delay}from'node:timers/promises';
const base=path.dirname(fileURLToPath(import.meta.url));
test('scheduled goal review inspects evidence, advances only its stored plan and publishes across restart without duplicate dispatch',async()=>{
 const root=fs.mkdtempSync('/tmp/hgoal-'),home=root+'/home',folder=home+'/data/local-chat/73757065726167656e74000000000001',cache=home+'/data/forge-chat/73757065726167656e74000000000001';fs.mkdirSync(folder,{recursive:true});fs.mkdirSync(cache,{recursive:true});fs.mkdirSync(home+'/data/super-agent',{recursive:true});
 const human={type:'user',id:'human_goal_grant',text:'完成这个目标：前置验证通过后接着做STG验证，不上生产、不替我批准工具。',time:Date.now(),seq:1};fs.writeFileSync(cache+'/events.jsonl',JSON.stringify(human)+'\n');
 const snapshot={at:Date.now(),core:'unused',sessions:[{address:'chat:qa-worker',conversation:'qa-worker',host:'qa',status:'idle'}]};fs.writeFileSync(home+'/data/super-agent/sessions.json',JSON.stringify(snapshot));
 const env={...process.env,HERDR_APP_HOME:home,PYTHONPATH:base};
 execFileSync('python3',['-c',`import goal_store as g,json,os;h=os.environ['HERDR_APP_HOME'];s=json.load(open(h+'/data/super-agent/sessions.json'))['sessions'];g.save(h,{'id':'qa-goal','expected_revision':0,'title':'STG验收','outcome':'STG真实验收','enabled':True,'mandate':{'source_ids':['human_goal_grant'],'actions':['test'],'scope':'仅STG验证','exclusions':'不生产发布、不批准工具'},'plan':[{'id':'continue','address':'chat:qa-worker','action':'test','instruction':'接续原STG验证','postcondition':'返回STG证据','depends_on':['ready']}]},s,admin=True)`],{env});
 const app=root+'/app';fs.writeFileSync(app,`#!/usr/bin/env python3
import json,os,sys,pathlib
h=pathlib.Path(os.environ['HERDR_APP_HOME']);args=json.loads(sys.argv[3]);cmd=sys.argv[2]
if cmd=='supervisor.list':print((h/'data/super-agent/sessions.json').read_text())
elif cmd=='supervisor.send':
 assert set(args)<=set(['address','text','request_id']), 'tool-only fields must not leak into the native app contract'
 p=h/'data/forge-chat/qa-worker';p.mkdir(parents=True,exist_ok=True)
 with (p/'dispatches.jsonl').open('a') as f:f.write(json.dumps(args)+'\\n')
 (p/'events.jsonl').write_text(json.dumps({'type':'prompt_confirmed','id':args['request_id'],'seq':1})+'\\n')
 print(json.dumps({'queued':True,'confirmed':False}))
else:print(json.dumps({'error':'Unsupported fixture control'}));sys.exit(1)
`,{mode:0o700});
 const fake=root+'/codex';fs.writeFileSync(fake,`#!/usr/bin/env node
let serial=800,stage=0,goal;const emit=v=>process.stdout.write(JSON.stringify(v)+'\\n');const call=(tool,args)=>emit({id:++serial,method:'item/tool/call',params:{threadId:'goal-thread',callId:'call-'+serial,tool,arguments:args}});
require('readline').createInterface({input:process.stdin}).on('line',l=>{const m=JSON.parse(l),reply=result=>emit({id:m.id,result});
if(m.method==='initialize')reply({});if(m.method==='config/read')reply({config:{}});if(m.method==='model/list')reply({data:[{model:'fixture',hidden:false,supportedReasoningEfforts:[]}],nextCursor:null});if(m.method==='thread/start'||m.method==='thread/resume')reply({thread:{id:'goal-thread'},model:'fixture'});
if(m.method==='turn/start'){reply({turn:{id:'goal-turn'}});stage=0;call('list_goals',{});}
if(m.id>=801&&m.result){let result={};try{result=JSON.parse(m.result.contentItems[0].text);}catch{}
if(stage===0){goal=result.goals[0];stage++;call('record_goal_progress',{id:'qa-goal',expected_revision:goal.revision,phase:'active',summary:'前置条件已从当前来源核实',next_action:'自动接续原STG验证',observed_at:Date.now(),source_ids:['qa-source-result'],milestones:{ready:{achieved:true,summary:'前置验证通过',source_ids:['qa-source-result']}}});}
else if(stage===1){stage++;call('advance_goal',{goal_id:'qa-goal',step_id:'continue'});}
else if(stage===2){stage++;call('advance_goal',{goal_id:'qa-goal',step_id:'continue'});}
else if(stage===3){stage++;call('send_message',{address:'chat:qa-worker',request_id:'unauthorized-raw-send',text:'must not dispatch'});}
else if(stage===4){stage++;call('record_goal',{id:'qa-goal',expected_revision:result.revision??goal.revision+1,title:'Escalated',outcome:'STG真实验收',enabled:true,mandate:{source_ids:['human_goal_grant'],actions:['test','deploy_prd'],scope:'Production too',exclusions:'none'}});}
else if(stage===5){stage++;call('list_goals',{});}
else if(stage===6){stage++;call('inspect_sessions',{});}
else if(stage===7){stage++;call('plan_followup',{after_minutes:15,publish:true,reason:'Reviewed goal evidence and continued its authorized step'});}
else {emit({method:'item/agentMessage/delta',params:{threadId:'goal-thread',itemId:'report',delta:'目标已自动接续，授权范围未扩大。'}});emit({method:'turn/completed',params:{turn:{id:'goal-turn',status:'completed'}}});}
}});`,{mode:0o700});
 const daemonEnv={...env,HERDR_APP_BIN:app,HERDR_CODEX_BIN:fake,HERDR_SUPER_AGENT:'1',HERDR_CHAT_LOCAL:'1',HERDR_CODEX_CONTROL_SOCKET:root+'/no.sock',HERDR_CODEX_WORKER_SOCKET:root+'/workers.sock'};
 let child;try{
  for(let run=0;run<2;run++){
   let records=fs.existsSync(folder+'/events.jsonl')?fs.readFileSync(folder+'/events.jsonl','utf8').trim().split('\n').map(JSON.parse):[{type:'settings',provider:'codex',model:'fixture',effort:'',time:Date.now(),seq:1}];records.push({type:'observation',id:'observe_checkin_goal_run_'+run,events:[{kind:'goal_review'}],seq:records.length+1,time:Date.now()});fs.writeFileSync(folder+'/events.jsonl',records.map(JSON.stringify).join('\n')+'\n');
   child=spawn(process.execPath,[base+'/daemon.mjs',folder,root,'opus'],{env:daemonEnv,stdio:'ignore'});
   let all=[];for(let i=0;i<350;i++){await delay(30);all=fs.readFileSync(folder+'/events.jsonl','utf8').trim().split('\n').map(JSON.parse);if(all.some(e=>e.type==='observation_complete'&&e.id==='observe_checkin_goal_run_'+run))break;}
   assert(all.some(e=>e.type==='observation_complete'&&e.id==='observe_checkin_goal_run_'+run),'background goal turn did not complete');
   assert(all.some(e=>e.type==='goal_review_complete'&&e.id==='observe_checkin_goal_run_'+run));assert(all.some(e=>e.type==='followup_plan'&&e.publish===true));assert(all.some(e=>e.type==='delta'&&e.text.includes('目标已自动接续')));assert(all.some(e=>e.type==='tool_result'&&e.is_error&&JSON.stringify(e.content).includes('Background turns')));
   child.kill('SIGTERM');await new Promise(r=>child.once('exit',r));child=null;
  }
  const dispatches=fs.readFileSync(home+'/data/forge-chat/qa-worker/dispatches.jsonl','utf8').trim().split('\n').map(JSON.parse);assert.equal(dispatches.length,1);assert(dispatches[0].text.includes('不生产发布、不批准工具'));
  const goals=JSON.parse(fs.readFileSync(home+'/data/super-agent/goals.json'));assert.deepEqual(goals.goals[0].mandate.actions,['test']);assert.equal(goals.goals[0].human_accepted,false);assert(goals.goals[0].steps_receipts[0].confirmed);
 }finally{if(child){child.kill('SIGTERM');await new Promise(r=>child.once('exit',r));}fs.rmSync(root,{recursive:true,force:true});}
});
