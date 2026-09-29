#!/usr/bin/env python3
"""Synthetic, loopback-only TLS IMAP and STARTTLS SMTP integration fixture."""
import argparse
import base64
import email
import json
import re
import socketserver
import ssl
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from proxy_fixture import Proxy

STATE = {"validity": 7, "seen": False, "body_fetches": 0, "attachment_fetches": 0,
         "smtp_accepted": 0, "smtp_dropped": 0, "append_count": 0, "bcc_header_seen": False, "proxy_connects": 0}
BODY = b"Hello Bailey,\r\n\r\nPlease review the attached file. Amount USD 12.50.\r\n\r\nThanks."
ATTACHMENT = b"fixture attachment bytes"
HEADER = b"From: Fixture Sender <sender@example.com>\r\nTo: recipient@example.com\r\nSubject: Synthetic integration mail\r\nMessage-ID: <fixture-1@example.com>\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n"
STRUCTURE = f'(("TEXT" "PLAIN" ("CHARSET" "UTF-8") NIL NIL "8BIT" {len(BODY)} 6)("APPLICATION" "OCTET-STREAM" ("NAME" "sample.bin") NIL NIL "BASE64" {len(base64.b64encode(ATTACHMENT))} NIL ("ATTACHMENT" ("FILENAME" "sample.bin")) NIL NIL) "MIXED" ("BOUNDARY" "x") NIL NIL)'


class IMAP(socketserver.StreamRequestHandler):
    def send(self, data):
        self.wfile.write(data.encode() if isinstance(data, str) else data)
        self.wfile.flush()

    def handle(self):
        self.send("* OK synthetic IMAP fixture\r\n")
        folder = "INBOX"
        while True:
            line = self.rfile.readline(16384)
            if not line:
                return
            command = line.decode().strip()
            fields = command.split(" ", 2)
            if len(fields) < 2:
                return
            tag, verb = fields[:2]
            upper = command.upper()
            if verb == "CAPABILITY":
                self.send("* CAPABILITY IMAP4rev1 UIDPLUS MOVE IDLE ID X-GM-EXT-1\r\n")
            elif verb == "LOGIN":
                pass
            elif verb == "ID":
                self.send('* ID ("name" "synthetic-fixture")\r\n')
            elif verb == "LIST":
                self.send('* LIST (\\HasNoChildren) "/" "INBOX"\r\n* LIST (\\HasNoChildren \\Sent) "/" "Sent"\r\n* LIST (\\HasNoChildren \\Trash) "/" "Trash"\r\n')
            elif verb == "SELECT":
                folder = command.split(" ", 2)[2].strip('"')
                self.send(f'* FLAGS (\\Seen \\Flagged)\r\n* {1 if folder=="INBOX" else 0} EXISTS\r\n* 0 RECENT\r\n* OK [UIDVALIDITY {STATE["validity"]}] valid\r\n* OK [UIDNEXT 2] next\r\n')
            elif "UID SEARCH" in upper:
                self.send("* SEARCH" + (" 1" if folder == "INBOX" else "") + "\r\n")
            elif "UID FETCH" in upper:
                flags = "\\Seen" if STATE["seen"] else ""
                if "BODY.PEEK[HEADER]" in upper:
                    self.send(f'* 1 FETCH (UID 1 FLAGS ({flags}) INTERNALDATE "28-Sep-2026 10:42:00 +0800" RFC822.SIZE 1024 X-GM-MSGID 1001 BODY[HEADER] {{{len(HEADER)}}}\r\n')
                    self.send(HEADER)
                    self.send(f" BODYSTRUCTURE {STRUCTURE})\r\n")
                elif "BODYSTRUCTURE" in upper:
                    self.send(f"* 1 FETCH (UID 1 BODYSTRUCTURE {STRUCTURE})\r\n")
                elif "BODY.PEEK[1]" in upper:
                    STATE["body_fetches"] += 1
                    self.send(f"* 1 FETCH (UID 1 BODY[1] {{{len(BODY)}}}\r\n")
                    self.send(BODY)
                    self.send(")\r\n")
                elif "BODY.PEEK[2]" in upper:
                    STATE["attachment_fetches"] += 1
                    encoded = base64.b64encode(ATTACHMENT)
                    self.send(f"* 1 FETCH (UID 1 BODY[2] {{{len(encoded)}}}\r\n")
                    self.send(encoded)
                    self.send(")\r\n")
                else:
                    self.send(f"* 1 FETCH (UID 1 FLAGS ({flags}))\r\n")
            elif "UID STORE" in upper:
                STATE["seen"] = "+FLAGS" in upper
            elif "UID MOVE" in upper:
                pass
            elif verb == "IDLE":
                self.send("+ idling\r\n* 1 EXISTS\r\n")
                self.rfile.readline()
            elif verb == "APPEND":
                size = int(re.search(r"\{(\d+)\}$", command)[1])
                self.send("+ Ready\r\n")
                self.rfile.read(size)
                self.rfile.readline()
                STATE["append_count"] += 1
            elif verb == "LOGOUT":
                self.send(f"* BYE closing\r\n{tag} OK LOGOUT\r\n")
                return
            self.send(f"{tag} OK completed\r\n")


