"""User preferences for read-only, inactivity-triggered Super Agent reports."""
import json
import pathlib
import time

DEFAULTS={'enabled':True,'idle_minutes':15,'interval_minutes':30,'desktop_notifications':True}

def path(home):return pathlib.Path(home)/'data/super-agent/checkins.json'
def load(home):
    try:
        value=json.loads(path(home).read_text())
        result=dict(DEFAULTS)
        for k,v in value.items():
            if k not in DEFAULTS:continue
            valid=isinstance(v,bool) if isinstance(DEFAULTS[k],bool) else isinstance(v,int) and not isinstance(v,bool) and 1<=v<=1440
            if valid:result[k]=v
        return result
    except (OSError,ValueError,AttributeError):return dict(DEFAULTS)

def configure(home,args):
    value=load(home)
    for name in DEFAULTS:
        if name not in args:continue
        v=args[name]
        if isinstance(DEFAULTS[name],bool):
            if not isinstance(v,bool):raise ValueError(name+' must be a boolean')
        elif isinstance(v,bool) or not isinstance(v,int) or not 1<=v<=1440:raise ValueError(name+' must be between 1 and 1440 minutes')
        value[name]=v
    target=path(home);target.parent.mkdir(parents=True,exist_ok=True,mode=0o700)
    import os,tempfile
    with tempfile.NamedTemporaryFile(mode='w',dir=target.parent,delete=False) as file:
        json.dump({**value,'updated_at':int(time.time()*1000)},file);temp=file.name
    os.replace(temp,target)
    return describe(home)

def describe(home):
    latest=None
    try:
        journal=pathlib.Path(home)/'data/local-chat/73757065726167656e74000000000001/events.jsonl'
        with journal.open('rb') as file:
            size=file.seek(0,2);file.seek(max(0,size-2*1024*1024));lines=file.read().splitlines()
        for line in reversed(lines):
            try:event=json.loads(line)
            except ValueError:continue
            if event.get('type')=='followup_plan':latest=event;break
    except OSError:pass
    return {'next_followup':latest,**load(home),'mode':'adaptive_followup','legacy_interval_note':'interval_minutes is retained for compatibility; adaptive scheduling does not repeat at that interval.','delivery':'While Herdr App is open, signed in and awake; missed intervals coalesce into one report. Busy turns and current human messages take priority. Uses the selected Super Agent model; each report consumes model usage. No report changes worker tasks, answers questions or approves operations.'}


def plan(home,args):
    """Resolve this live manager observation through its existing private socket."""
    import socket,hashlib
    context=json.loads((pathlib.Path(home)/'data/super-agent/active-observation.json').read_text())
    if not context.get('id','').startswith('observe_'):raise ValueError('Follow-up planning is only available during an active observation')
    request='followup_'+hashlib.sha256(json.dumps([context['id'],args],sort_keys=True).encode()).hexdigest()[:40]
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(5);client.connect(context['socket']);buffer=b''
        def receive():
            nonlocal buffer
            while b'\n' not in buffer:
                block=client.recv(65536)
                if not block or len(buffer)>2*1024*1024:raise ValueError('Follow-up bridge unavailable')
                buffer+=block
            line,buffer=buffer.split(b'\n',1);return json.loads(line)
        client.sendall((json.dumps({'action':'subscribe','after':context['cursor']})+'\n').encode())
        while receive().get('type')!='connected':pass
        client.sendall((json.dumps({'action':'plan_followup','request_id':request,'observation_id':context['id'],**args})+'\n').encode())
        while True:
            event=receive()
            if event.get('request_id')==request:
                if event.get('type')=='ack':return event['plan']
                if event.get('type')=='command_error':raise ValueError('Observation changed or the decision is invalid; no follow-up was scheduled')
