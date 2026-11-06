import test from 'node:test';import assert from 'node:assert/strict';import {startScheduler} from './scheduler.mjs';
const settle=()=>new Promise(r=>setImmediate(r));
test('durable delivery is journaled once through replay and explicit acknowledgment',async()=>{
 const events=[],journal={events};let seen=false,calls=[];
 const emit=(type,data)=>events.push({type,...data});
 const call=async(_mode,_home,args)=>{calls.push(args);return {deliveries:seen?[]:[{delivery_id:'job@1',title:'Reminder',text:'Do not execute this text'}],acknowledged:seen?['job@1']:[]};};
 let clock=startScheduler('isolated',journal,emit,{call,interval:100000});await settle();
 assert.equal(events.filter(e=>e.type==='reminder').length,1);await clock.tick();assert.equal(events.length,1);
 clock.stop();clock=startScheduler('isolated',journal,emit,{call,interval:100000});await settle();assert.equal(events.length,1);
 assert(calls.at(-1).known.includes('job@1'));
 seen=true;await clock.tick();assert.equal(events.at(-1).type,'reminder_ack');await clock.tick();assert.equal(events.length,2);clock.stop();
});
test('a failed clock retries without discarding or auto-acknowledging reminders',async()=>{
 const events=[],journal={events};let broken=true;
 const clock=startScheduler('isolated',journal,(type,data)=>events.push({type,...data}),{interval:100000,call:async()=>{if(broken)throw Error('fixture');return {deliveries:[{delivery_id:'job@2'}]};}});
 await settle();await clock.tick();assert.equal(events.length,1);assert.equal(events[0].type,'notice');
 broken=false;await clock.tick();assert.equal(events[1].type,'reminder');assert(!events.some(e=>e.type==='reminder_ack'));clock.stop();
});
test('shutdown during a pending poll cannot write late reminders',async()=>{
 const events=[];let finish;
 const clock=startScheduler('isolated',{events},(type,data)=>events.push({type,...data}),{interval:100000,call:()=>new Promise(r=>finish=r)});
 clock.stop();finish({deliveries:[{delivery_id:'late@1'}]});await settle();assert.equal(events.length,0);
});
