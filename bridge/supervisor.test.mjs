import {test} from 'node:test';import assert from 'node:assert/strict';import {changes} from './supervisor.mjs';
test('manual discovery, manager exclusion and reconnect do not lose source positions',()=>{
 const a={address:'chat:p',status:'idle',completion:8,title:'Worker'};
 const r=changes({},[a,{...a,address:'manager',super_agent:true}]);assert.equal(r.events.length,1);assert.equal(r.next.manager,undefined);
 assert.equal(changes(r.next,[]).next[a.address].completion,8);
 assert.equal(changes(r.next,[a]).events.length,0);
});
test('heartbeat and viewed Done do not wake manager; completed result and question do',()=>{
 const a={address:'chat:p',status:'working',completion:2};let r=changes({},[a]);
 assert.equal(changes(r.next,[{...a,seq:30}]).events.length,0);
 r=changes(r.next,[{...a,status:'done',completion:3}]);assert.equal(r.events[0].kind,'result');
 assert.equal(changes(r.next,[{...a,status:'idle',completion:3}]).events.length,0);
 r=changes(r.next,[{...a,status:'blocked',completion:3}]);assert.equal(r.events[0].kind,'needs_user');
 assert.equal(changes(r.next,[{...a,status:'blocked',completion:3}]).events.length,0);
});

test('transient reconnect is quiet; a persistent outage is reported once and can recover',()=>{const a={address:'chat:p',status:'idle',detail:'SSH disconnected — reconnecting'};let r=changes({},[a],1000);assert.equal(r.events.length,1);r=changes(r.next,[a],20000);assert.equal(r.events.length,0);r=changes(r.next,[a],32000);assert.equal(r.events[0].kind,'connection_lost');r=changes(r.next,[a],60000);assert.equal(r.events.length,0);r=changes(r.next,[{...a,detail:'idle'}],61000);assert.equal(r.events[0].kind,'connection_restored');});

test('a second question while blocked is a distinct event',()=>{const a={address:'chat:p',status:'blocked',attention:'question-1',seq:1};let r=changes({},[a]);r=changes(r.next,[{...a,attention:'question-2',seq:2}]);assert.equal(r.events[0].kind,'needs_user');assert.equal(r.events[0].attention,'question-2');assert.equal(changes(r.next,[{...a,attention:'question-2',seq:2}]).events.length,0);});

test('a fast terminal turn is observed even when the UI never saw working',()=>{let a={address:'t1',status:'done',execution_state:'done',completion:1};let r=changes({},[a]);r=changes(r.next,[{...a,completion:2}]);assert.equal(r.events[0].kind,'result');r=changes(r.next,[{...a,execution_state:'working',completion:0}]);assert.equal(r.events.length,0);r=changes(r.next,[{...a,completion:3}]);assert.equal(r.events[0].kind,'result');assert.equal(changes(r.next,[{...a,completion:3}]).events.length,0);});

test('verified goal prerequisites wake continuation but ordinary progress never loops',async()=>{const {goalVersion}=await import('./supervisor.mjs');const g={id:'goal',enabled:true,phase:'active',mandate:{scope:'STG'},plan:[{id:'next'}],milestones:{ready:{achieved:false,observed_at:1}}};const before=goalVersion([g]);assert.equal(goalVersion([{...g,summary:'fresh wording',revision:20,observed_at:200,milestones:{ready:{achieved:false,observed_at:200}}}]),before);assert.notEqual(goalVersion([{...g,milestones:{ready:{achieved:true}}}]),before);assert.notEqual(goalVersion([{...g,needs_user:true}]),before);assert.notEqual(goalVersion([{...g,enabled:false}]),before);});

test('quiet goal workers ignore heartbeat/goal-summary refreshes and reset on real work, completion or blockers',async()=>{
 const {quietGoalWorkers}=await import('./supervisor.mjs');const now=Date.now(),row={address:'chat:p',conversation:'c',host:'qa',status:'idle',completion:1};const binding={conversation:'c',host:'qa',read_identity:null};const goals=[{id:'g',enabled:true,phase:'active',plan:[{id:'s',address:'chat:p',binding}]}];
 let a=quietGoalWorkers({},goals,[row],now);assert.equal(a.events.length,0);
 let b=quietGoalWorkers(a.next,goals,[{...row,seq:999}],now+16*60000);assert.equal(b.events[0].kind,'goal_session_quiet');
 assert.equal(quietGoalWorkers(b.next,goals,[row],now+17*60000).events.length,0);
 assert.equal(quietGoalWorkers(b.next,goals,[{...row,status:'working'}],now+18*60000).next['chat:p'],undefined);
 assert.equal(quietGoalWorkers(b.next,goals,[{...row,completion:2}],now+18*60000).events.length,0);
 assert.equal(quietGoalWorkers(b.next,[{...goals[0],needs_user:true}],[row],now+60*60000).events.length,0);
});
