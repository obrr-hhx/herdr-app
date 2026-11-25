"""Bounded public tool views. Internal authority/delivery checks keep full data."""
import hashlib,json,os,pathlib,re,tempfile,time
from redaction import redact
MAX_OUTPUT=12000
PAGE_BUDGET=8000
PAGE_SIZE=8
PAGE_MAX=25
CACHE_MAX=16*1024*1024
CACHE_TTL=6*3600
FIELDS=('id','offset','match_offset','address','title','name','role','status','stage','phase','revision','host','folder','app','enabled','priority','needs_user','available','cached','source','detail','created_at','updated_at','observed_at','time','next_at','state','repeat','delivery','confirmed','verified','runtime_stopped','pending','request_id','question_id','version','compiled','scope','authority','historical_snapshot','goal_id','session_closed','summary','next_action','context','reason','text','content','answer')
LISTS={'list_sessions':'sessions','inspect_sessions':'sessions','list_decisions':'decisions','list_goals':'goals','list_reminders':'schedules','list_memories':'memories','search_memories':'memories','review_memories':'memories'}
COLLECTIONS={'list_sessions':['sessions','machines','tasks','goals','assignments','closed_sessions'],'list_reminders':['schedules','unread_reminders'],'review_memories':['memories','pending_human_feedback']}

def encoded(value):return json.dumps(value,ensure_ascii=False,separators=(',',':')).encode()
def clip(text,size):return text.encode()[:size].decode('utf-8',errors='ignore')
def number(a,key,default,maximum):
 value=a.get(key,default)
 if isinstance(value,bool) or not isinstance(value,int) or not (0 if key=='offset' else 1)<=value<=maximum:raise ValueError(f'Invalid {key}')
 return value

def params(a):
 offset=number(a,'offset',0,10000000);limit=number(a,'limit',PAGE_SIZE,PAGE_MAX);query=a.get('query','')
 if not isinstance(query,str) or len(query)>300:raise ValueError('query must be at most 300 characters')
 if 'status' in a and (not isinstance(a['status'],str) or len(a['status'])>200):raise ValueError('Invalid status filter')
 return offset,limit,query

def matches(value,query):
 def values(v):
  if isinstance(v,dict):return ' '.join(values(x) for x in v.values())
  if isinstance(v,list):return ' '.join(values(x) for x in v)
  return str(v)
 text=values(value).casefold()
 return all(term in text for term in query.casefold().split())

def pointer(base,key):return base+'/'+str(key).replace('~','~0').replace('/','~1')
def select(value,path):
 if not isinstance(path,str) or len(path)>1000 or path and not path.startswith('/'):raise ValueError('path must be a JSON pointer, for example /decisions/0/context')
 for raw in path.split('/')[1:] if path else []:
  key=raw.replace('~1','/').replace('~0','~')
  try:value=value[int(key)] if isinstance(value,list) and key.isdigit() else value[key]
  except (IndexError,KeyError,TypeError):raise ValueError('No such result field')
 return value

def summary(row,path):
 if not isinstance(row,dict):return {'value':clip(str(row),600),'detail_path':path,'summary_only':True}
 out={};shortened=[]
 for key in FIELDS:
  if key not in row:continue
  value=row[key]
  if isinstance(value,str):
   size=700 if key=='text' else 240 if key in ('summary','context','content','answer','reason','next_action') else 240
   out[key]=clip(value,size)
   if out[key]!=value:shortened.append(key)
  elif value is None or isinstance(value,(bool,int,float)):out[key]=value
 if isinstance(row.get('messages'),list) and row['messages']:
  last=row['messages'][-1]
  if isinstance(last,dict):out['latest_message']={'id':last.get('id'),'role':last.get('role'),'text_excerpt':clip(last.get('text',''),500),'detail_path':pointer(pointer(path,'messages'),len(row['messages'])-1)}
 for key,value in row.items():
  if isinstance(value,(list,dict)) and value:out[key+'_count']=len(value)
 if not out:out['fields']=list(row)[:20]
 out.update(detail_path=path,summary_only=True)
 if shortened:out['truncated_fields']=shortened
 return out

