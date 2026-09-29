#!/usr/bin/env python3
"""Run isolated protocol checks. No production account or API credentials used."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
APP = ROOT / "dist/轻邮.app/Contents/MacOS/Lightmail"


def run(args, **kwargs):
    subprocess.run(args, cwd=ROOT, check=True, **kwargs)


def wait_file(path, server):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if path.exists() and path.stat().st_size:
            return
        if server.poll() is not None:
            raise RuntimeError("Fixture exited before ready")
        time.sleep(0.05)
    raise RuntimeError("Fixture startup timed out")


def main():
    run([sys.executable, "tests/loopback_http.py"])
    run(["bash", "scripts/cargo.sh", "test", "--lib"])
    run([str(APP), "--self-test"])
    with tempfile.TemporaryDirectory(prefix="lightmail-render-", dir=ROOT / "build") as fixture:
        renderer = subprocess.Popen([sys.executable, "tests/render_fixture.py", fixture], cwd=ROOT)
        try:
            wait_file(Path(fixture) / "port", renderer)
            run([str(APP), "--render-test", fixture])
        finally:
            renderer.terminate(); renderer.wait(timeout=10)
    with tempfile.TemporaryDirectory(prefix="lightmail-memory-", dir=ROOT / "build") as fixture:
        run([sys.executable, "scripts/memory-watch.py", str(APP), "--memory-test", fixture,
             "--reader-stress"], env=dict(os.environ, MEMORY_TEST_DEADLINE="120"))
    with tempfile.TemporaryDirectory(prefix="lightmail-check-", dir=ROOT / "build") as temp:
        temp = Path(temp)
        cert, key = temp / "cert.pem", temp / "key.pem"
        run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", str(key),
             "-out", str(cert), "-days", "1", "-config", "tests/tls-fixture.cnf"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        chat_port = temp / "chat.port"
        chat = subprocess.Popen([sys.executable, "tests/chat_fixture.py", "--port-file", str(chat_port)], cwd=ROOT)
        try:
            wait_file(chat_port, chat)
            for kind in ("direct", "socks5", "http"):
                ports = temp / f"mail-{kind}.json"
                mail = subprocess.Popen([sys.executable, "tests/mail_fixture.py", "--cert", str(cert),
                                         "--key", str(key), "--ports-file", str(ports)], cwd=ROOT)
                try:
                    wait_file(ports, mail)
                    env = dict(os.environ, LIGHTMAIL_TEST_CA=str(cert), LIGHTMAIL_TEST_PORTS=str(ports), LIGHTMAIL_PROXY_KIND=kind)
                    run(["bash", "scripts/cargo.sh", "test", "--lib", "local_protocol_integration",
                         "--", "--ignored", "--nocapture"], env=env)
                finally:
                    mail.terminate(); mail.wait(timeout=10)
            run([str(APP), "--integration-test", f"http://127.0.0.1:{chat_port.read_text()}/v1"])
        finally:
            for process in [chat]:
                process.terminate()
                process.wait(timeout=10)
    if "--benchmark" in sys.argv:
        run(["bash", "scripts/cargo.sh", "test", "--release", "--lib", "performance_dataset",
             "--", "--ignored", "--nocapture"],
            env=dict(os.environ, LIGHTMAIL_BENCH_REPORT=str(ROOT / "build/performance.json")))


if __name__ == "__main__":
    main()
