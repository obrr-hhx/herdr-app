"""Accept bounded PNG uploads over SSH stdin; never accept arbitrary remote paths."""
import base64,hashlib,json,os,pathlib,re,struct,sys,tempfile
MAX_BYTES=8*1024*1024
def receive(home,identity,payload):
    if not re.fullmatch(r'[a-f0-9]{16,64}',identity):raise ValueError('conversation')
    entries=payload.get('images')
    if not isinstance(entries,list)or not 1<=len(entries)<=4:raise ValueError('count')
    base=pathlib.Path(os.environ.get('HERDR_CHAT_HOME',str(home/'.local/share/herdr-chat')))/'conversations';base.mkdir(parents=True,exist_ok=True,mode=0o700)
    root=base/identity
    if root.is_symlink():raise ValueError('symlink')
    root.mkdir(exist_ok=True,mode=0o700);dest=root/'images'
    if dest.is_symlink():raise ValueError('symlink')
    dest.mkdir(exist_ok=True,mode=0o700);ids=[];total=0
    for entry in entries:
        ident=entry.get('id','')
        if not isinstance(ident,str)or not re.fullmatch(r'[a-f0-9]{64}',ident):raise ValueError('id')
        raw=base64.b64decode(entry.get('data',''),validate=True);total+=len(raw)
        if not 24<=len(raw)<=MAX_BYTES or total>4*MAX_BYTES:raise ValueError('size')
        if raw[:8]!=b'\x89PNG\r\n\x1a\n'or raw[12:16]!=b'IHDR':raise ValueError('png')
        w,h=struct.unpack('>II',raw[16:24])
        if not 1<=w<=8192 or not 1<=h<=8192 or hashlib.sha256(raw).hexdigest()!=ident:raise ValueError('content')
        p=dest/(ident+'.png')
        if p.is_symlink():raise ValueError('symlink')
        fd,tmp=tempfile.mkstemp(dir=dest,prefix='.upload-')
        try:
            with os.fdopen(fd,'wb')as f:f.write(raw)
            os.replace(tmp,p)
        finally:
            if os.path.exists(tmp):os.unlink(tmp)
        ids.append(ident)
    return ids
if __name__=='__main__':
    try:
        data=sys.stdin.buffer.read(46*1024*1024+1)
        if len(data)>46*1024*1024:raise ValueError('payload')
        ids=receive(pathlib.Path.home(),sys.argv[1],json.loads(data));print(json.dumps({'ok':True,'images':ids}))
    except Exception:
        print(json.dumps({'ok':False}));sys.exit(1)