class Results:
 def __init__(self,home):self.root=pathlib.Path(home)/'data/super-agent/tool-results'
 def save(self,tool,value):
  data=encoded({'tool':tool,'result':redact(value)})
  if len(data)>CACHE_MAX:return None
  ident=hashlib.sha256(data).hexdigest();temp=None
  try:
   self.root.mkdir(parents=True,exist_ok=True,mode=0o700);os.chmod(self.root,0o700)
   dest=self.root/(ident+'.json')
   if dest.is_symlink():dest.unlink()
   if not dest.exists():
    fd,temp=tempfile.mkstemp(prefix='.result-',dir=self.root)
    with os.fdopen(fd,'wb') as f:f.write(encoded({**json.loads(data),'captured_at':int(time.time()*1000)}))
    os.replace(temp,dest);temp=None
   os.utime(dest,None)
   files=sorted((p for p in self.root.glob('*.json') if re.fullmatch('[a-f0-9]{64}.json',p.name)),key=lambda p:p.stat().st_mtime,reverse=True)
   size=0
   for i,p in enumerate(files):
    size+=p.stat().st_size
    if p!=dest and (i>=128 or size>64*1024*1024 or time.time()-p.stat().st_mtime>CACHE_TTL):p.unlink(missing_ok=True)
   return ident
  except OSError:return None  # A cache error must never disguise a completed write as failed.
  finally:
   if temp:
    try:os.unlink(temp)
    except OSError:pass
 def load(self,ident):
  if not isinstance(ident,str) or not re.fullmatch('[a-f0-9]{64}',ident):raise ValueError('Invalid result_id')
  p=self.root/(ident+'.json')
  try:
   if p.is_symlink() or p.stat().st_size>CACHE_MAX+1000 or time.time()-p.stat().st_mtime>CACHE_TTL:raise OSError()
   return redact(json.loads(p.read_text()))
  except (OSError,ValueError):raise ValueError('Result unavailable or expired. Refresh a read-only query; never repeat a write just to recover its result.')

def page(rows,a,base='',default_status=None):
 offset,limit,query=params(a);status=a.get('status',default_status or 'all');states=None if status=='all' else set(status.split(','))
 selected=[]
 for index,row in enumerate(rows):
  if a.get('address') and (not isinstance(row,dict) or row.get('address')!=a['address']):continue
  if states is not None and (not isinstance(row,dict) or row.get('status',row.get('phase',row.get('stage',row.get('state')))) not in states):continue
  if matches(row,query):selected.append((index,row))
 # Source order is stable within a result snapshot. No truncation is hidden.
 items=[];size=0
 for index,row in selected[offset:offset+limit]:
  item=summary(row,pointer(base,index));n=len(encoded(item))
  if items and size+n>PAGE_BUDGET:break
  items.append(item);size+=n
 end=offset+len(items);more=end<len(selected)
 return items,{'total':len(selected),'collection_total':len(rows),'offset':offset,'limit':limit,'returned':len(items),'next_offset':end if more else None,'has_more':more,'query':query,'status':status,'summary_only':True}

def ref(ident):return {'result_id':ident,'details_tool':'read_tool_result'} if ident else {'details_unavailable':True,'details_hint':'Use a smaller read-only source query; do not repeat a mutation.'}

def capped(value,ident,budget=MAX_OUTPUT):
 value={**value,**ref(ident)}
 if len(encoded(value))<=budget:return value
 # Keep execution receipts exact. Full data remains in the private result snapshot.
 keys=('confirmed','verified','runtime_stopped','queued','pending','refused','missing','enabled','compiled','authority','id','request_id','address','pane','terminal','revision','version','status','stage','delivery','error','phase','scope','source','next_offset','has_more')
 out={**ref(ident),'response_shortened':True,'detail_path':''};omitted=[]
 for key in keys:
  if key not in value:continue
  if len(encoded(value[key]))>=1000 or len(encoded({**out,key:value[key]}))>budget-500:omitted.append(key)
  else:out[key]=value[key]
 if omitted:out['receipt_fields_omitted']=omitted
 return out

