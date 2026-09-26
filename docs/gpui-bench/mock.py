#!/usr/bin/env python3
"""Mock Torznab (serves a fixed fixture) + mock rqbit (records POSTs, adds nothing)."""
import sys, threading, json, time
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
FIXTURE = open(sys.argv[1], 'rb').read()
LOG = sys.argv[2]
class Torznab(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.send_header('Content-Type', 'application/rss+xml; charset=utf-8')
        self.send_header('Content-Length', str(len(FIXTURE))); self.end_headers(); self.wfile.write(FIXTURE)
    def log_message(self, *a): pass
class Rqbit(BaseHTTPRequestHandler):
    def do_GET(self):
        b = b'{"torrents":[]}'
        self.send_response(200); self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(b))); self.end_headers(); self.wfile.write(b)
    def do_POST(self):
        n = int(self.headers.get('Content-Length') or 0); body = self.rfile.read(n).decode('utf-8', 'replace')
        with open(LOG, 'a') as f: f.write(json.dumps({'t': time.time(), 'path': self.path, 'body': body}) + '\n')
        b = b'{"id":0}'
        self.send_response(200); self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(b))); self.end_headers(); self.wfile.write(b)
    def log_message(self, *a): pass
for port, h in ((18420, Torznab), (18030, Rqbit)):
    s = ThreadingHTTPServer(('127.0.0.1', port), h)
    threading.Thread(target=s.serve_forever, daemon=True).start()
print('mocks up', flush=True)
threading.Event().wait()
