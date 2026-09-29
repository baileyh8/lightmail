"""Loopback-only proxy fixtures; never forward to external destinations."""
import select
import socket
import socketserver
import struct


def exact(sock, size):
    data = b""
    while len(data) < size:
        chunk = sock.recv(size - len(data))
        if not chunk:
            raise ConnectionError("fixture peer closed")
        data += chunk
    return data


class Proxy(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.settimeout(10)
        if self.server.kind == "socks5":
            version, length = exact(self.request, 2)
            assert version == 5 and 0 in exact(self.request, length)
            self.request.sendall(b"\x05\x00")
            assert exact(self.request, 4) == b"\x05\x01\x00\x03"
            host = exact(self.request, exact(self.request, 1)[0]).decode("ascii")
            port = struct.unpack("!H", exact(self.request, 2))[0]
        else:
            data = b""
            while not data.endswith(b"\r\n\r\n") and len(data) < 16384:
                data += exact(self.request, 1)
            method, target, protocol = data.split(b"\r\n", 1)[0].decode().split()
            assert method == "CONNECT" and protocol == "HTTP/1.1"
            host, port = target.rsplit(":", 1)
            port = int(port)
        assert host in ("localhost", "127.0.0.1") and port in self.server.allowed_ports
        with socket.create_connection(("127.0.0.1", port), timeout=10) as upstream:
            self.server.state["proxy_connects"] += 1
            if self.server.kind == "socks5":
                self.request.sendall(b"\x05\x00\x00\x01\x7f\x00\x00\x01\x00\x00")
            else:
                self.request.sendall(b"HTTP/1.1 200 Connection established\r\n\r\n")
            while True:
                readable, _, _ = select.select([self.request, upstream], [], [], 30)
                if not readable:
                    return
                for source in readable:
                    data = source.recv(65536)
                    if not data:
                        return
                    (upstream if source is self.request else self.request).sendall(data)