def read_result(home,a):
 cache=Results(home);record=cache.load(a.get('result_id'));data=select(record['result'],a.get('path',''));ident=a['result_id'];query=a.get('query','')
 if not isinstance(query,str) or len(query)>300:raise ValueError('Invalid query')
 def done(value):return capped({**value,'snapshot_at':record.get('captured_at')},ident)
 if isinstance(data,str) and not query:
  offset=number(a,'offset',0,100000000);limit=number(a,'limit',2000,2000);text=data[offset:offset+limit]
  while len(encoded(text))>PAGE_BUDGET:text=text[:max(1,len(text)*3//4)]
  end=offset+len(text)
  return done({'text':text,'offset':offset,'total':len(data),'next_offset':end if end<len(data) else None,'has_more':end<len(data),'verbatim':True,'path':a.get('path',''),'snapshot_tool':record['tool'],'cached':True})
 if isinstance(data,str):
  offset,limit,_=params(a);rows=[];total=0
  for m in re.finditer(re.escape(query),data,re.I):
   if offset<=total<offset+limit:rows.append({'match_offset':m.start(),'text':data[max(0,m.start()-100):m.end()+200]})
   total+=1
  items,meta=page(rows,{'limit':limit})
  for item in items:item['detail_path']=a.get('path','')
  end=offset+len(items)
  return done({'matches':items,**meta,'offset':offset,'total':total,'collection_total':total,'next_offset':end if end<total else None,'has_more':end<total,'query':query,'path':a.get('path',''),'cached':True})
 if isinstance(data,list):
  items,meta=page(data,a,a.get('path',''));return done({'items':items,**meta,'path':a.get('path',''),'cached':True})
 if isinstance(data,dict):
  rows=[{'id':k,'value':v} for k,v in data.items()];items,meta=page(rows,a)
  for item in items:
   index=int(item['detail_path'].rsplit('/',1)[-1]);key=rows[index]['id'];value=data[key]
   item['detail_path']=pointer(a.get('path',''),key)
   item['value']=clip(value,600) if isinstance(value,str) else value if value is None or isinstance(value,(bool,int,float)) else {'type':type(value).__name__,'count':len(value)}
  return done({'fields':items,**meta,'path':a.get('path',''),'cached':True})
 return done({'value':data,'path':a.get('path',''),'cached':True})

def invoke(home,name,a,raw):
 if name=='read_tool_result':return read_result(home,a)
 cache=Results(home)
 if name in LISTS:
  params(a)
  if name=='inspect_sessions' and a.get('result_id'):raise ValueError('Inspect fresh inventory pages with offset; use read_tool_result for cached details.')
  if a.get('result_id'):
   record=cache.load(a['result_id'])
   if record['tool']!=name:raise ValueError('result_id belongs to another tool')
   value=record['result']
  elif name in ('list_memories','search_memories'):value=raw('_memory_snapshot',{})
  elif name=='review_memories':value=raw(name,{})
  elif name=='list_sessions':value=raw('_inventory',{'collection':a.get('collection','sessions')})
  elif name=='list_reminders':value=raw(name,{'timezone':a.get('timezone','Asia/Shanghai')})
  elif name=='inspect_sessions':
   inventory=raw('_inventory',{});rows=[r for r in inventory.get('sessions',[]) if not r.get('super_agent') and (a.get('addresses') is None or r.get('address') in a['addresses'])]
   selected,meta=page(rows,a,'/sessions');addresses=[r['address'] for r in selected]
   value=raw(name,{'addresses':addresses}) if addresses else {'sessions':[]}
   value=redact(value);ident=cache.save(name,value);items,_=page(value.get('sessions',[]),{'limit':PAGE_MAX},'/sessions')
   meta.update(next_offset=meta['offset']+len(items) if meta['offset']+len(items)<meta['total'] else None,has_more=meta['offset']+len(items)<meta['total'])
   return capped({'sessions':items,**meta,'returned':len(items),'inspected_at':value.get('inspected_at'),'inspection_partial':meta['has_more']},ident)
  else:value=raw(name,{})
  value=redact(value);ident=cache.save(name,value);collection=a.get('collection',LISTS[name]);allowed=COLLECTIONS.get(name,[LISTS[name]])
  if collection not in allowed:raise ValueError('Unknown collection')
  rows=value.get(collection,[])
  if not isinstance(rows,list):raise ValueError('Collection unavailable')
  items,meta=page(rows,a,pointer('',collection),'pending,answered' if name=='list_decisions' else None)
  result={collection:items,**meta,'cached':bool(a.get('result_id')),'snapshot_at':record.get('captured_at') if a.get('result_id') else int(time.time()*1000),'collection':collection,'collections':{k:len(v) for k,v in value.items() if isinstance(v,list)}}
  for key in ('version','now','now_local','timezone','scope','authority','policy','receipt_reconciled'):
   if key in value:result[key]=value[key]
  if name=='list_sessions':
   intent=value.get('latest_user_intent') or {};result['latest_user_intent']={k:clip(v,500) if isinstance(v,str) else v for k,v in intent.items() if k in ('id','time','text')};result['intent_detail_path']='/latest_user_intent'
   if isinstance(intent.get('text'),str) and len(intent['text'].encode())>500:result['latest_user_intent']['text_truncated']=True
  if name=='review_memories':
   result['pending_feedback_count']=len(value.get('pending_human_feedback',[]));result['pending_feedback_ids']=[r['id'] for r in value.get('pending_human_feedback',[])];result['review_instruction']='Read all memory and feedback pages and required full text before consolidating; a summary is not complete source evidence.'
  return capped(result,ident)
 if name=='read_session':
  offset,limit,query=params(a);start=offset;found=[];source=None;scanned=0
  while scanned<120 and len(found)<limit:
   count=min(40,120-scanned) if query else limit
   source=redact(raw(name,{'address':a['address'],'offset':offset,'limit':count}))
   messages=source.get('messages',[]);consumed=0
   for message in reversed(messages):
    offset+=1;scanned+=1;consumed+=1
    if matches(message,query):found.append(message)
    if len(found)>=limit:break
   if not query or len(found)>=limit or not source.get('has_more') or not messages:break
  more=bool(messages) and (consumed<len(messages) or bool(source.get('has_more',len(messages)==count)))
  value={**(source or {}),'messages':list(reversed(found))};ident=cache.save(name,value)
  items,_=page(value['messages'],{'limit':PAGE_MAX},'/messages')
  result={k:v for k,v in value.items() if k in ('status','source','available','cached','partial','captured_at','content_updated_at','last_known_only','active_turn','steer_available','delivery','read_error')}
  result.update(messages=items,has_more=more,messages_in_snapshot=len(value['messages']),snapshot_messages_remaining=len(value['messages'])-len(items),offset=start,next_offset=offset if more else None,query=query,searched_messages=scanned,search_scope='bounded source history; continue with next_offset',partial=True)
  if source and not source.get('messages') and source.get('text'):
   result['text']=clip(source['text'],3000) if matches(source['text'],query) else '';result['text_excerpt']=True
  for key in ('pending_questions','pending_permissions','dispatches'):
   data=value.get(key,{})
   result[key+'_count']=len(data);result[key+'_detail_path']='/'+key
   if data:
    result[key]=[{**summary(v,pointer('/'+key,k)),'id':k} for k,v in list(data.items())[:PAGE_SIZE]] if isinstance(data,dict) else [summary(v,pointer('/'+key,k)) for k,v in enumerate(data[:PAGE_SIZE])]
  return capped(result,ident)
 args={k:v for k,v in a.items() if k not in ('offset','limit','query','path')} if name=='read_memory' else a
 value=redact(raw(name,args));ident=cache.save(name,value)
 if name=='read_memory' and any(k in a for k in ['query','offset','limit','path']):
  if not ident:raise ValueError('Detail snapshot unavailable; use a smaller read-only query')
  return read_result(home,{**a,'result_id':ident,'path':a.get('path','/memory/content')})
 if name=='read_memory':
  row=value.get('memory');return capped({'memory':summary(row,'/memory') if row else None,'version':value.get('version'),'missing':value.get('missing')},ident)
 return capped(value if isinstance(value,dict) else {'value':value},ident,budget=4000)
