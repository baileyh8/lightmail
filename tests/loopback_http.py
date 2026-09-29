"""HTTP fixtures must not wait for a CI machine's reverse DNS configuration."""
from http.server import HTTPServer, ThreadingHTTPServer
from socketserver import TCPServer


class LoopbackName:
    def server_bind(self):
        if self.server_address[0] != "127.0.0.1":
            raise ValueError("Test servers must bind to 127.0.0.1")
        # HTTPServer.server_bind calls socket.getfqdn(), which may block on
        # external DNS even for loopback on hosted macOS runners.
        TCPServer.server_bind(self)
        self.server_name = "localhost"
        self.server_port = self.server_address[1]


class LoopbackHTTPServer(LoopbackName, HTTPServer):
    pass


class LoopbackThreadingHTTPServer(LoopbackName, ThreadingHTTPServer):
    pass


if __name__ == "__main__":
    from http.server import BaseHTTPRequestHandler
    from unittest.mock import patch

    with patch("socket.getfqdn", side_effect=AssertionError("Unexpected DNS lookup")):
        for server_type in (LoopbackHTTPServer, LoopbackThreadingHTTPServer):
            with server_type(("127.0.0.1", 0), BaseHTTPRequestHandler) as server:
                assert server.server_name == "localhost" and server.server_port > 0
            try:
                server_type(("0.0.0.0", 0), BaseHTTPRequestHandler)
            except ValueError:
                pass
            else:
                raise AssertionError("Non-loopback bind was allowed")
    print("PASS HTTP fixtures start without DNS and reject non-loopback addresses")
