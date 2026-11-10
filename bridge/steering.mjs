// Native Claude steering is sendNow in the existing ordinary interactive PTY.
export const normalizePrompt=text=>text.replaceAll('\r\n','\n').trim();
export class Steering {
 pending=null;
 refusal(c,{manager,supported,busy,ready,approval,turn}){
  if(manager||!supported)return 'This runtime does not support native steering; update/reconnect when idle.';
  if(!busy||!ready||!turn)return 'No verified active turn; send a regular message instead.';
  if(approval)return 'Resolve the pending question or tool approval first; no steering message was sent.';
  if(c.expected_turn!==turn)return 'The active turn changed; inspect this same conversation before steering.';
  if(this.pending)return 'A steering message is awaiting Claude confirmation; do not resend it.';
  if(/^[\s]*[\/!]/.test(c.text))return 'Steering accepts conversation text, not slash or shell commands.';
  return null;
 }
 begin(c,prompt){this.pending={command:c,prompt:normalizePrompt(prompt)};}
 confirm(prompt){if(typeof prompt!=='string'||!this.pending||normalizePrompt(prompt)!==this.pending.prompt)return null;const c=this.pending.command;this.pending=null;return c;}
 cancel(){const c=this.pending?.command;this.pending=null;return c;}
}

export function unconfirmedSteering(events){
 const ids=new Set(events.filter(e=>e.type==='user'&&e.mode==='steer').map(e=>e.id));
 for(const e of events)if(e.type==='prompt_confirmed'||e.type==='dispatch_receipt'&&['refused','cancelled','confirmed'].includes(e.stage))ids.delete(e.id);
 return [...ids];
}
