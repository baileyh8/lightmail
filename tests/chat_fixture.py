#!/usr/bin/env python3
"""Loopback-only Chat Completions fixture. Contains no credentials or user mail."""
import argparse
import json
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler

TRANSLATIONS = {
    "Q4 launch · final review": "第4季度发布 · 最终审阅",
    "Hi Bailey,": "Bailey，你好：",
    "The updated launch plan is ready for a final review.": "更新后的发布计划已经准备好，可以进行最终审阅。",
    "We have simplified the onboarding flow and brought all account settings into one place. The latest draft is attached below.": "我们简化了首次使用流程，并将所有账号设置整合到了同一个位置。最新草稿见下方附件。",
    "Before Friday, could you take a look at:": "方便在周五之前帮忙看看以下内容吗？",
    "- The first-time setup experience\n- The wording in the account switcher\n- The final launch checklist": "- 首次设置体验\n- 账号切换器中的文案\n- 最终发布检查清单",
    "A few focused comments would be perfect. We can finalize the details together tomorrow.": "给出几条有针对性的建议就很好。我们可以明天一起敲定细节。",
    "Thanks,\nEmma": "谢谢，\nEmma",
}


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_args):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))))
        model = request.get("model", "")
        if model == "fixture-denied":
            self.respond(401, {"error": {"message": "synthetic rejection"}})
            return
        if model == "fixture-redirect":
            self.send_response(307)
            self.send_header("Location", f"http://localhost:{self.server.server_port}/trap")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        user = request["messages"][-1]["content"]
        try:
            source = json.loads(user)
            output = {"subject": TRANSLATIONS.get(source["subject"], source["subject"]), "blocks": [
                {"id": b["id"], "text": TRANSLATIONS.get(b["text"], b["text"])} for b in source["blocks"]
            ]}
        except (ValueError, KeyError):
            output = {"subject": "测试", "blocks": [{"id": 0, "text": "连接成功"}]}
        if model == "fixture-omitted":
            output["blocks"] = []
        content = json.dumps(output, ensure_ascii=False)
        finish = "length" if model == "fixture-truncated" else "stop"
        if request.get("stream"):
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream; charset=utf-8")
            self.send_header("Connection", "close")
            self.end_headers()
            try:
                for offset in range(0, len(content), 23):
                    event = {"choices": [{"index": 0, "delta": {"content": content[offset:offset+23]}, "finish_reason": None}]}
                    self.wfile.write(("data: " + json.dumps(event, ensure_ascii=False) + "\n\n").encode())
                if model != "fixture-dropped":
                    self.wfile.write(("data: " + json.dumps({"choices": [{"delta": {}, "finish_reason": finish}]}) + "\n\n").encode())
                    self.wfile.write(b"data: [DONE]\n\n")
                self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError):
                pass
            self.close_connection = True
        else:
            self.respond(200, {"choices": [{"index": 0, "message": {"role": "assistant", "content": content}, "finish_reason": finish}], "usage": {"prompt_tokens": 123, "completion_tokens": 100}})

    def respond(self, code, data):
        data = json.dumps(data).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


if __name__ == "__main__":
    p = argparse.ArgumentParser()
    p.add_argument("--port", type=int, default=0)
    p.add_argument("--port-file", required=True)
    args = p.parse_args()
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    with open(args.port_file, "w") as f:
        f.write(str(server.server_port))
    print(f"Synthetic Chat fixture ready on 127.0.0.1:{server.server_port}", flush=True)
    server.serve_forever()
