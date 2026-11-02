import net from 'node:net';
import fs from 'node:fs';
import path from 'node:path';
import readline from 'node:readline';
import crypto from 'node:crypto';
import {spawn} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {Journal} from './journal.mjs';
import {Display} from './display.mjs';
const [folder,requestedCwd,model='opus']=process.argv.slice(2);
const cwd=fs.realpathSync(requestedCwd);
if(!folder||!path.isAbsolute(cwd))process.exit(2);
const journal=new Journal(folder),peers=new Set(),pending=new Map();
const session=journal.events.findLast(e=>e.type==='session')?.session_id??crypto.randomUUID();
let cli=null,ready=false,busy=false,status='starting',current=null,queue=[];
const emit=(type,data={})=>{const e=journal.append(type,data),line=JSON.stringify(e)+'\n';for(const p of peers){if(p.writableLength>8*1024*1024)p.destroy();else p.write(line);}return e;};
if(journal.events.length)emit('state',{status:'interrupted',message:'Service restarted. Previously accepted prompts were not automatically submitted again.'});
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
 if(e.type==='permission')oldPending.add(e.id);
 if(e.type==='permission_resolved')oldPending.delete(e.id);
 if(e.type==='hook'&&e.event.hook_event_name==='MessageDisplay')display.accept(e.event);
}
for(const id of oldPending)emit('permission_resolved',{id,decision:'cancelled'});
function state(s){status=s;emit('state',{status:s});}
function cancelPermissions(){for(const [id,p]of pending){p.reply('deny');clearTimeout(p.timer);emit('permission_resolved',{id,decision:'cancelled'});}pending.clear();}
function dispatch(){if(!cli||!ready||busy||!queue.length)return;current=queue.shift();busy=true;state('working');cli.stdin.write(JSON.stringify({...current,action:'send'})+'\n');}
function start(){
 if(cli)return;
 ready=false;
 const command=`python3 ${quote(fileURLToPath(new URL('./hook.py',import.meta.url)))} ${quote(folder)}`;
 const hooks=Object.fromEntries(['SessionStart','UserPromptSubmit','MessageDisplay','PreToolUse','PermissionRequest','PostToolUse','PostToolUseFailure','Stop','StopFailure','SessionEnd'].map(e=>[e,[{hooks:[{type:'command',command,timeout:['PermissionRequest','PreToolUse'].includes(e)?600:5}]}]]));
 fs.writeFileSync(folder+'/settings.json',JSON.stringify({hooks}),{mode:0o600});
 cli=spawn('python3',[fileURLToPath(new URL('./runner.py',import.meta.url)),folder,cwd,model,session],{stdio:['pipe','pipe','ignore']});
 cli.on('error',()=>{cli=null;state('error');});
 cli.stdin.on('error',()=>{ready=false;state('error');});
 readline.createInterface({input:cli.stdout}).on('line',line=>{try{const e=JSON.parse(line);if(e.type==='rejected')emit('notice',{message:'Prompt contains unsupported terminal control characters.'});}catch{}});
 cli.on('exit',()=>{cli=null;ready=false;busy=false;queue=[];cancelPermissions();state('interrupted');emit('notice',{message:'Interactive Claude stopped. Connect and send a new prompt to resume; earlier prompts are not automatically retried.'});});
}
function quote(s){return "'"+s.replaceAll("'","'\\''")+"'";}
function hook(c,socket){
 const e=c.event;if(!e||e.session_id!==session||e.cwd!==cwd)throw Error('wrong_session');
 const reply=output=>{if(!socket.destroyed)socket.end(JSON.stringify({type:'hook_ack',...(output?{output}:{})})+'\n');};
 const name=e.hook_event_name;
 // Record raw structured events before acknowledging, for crash recovery and audit.
 const recorded={...e};if(name==='StopFailure'){delete recorded.error_details;delete recorded.last_assistant_message;}
 emit('hook',{event:recorded});
 if(name==='MessageDisplay'){display.accept(e);reply();return;}
 if(name==='SessionStart'){reply();setTimeout(()=>{ready=true;state('idle');dispatch();},400);return;}
 if(name==='UserPromptSubmit'){fs.writeFileSync(folder+'/has-session','1',{mode:0o600});if(current){emit('prompt_confirmed',{id:current.request_id});current=null;}reply();return;}
 if(name==='PermissionRequest'||name==='PreToolUse'&&e.tool_name==='AskUserQuestion'){
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
 if(name==='PreToolUse')emit('tool',{id:e.tool_use_id,name:e.tool_name,input:e.tool_input});
 if(name==='PostToolUse'||name==='PostToolUseFailure')emit('tool_result',{id:e.tool_use_id,content:e.tool_response??e.error,is_error:name==='PostToolUseFailure'});
 if(name==='StopFailure'){busy=false;current=null;queue=[];cancelPermissions();state('error');const code=['rate_limit','overloaded','authentication_failed','oauth_org_not_allowed','account_on_hold','billing_error','invalid_request','model_not_found','server_error','max_output_tokens','cloud_credential_error'].includes(e.error)?e.error:'unknown';emit('notice',{message:`Claude ended the turn: ${code}. Earlier prompts were not automatically retried.`});}
 if(name==='Stop'){busy=false;current=null;state('idle');setTimeout(dispatch,300);}
 if(name==='SessionEnd'){ready=false;cancelPermissions();}
 reply();
}
const server=net.createServer(socket=>{
 let subscribed=false,buffer='';socket.setEncoding('utf8');
 const handle=async line=>{let c;try{
   c=JSON.parse(line);
   if(c.action==='hook'){hook(c,socket);return;}
   if(c.action==='subscribe'){
     if(!Number.isSafeInteger(c.after)||c.after<0||c.after>journal.events.length)throw Error('cursor');
     for(const e of journal.after(c.after))socket.write(JSON.stringify(e)+'\n');peers.add(socket);subscribed=true;
     socket.write(JSON.stringify({type:'connected',status,head:journal.events.length,pending:[...pending.keys()]})+'\n');return;
   }
   if(!subscribed||typeof c.request_id!=='string'||! /^[a-zA-Z0-9_-]{8,100}$/.test(c.request_id))throw Error('invalid_command');
   if(!['send','interrupt','permission'].includes(c.action))throw Error('unknown_action');
   if(c.action==='send'&&(typeof c.text!=='string'||!c.text.trim()||Buffer.byteLength(c.text)>256*1024||/[\x00-\x08\x0b-\x1f\x7f]/.test(c.text)))throw Error('invalid_message');
   if(c.action==='permission'&&!['allow','deny'].includes(c.decision))throw Error('invalid_decision');
   if(journal.accept(c)){
     const accepted=journal.events.at(-1);for(const p of peers)p.write(JSON.stringify(accepted)+'\n');
     if(c.action==='send'){emit('user',{id:c.request_id,text:c.text});queue.push(c);start();dispatch();}
     if(c.action==='permission'){const p=pending.get(c.id);if(p){pending.delete(c.id);clearTimeout(p.timer);p.reply(c.decision,c.answers);emit('permission_resolved',{id:c.id,decision:c.decision});}else emit('notice',{message:'This approval request has ended.'});}
     if(c.action==='interrupt'){queue=[];cancelPermissions();cli?.stdin.write(JSON.stringify({action:'interrupt'})+'\n');busy=false;current=null;state('interrupting');emit('notice',{message:'Stop requested. Waiting for Claude to finish interrupting.'});}
   }
   socket.write(JSON.stringify({type:'ack',request_id:c.request_id})+'\n');
 }catch{socket.write(JSON.stringify({type:'command_error',request_id:c?.request_id??'',message:'Command could not be accepted. Your outgoing command is retained; inspect the connection before retrying.'})+'\n');}};
 socket.on('data',data=>{buffer+=data;if(Buffer.byteLength(buffer)>1024*1024){socket.destroy();return;}let n;while((n=buffer.indexOf('\n'))>=0){const line=buffer.slice(0,n);buffer=buffer.slice(n+1);handle(line);}});
 socket.on('error',()=>{});socket.on('close',()=>peers.delete(socket));
});
const sock=folder+'/run.sock';try{fs.unlinkSync(sock);}catch{}
server.listen(sock,()=>{fs.chmodSync(sock,0o600);fs.writeFileSync(folder+'/daemon.pid',String(process.pid),{mode:0o600});});
function quit(){cancelPermissions();server.close();cli?.kill('SIGTERM');setTimeout(()=>process.exit(0),300);}process.on('SIGTERM',quit);process.on('SIGINT',quit);
