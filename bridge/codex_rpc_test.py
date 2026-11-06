import base64
import hashlib
import json
import pathlib
import queue
import socket
import struct
import tempfile
import threading
import unittest
from codex_rpc import CodexRPC

class Transport(unittest.TestCase):
    def test_real_unix_websocket_handles_fragmented_response_and_server_request(self):
        with tempfile.TemporaryDirectory(prefix='hcws-',dir='/tmp') as root:
            endpoint=pathlib.Path(root)/'rpc.sock';server=socket.socket(socket.AF_UNIX);server.bind(str(endpoint));server.listen();failures=[];received=queue.Queue()
            def peer():
                try:
                    with server.accept()[0] as conn:
                        raw=b''
                        while b'\r\n\r\n' not in raw:raw+=conn.recv(4096)
                        key=next(line.split(b': ',1)[1] for line in raw.split(b'\r\n') if line.startswith(b'Sec-WebSocket-Key:'))
                        accept=base64.b64encode(hashlib.sha1(key+b'258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest())
                        conn.sendall(b'HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: '+accept+b'\r\n\r\n')
                        def exact(n):
                            data=b''
                            while len(data)<n:
                                block=conn.recv(n-len(data))
                                if not block:raise EOFError()
                                data+=block
                            return data
                        def read():
                            h=exact(2);n=h[1]&127
                            if n==126:n=struct.unpack('!H',exact(2))[0]
                            if n==127:n=struct.unpack('!Q',exact(8))[0]
                            self.assertTrue(h[1]&128,'Clients must mask WebSocket frames')
                            mask=exact(4);raw=exact(n);raw=bytes(b^mask[i%4] for i,b in enumerate(raw))
                            return h[0]&15,raw
                        def send(value,fragment=False):
                            body=json.dumps(value).encode()
                            if fragment:
                                cut=len(body)//2;conn.sendall(bytes([1,cut])+body[:cut]+bytes([128,len(body)-cut])+body[cut:])
                            else:conn.sendall(bytes([129,len(body)])+body)
                        _,raw=read();init=json.loads(raw);send({'id':init['id'],'result':{}},True)
                        read() # initialized
                        _,raw=read();request=json.loads(raw)
                        conn.sendall(b'\x89\x02ok');send({'id':request['id'],'result':{'data':['thread']}},True)
                        send({'id':27,'method':'question','params':{'threadId':'thread'}})
                        opcode,payload=read();self.assertEqual((opcode,payload),(10,b'ok'))
                        opcode,payload=read();received.put(json.loads(payload))
                except Exception as e:failures.append(e)
            thread=threading.Thread(target=peer,daemon=True);thread.start();messages=queue.Queue()
            client=CodexRPC(endpoint,on_message=messages.put)
            try:
                self.assertEqual(client.rpc('thread/loaded/list',{}),{'data':['thread']})
                question=messages.get(timeout=2);self.assertEqual(question['id'],27)
                client.send({'id':27,'result':{'answers':{}}});self.assertEqual(received.get(timeout=2)['id'],27)
            finally:client.close();thread.join(timeout=2);server.close()
            self.assertFalse(failures,failures)
    def test_regular_file_is_never_treated_as_codex_socket(self):
        with tempfile.NamedTemporaryFile() as file:
            with self.assertRaises(ValueError):CodexRPC(file.name)

if __name__=='__main__':unittest.main()
