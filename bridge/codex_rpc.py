"""Codex local app-server JSON-RPC over its documented Unix WebSocket transport."""
import base64
import hashlib
import json
import os
import queue
import socket
import stat
import struct
import threading


class CodexRPC:
    def __init__(self, path, on_message=lambda _: None, on_close=lambda: None):
        meta=os.stat(path)
        if not stat.S_ISSOCK(meta.st_mode) or hasattr(os,'getuid') and meta.st_uid!=os.getuid():
            raise ValueError('Codex socket is not owned by this user')
        self.socket=socket.socket(socket.AF_UNIX);self.socket.settimeout(5)
        try:
            self.socket.connect(str(path));self.buffer=b'';self.pending={};self.serial=0
            self.lock=threading.RLock();self.writer=threading.Lock();self.closed=False
            self.on_message,self.on_close=on_message,on_close
            key=base64.b64encode(os.urandom(16)).decode()
            self.socket.sendall(f'GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {key}\r\n\r\n'.encode())
            while b'\r\n\r\n' not in self.buffer:
                data=self.socket.recv(4096)
                if not data or len(self.buffer)>16384:raise ValueError('Codex WebSocket handshake failed')
                self.buffer+=data
            header,self.buffer=self.buffer.split(b'\r\n\r\n',1)
            accept=base64.b64encode(hashlib.sha1((key+'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest())
            if b' 101 ' not in header.split(b'\r\n')[0] or accept.lower() not in header.lower():
                self.socket.close();raise ValueError('Codex WebSocket upgrade rejected')
            self.socket.settimeout(None)
            self.reader=threading.Thread(target=self._read,daemon=True);self.reader.start()
            self.rpc('initialize',{'clientInfo':{'name':'herdr-session-bridge','version':'0.1.0'},'capabilities':{'experimentalApi':True}},timeout=5)
            self.send({'method':'initialized'})
        except Exception:
            self.socket.close();raise

    def _exact(self,n):
        while len(self.buffer)<n:
            data=self.socket.recv(max(4096,min(n-len(self.buffer),65536)))
            if not data:raise EOFError()
            self.buffer+=data
        result,self.buffer=self.buffer[:n],self.buffer[n:]
        return result

    def _frame(self,raw,opcode=1):
        n=len(raw);mask=os.urandom(4)
        header=bytes([0x80|opcode,0x80|n]) if n<126 else bytes([0x80|opcode,0x80|126])+struct.pack('!H',n) if n<65536 else bytes([0x80|opcode,0x80|127])+struct.pack('!Q',n)
        with self.writer:self.socket.sendall(header+mask+bytes(b^mask[i%4] for i,b in enumerate(raw)))

    def send(self,value):
        if self.closed:raise ConnectionError('Codex connection closed')
        self._frame(json.dumps(value,ensure_ascii=False).encode())

    def rpc(self,method,params,timeout=5):
        with self.lock:
            self.serial+=1;ident=f'herdr-{self.serial}';reply=queue.Queue(1);self.pending[ident]=reply
        try:
            self.send({'id':ident,'method':method,'params':params})
            value=reply.get(timeout=timeout)
            if isinstance(value,Exception):raise value
            if 'error' in value:raise ValueError('Codex rejected '+method)
            return value.get('result',{})
        except queue.Empty:raise TimeoutError('Codex request timed out')
        finally:
            with self.lock:self.pending.pop(ident,None)

    def _read(self):
        fragments=b''
        try:
            while not self.closed:
                h=self._exact(2);opcode=h[0]&15;length=h[1]&127
                length=struct.unpack('!H',self._exact(2))[0] if length==126 else struct.unpack('!Q',self._exact(8))[0] if length==127 else length
                if length>16*1024*1024 or len(fragments)+length>16*1024*1024:raise ValueError('Codex frame too large')
                mask=self._exact(4) if h[1]&128 else None;raw=self._exact(length)
                if mask:raw=bytes(b^mask[i%4] for i,b in enumerate(raw))
                if opcode==8:raise EOFError()
                if opcode==9:self._frame(raw,10);continue
                if opcode==10:continue
                if opcode not in (0,1):raise ValueError('Unsupported Codex frame')
                fragments+=raw
                if not h[0]&128:continue
                value=json.loads(fragments);fragments=b''
                with self.lock:reply=self.pending.get(value.get('id')) if 'method' not in value else None
                if reply:reply.put_nowait(value)
                else:self.on_message(value)
        except (OSError,ValueError,EOFError,KeyError,TypeError,queue.Full):pass
        finally:
            self.closed=True
            with self.lock:
                for reply in self.pending.values():
                    try:reply.put_nowait(ConnectionError('Codex connection lost'))
                    except queue.Full:pass
            self.on_close()

    def close(self):
        self.closed=True
        try:self.socket.shutdown(socket.SHUT_RDWR)
        except OSError:pass
        self.socket.close()
