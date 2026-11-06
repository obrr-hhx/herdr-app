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
