import {memoryAdditionalContext} from './memory_context.mjs';
// Codex's supported app-server protocol; no TUI parsing and no exec/print mode.
import {readOnlyTurn,readOnlyTool} from './checkins.mjs';
import crypto from 'node:crypto';import fs from 'node:fs';import path from 'node:path';import readline from 'node:readline';import {spawn} from 'node:child_process';import {EventEmitter} from 'node:events';import {fileURLToPath} from 'node:url';
export function errorLabel(info){
 const allowed=['contextWindowExceeded','sessionBudgetExceeded','usageLimitExceeded','rateLimitExceeded','flexUnavailable','serverOverloaded','modelNotFound','policyCapacityUnavailable','cyberPolicy','misalignmentPolicyViolation','tooManyDenials','internalServerError','unauthorized','badRequest','threadRollbackFailed','sandboxError','other'];
 if(typeof info==='string')return allowed.includes(info)?info:'other';
 for(const name of ['httpConnectionFailed','responseStreamConnectionFailed','responseStreamDisconnected','responseTooManyFailedAttempts'])if(info?.[name]){const code=info[name].httpStatusCode;return name+(Number.isInteger(code)&&code>=100&&code<600?` HTTP ${code}`:'');}
 return 'other';
}
const base=path.dirname(fileURLToPath(import.meta.url));
function toolProcess(args){return new Promise((resolve,reject)=>{
 const p=spawn('python3',[path.join(base,'supervisor.py'),...args],{stdio:['ignore','pipe','ignore']});let out='';
 const timer=setTimeout(()=>{p.kill();reject(Error('management_timeout'));},35000);
 p.stdout.on('data',b=>{out+=b;if(Buffer.byteLength(out)>1024*1024)p.kill();});p.on('error',reject);
 p.on('exit',code=>{clearTimeout(timer);try{const r=JSON.parse(out);resolve({ok:code===0,value:r});}catch{reject(Error('management_unavailable'));}});
});}
export function codexRunner(folder,cwd,callbacks,settings={}){
 const runner=new EventEmitter(),pending=new Map();let next=0,thread='',turn='',command=null,closed=false,lastError='other',models=[];
 const app=spawn(process.env.HERDR_CODEX_BIN??'codex',['app-server','--stdio','-c','apps._default.enabled=false','-c','web_search="disabled"'],{stdio:['pipe','pipe','ignore']});
 const write=v=>app.stdin.write(JSON.stringify(v)+'\n');
 const rpc=(method,params)=>new Promise((resolve,reject)=>{const id=++next;const timer=setTimeout(()=>{pending.delete(id);reject(Error('app_server_timeout'));},30000);pending.set(id,{resolve,reject,timer});write({id,method,params});});
 app.stdin.on('error',()=>{});app.on('error',()=>callbacks.failure('Codex could not start. Install Codex CLI and sign in locally.',true));
 runner.kill=()=>{closed=true;app.kill('SIGTERM');const timer=setTimeout(()=>{if(app.exitCode===null&&app.signalCode===null)app.kill('SIGKILL');},5000);timer.unref();};
 runner.stdin={on(){},write(line){let c;try{c=JSON.parse(line);}catch{return;}
  if(c.action==='interrupt'){if(turn)rpc('turn/interrupt',{threadId:thread,turnId:turn}).catch(()=>callbacks.failure('Could not interrupt Codex'));return;}
  if(c.action!=='send')return;
  command=c;
  const input=[...(c.text?[{type:'text',text:c.text}]:[]),...(c.image_paths??[]).map(p=>({type:'localImage',path:p}))];
  rpc('turn/start',{threadId:thread,input,...(Object.hasOwn(c,'memory_snapshot')?{additionalContext:memoryAdditionalContext(c.memory_snapshot)}:{}),clientUserMessageId:c.request_id,environments:[],...(settings.model&&settings.model!=='default'?{model:settings.model}:{}),effort:settings.effort||models.find(m=>m.model===settings.model)?.defaultReasoningEffort||null}).then(r=>{turn=r.turn.id;callbacks.confirmed(c.request_id);}).catch(()=>{if(closed)return;command=null;callbacks.failure('Codex did not confirm this turn. Inspect the conversation before retrying.');});
 }};
 const pieces=new Map();
 readline.createInterface({input:app.stdout}).on('line',async line=>{
  if(closed)return;let m;try{m=JSON.parse(line);}catch{return;}
  if(m.id!==undefined&&pending.has(m.id)&&!m.method){const p=pending.get(m.id);pending.delete(m.id);clearTimeout(p.timer);m.error?p.reject(Error('app_server_error')):p.resolve(m.result);return;}
  if(m.method==='item/tool/call'&&m.id!==undefined){
   try{
    if(readOnlyTurn(command?.request_id)&&!readOnlyTool(m.params.tool,command?.request_id)){write({id:m.id,result:{contentItems:[{type:'inputText',text:'Session inspections are read-only; wait for explicit human authority.'}],success:false}});return;}
    callbacks.tool(m.params.callId,m.params.tool,m.params.arguments);
    const result=await toolProcess(['--call',m.params.tool,JSON.stringify(m.params.arguments)]);
    callbacks.toolResult(m.params.callId,result.value,!result.ok);
    write({id:m.id,result:{contentItems:[{type:'inputText',text:JSON.stringify(result.value)}],success:result.ok}});
   }catch{write({id:m.id,result:{contentItems:[{type:'inputText',text:'Management tool unavailable; no automatic retry.'}],success:false}});}return;
  }
  if(m.id!==undefined&&m.method){write({id:m.id,error:{code:-32601,message:'This manager only executes session management tools. Ask the user in chat.'}});return;}
  const p=m.params??{};
  if(p.threadId&&thread&&p.threadId!==thread)return;
  if(m.method==='item/agentMessage/delta'){const index=pieces.get(p.itemId)??0;pieces.set(p.itemId,index+1);callbacks.delta('codex:'+p.itemId,index,p.delta);}
  if(m.method==='item/completed'&&p.item?.type==='agentMessage')callbacks.complete('codex:'+p.item.id);
  if(m.method==='turn/completed'){turn='';command=null;if(p.turn?.status==='failed')callbacks.failure(`Codex turn failed (${lastError}). Check account availability and connection.`);else callbacks.done();}
  if(m.method==='thread/tokenUsage/updated')callbacks.usage(p.tokenUsage);
  if(m.method==='error')lastError=errorLabel(p.error?.codexErrorInfo);
 });
 app.on('exit',()=>{for(const p of pending.values()){clearTimeout(p.timer);p.reject(Error('app_server_stopped'));}pending.clear();runner.emit('exit');});
 (async()=>{
  await rpc('initialize',{clientInfo:{name:'herdr-super-agent',version:'0.1.0'},capabilities:{experimentalApi:true}});write({method:'initialized'});
  const toolResult=await toolProcess(['--tools']);if(!toolResult.ok)throw Error('tool_schema');
  const tools=toolResult.value.map(t=>({...t,type:'function'}));
  const config={ 'apps._default.enabled':false,web_search:'disabled'};
  // Disable inherited MCP servers by name without exposing their credentials.
  const cfg=await rpc('config/read',{includeLayers:false});
  for(const name of Object.keys(cfg.config?.mcp_servers??{}))config[`mcp_servers.${name}.enabled`]=false;
  if(settings.effort)config.model_reasoning_effort=settings.effort;
  const catalog=[];let cursor=null;do{const page=await rpc('model/list',{limit:100,cursor,includeHidden:false});catalog.push(...(page.data??[]));cursor=page.nextCursor??null;}while(cursor&&catalog.length<500);
  models=catalog.filter(m=>!m.hidden).map(m=>({model:m.model,displayName:m.displayName,description:m.description,defaultReasoningEffort:m.defaultReasoningEffort,supportedReasoningEfforts:m.supportedReasoningEfforts}));callbacks.models?.(models);
  const options={...(settings.model&&settings.model!=='default'?{model:settings.model}:{}),cwd,environments:[],sandbox:'read-only',approvalPolicy:'never',baseInstructions:fs.readFileSync(path.join(base,'supervisor-prompt.md'),'utf8'),config};
  const file=path.join(folder,'codex-thread.json');let saved;try{saved=JSON.parse(fs.readFileSync(file,'utf8'));}catch{}
  const schemaHash=crypto.createHash('sha256').update(JSON.stringify(tools)).digest('hex');
  const canResume=saved?.threadId&&saved.toolSchemaHash===schemaHash;
  const r=canResume?await rpc('thread/resume',{...options,threadId:saved.threadId,excludeTurns:true}):await rpc('thread/start',{...options,dynamicTools:tools});
  if(saved?.threadId&&!canResume){fs.writeFileSync(path.join(folder,'codex-thread-previous-'+Date.now()+'.json'),JSON.stringify(saved),{mode:0o600});callbacks.notice?.('Manager tools updated. Previous backend history is retained; the shared task ledger and visible chat history are unchanged.');}
  thread=r.thread.id;const tmp=file+'.new';fs.writeFileSync(tmp,JSON.stringify({threadId:thread,toolSchemaHash:schemaHash}),{mode:0o600});fs.renameSync(tmp,file);
  if(closed)return;callbacks.ready(r.model??'Codex');
 })().catch(()=>{if(!closed)callbacks.failure('Could not initialize Codex. Check the signed-in CLI and app-server compatibility.',true);runner.kill();});
 return runner;
}