class SMTP(socketserver.StreamRequestHandler):
    def send(self, s):
        self.wfile.write(s.encode()); self.wfile.flush()

    def handle(self):
        self.send("220 localhost ESMTP synthetic-fixture\r\n")
        rcpt = ""
        while True:
            raw = self.rfile.readline(32768)
            if not raw:
                return
            line = raw.decode().strip()
            verb = line.split(" ")[0].upper()
            if verb == "EHLO":
                self.send("250-localhost\r\n250-STARTTLS\r\n250 AUTH PLAIN LOGIN\r\n")
            elif verb == "STARTTLS":
                self.send("220 Ready to start TLS\r\n")
                self.connection = self.server.context.wrap_socket(self.connection, server_side=True)
                self.rfile = self.connection.makefile("rb")
                self.wfile = self.connection.makefile("wb")
            elif verb == "AUTH":
                self.send("235 Authentication successful\r\n")
            elif verb == "RCPT":
                rcpt += line
                self.send("550 synthetic permanent rejection\r\n" if "reject@example.com" in line else "250 OK\r\n")
            elif verb == "DATA":
                self.send("354 End data with dot\r\n")
                chunks = []
                while True:
                    item = self.rfile.readline(65536)
                    if item == b".\r\n" or not item:
                        break
                    chunks.append(item)
                parsed = email.message_from_bytes(b"".join(chunks))
                STATE["bcc_header_seen"] = parsed.get("Bcc") is not None
                if "drop@example.com" in rcpt:
                    STATE["smtp_dropped"] += 1
                    return
                STATE["smtp_accepted"] += 1
                self.send("250 accepted by synthetic fixture\r\n")
            elif verb == "QUIT":
                self.send("221 bye\r\n"); return
            else:
                self.send("250 OK\r\n")


class Control(BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def do_GET(self):
        data = json.dumps(STATE).encode()
        self.send_response(200); self.send_header("Content-Length",str(len(data))); self.end_headers(); self.wfile.write(data)


class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


class TLSServer(Server):
    def get_request(self):
        sock, address = super().get_request()
        return self.context.wrap_socket(sock,server_side=True), address


if __name__ == "__main__":
    p=argparse.ArgumentParser();p.add_argument("--cert",required=True);p.add_argument("--key",required=True);p.add_argument("--ports-file",required=True);args=p.parse_args()
    context=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);context.load_cert_chain(args.cert,args.key)
    imap=TLSServer(("127.0.0.1",0),IMAP);imap.context=context
    smtp=Server(("127.0.0.1",0),SMTP);smtp.context=context
    socks=Server(("127.0.0.1",0),Proxy);socks.kind="socks5"
    http=Server(("127.0.0.1",0),Proxy);http.kind="http"
    for proxy in (socks,http):
        proxy.allowed_ports={imap.server_address[1],smtp.server_address[1]};proxy.state=STATE
    control=ThreadingHTTPServer(("127.0.0.1",0),Control)
    with open(args.ports_file,"w")as f:json.dump({"imap":imap.server_address[1],"smtp":smtp.server_address[1],"control":control.server_address[1],"socks5":socks.server_address[1],"http":http.server_address[1]},f)
    for server in [imap,smtp,socks,http]:threading.Thread(target=server.serve_forever,daemon=True).start()
    print("Loopback mail fixtures ready",flush=True)
    control.serve_forever()
