#!/usr/bin/env python3
"""Loopback-only image server. Records synthetic paths, never real mail."""
import base64
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
import sys

root = Path(sys.argv[1])
png = base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=')

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass
    def do_GET(self):
        with (root / 'requests.txt').open('a') as log:
            log.write(self.path + '\n')
        self.send_response(200)
        self.send_header('Content-Type', 'image/png' if self.path.endswith('.png') else 'text/plain')
        self.end_headers()
        self.wfile.write(png if self.path.endswith('.png') else b'blocked resource')

server = HTTPServer(('127.0.0.1', 0), Handler)
(root / 'requests.txt').write_text('')
(root / 'port').write_text(str(server.server_port))
server.serve_forever()
