// Hooks can arrive out of order, including a final batch before an earlier one.
export class Display {
  constructor(emit) {this.emit=emit;this.messages=new Map();}
  accept(e) {
    if(typeof e.message_id!=='string'||!Number.isSafeInteger(e.index)||e.index<0||typeof e.delta!=='string')throw Error('invalid_display');
    let m=this.messages.get(e.message_id);
    if(!m){m={next:0,batches:new Map(),final:null};this.messages.set(e.message_id,m);}
    if(e.index<m.next)return;
    if(e.final)m.final=e.index;
    m.batches.set(e.index,e);
    while(m.batches.has(m.next)){
      const b=m.batches.get(m.next);m.batches.delete(m.next++);
      this.emit('delta',{id:e.message_id,index:b.index,text:b.delta,kind:'text_delta'});
    }
    if(m.final!==null&&m.next>m.final)this.emit('message_complete',{id:e.message_id});
  }
}
