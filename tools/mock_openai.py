#!/usr/bin/env python3
"""Local stand-in for any OpenAI-compatible chat endpoint (M5 end-to-end checks).

The app is provider-agnostic, so this server implements only the shape the app
sends and reads:

    POST <base>/chat/completions
    {"model": "...", "max_tokens": N, "messages": [{"role": "system", ...}, ...]}
    -> {"model": "...", "choices": [{"message": {"role": "assistant", "content": ...}}],
        "usage": {"prompt_tokens": N, "completion_tokens": M}}

Behaviour is chosen per request from a path segment, so one process serves all
five upstream behaviours the acceptance list asks for:

    http://127.0.0.1:8777/v1/chat/completions          -> normal
    http://127.0.0.1:8777/401/v1/chat/completions      -> 401
    http://127.0.0.1:8777/500/v1/chat/completions      -> 500
    http://127.0.0.1:8777/timeout/v1/chat/completions  -> stall past the client timeout
    http://127.0.0.1:8777/nonjson/v1/chat/completions  -> 200 with a non-JSON body

Point ZGC_AI_BASE at the prefix, e.g. http://127.0.0.1:8777/401/v1.

The 401/500 bodies deliberately echo the received bearer token in the message
text: that is what a real provider does ("Incorrect API key provided: sk-..."),
and it is how the app's redaction is proven rather than asserted.

Usage: python3 tools/mock_openai.py [--port 8777] [--dump /tmp/zgc_mock_last.json]
"""

import argparse
import json
import socket
import socketserver
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse

MODES = ("normal", "401", "500", "timeout", "nonjson")
DEFAULT_ANSWER = (
    "此段言衞鞅自魏入秦，孝公以之爲相。\n\n"
    "【原文所載】「衞鞅亡魏入秦，孝公以爲相。」\n"
    "【後世推斷】其相秦之年，諸家說法不一，此處史料不足，不作定論。\n\n"
    "（mock 回答，供端到端驗證）"
)


def mode_of(path, fallback):
    for segment in path.split("/"):
        if segment in MODES:
            return segment
    return fallback


class Server(ThreadingHTTPServer):
    daemon_threads = True

    def server_bind(self):
        # HTTPServer.server_bind() calls socket.getfqdn(), a reverse-DNS lookup
        # that can stall for a long time on a machine with no working resolver.
        # Nothing here uses the fqdn, so bind like the plain TCP server does.
        socketserver.TCPServer.server_bind(self)
        self.server_name, self.server_port = self.server_address[:2]


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):  # keep the console readable
        sys.stderr.write("[mock] " + fmt % args + "\n")

    def _read_body(self):
        length = int(self.headers.get("content-length") or 0)
        raw = self.rfile.read(length) if length else b""
        try:
            return json.loads(raw.decode("utf-8"))
        except Exception:
            return {}

    def _send(self, status, payload, content_type="application/json"):
        body = payload.encode("utf-8")
        self.send_response(status)
        self.send_header("content-type", content_type)
        self.send_header("content-length", str(len(body)))
        self.send_header("connection", "close")
        self.end_headers()
        self.wfile.write(body)
        self.wfile.flush()

    def _error(self, status, message):
        return self._send(status, json.dumps({"error": {"message": message}}, ensure_ascii=False))

    def do_POST(self):
        path = urlparse(self.path).path
        mode = mode_of(path, self.server.mode)
        request = self._read_body()
        messages = request.get("messages") or []
        system = next((m.get("content", "") for m in messages if m.get("role") == "system"), "")
        user = next((m.get("content", "") for m in messages if m.get("role") == "user"), "")
        model = request.get("model") or "mock-model"
        auth = self.headers.get("authorization") or ""
        token = auth[len("Bearer "):] if auth.startswith("Bearer ") else ""

        # The token itself is never logged: only whether one arrived.
        print(
            f"[mock] {mode} {path} model={model} system={len(system)}c user={len(user)}c "
            f"authorization={'set' if token else 'unset'}",
            file=sys.stderr,
            flush=True,
        )

        if self.server.dump:
            with open(self.server.dump, "w", encoding="utf-8") as handle:
                json.dump(
                    {
                        "mode": mode,
                        "path": path,
                        "model": model,
                        "system": system,
                        "user": user,
                        "authorization_present": bool(token),
                    },
                    handle,
                    ensure_ascii=False,
                    indent=2,
                )

        if mode == "401":
            return self._error(401, f"Incorrect API key provided: {token or 'none'}. (mock)")
        if mode == "500":
            return self._error(500, "The server had an error while processing your request. (mock)")
        if mode == "nonjson":
            return self._send(200, "<html><body>gateway says hi</body></html>", "text/html")
        if mode == "timeout":
            # Outlive the client's ZGC_AI_TIMEOUT_SECS, then answer into a socket
            # the client has already abandoned.
            time.sleep(self.server.delay)
            return self._send(200, self._ok(model, DEFAULT_ANSWER))
        return self._send(200, self._ok(model, DEFAULT_ANSWER))

    def _ok(self, model, answer):
        prompt_chars = 0
        return json.dumps(
            {
                "id": "chatcmpl-mock",
                "object": "chat.completion",
                "model": model,
                "choices": [
                    {
                        "index": 0,
                        "message": {"role": "assistant", "content": answer},
                        "finish_reason": "stop",
                    }
                ],
                "usage": {"prompt_tokens": 428, "completion_tokens": 96, "total_tokens": 524},
            },
            ensure_ascii=False,
        )

    def do_GET(self):
        self._send(200, json.dumps({"status": "ok", "modes": list(MODES)}))


def main():
    parser = argparse.ArgumentParser(description="OpenAI-compatible mock endpoint")
    parser.add_argument("--port", type=int, default=8777)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--mode", default="normal", choices=MODES)
    parser.add_argument("--delay", type=float, default=8.0, help="seconds the timeout mode stalls")
    parser.add_argument("--dump", default="/tmp/zgc_mock_last.json", help="write the last request here")
    args = parser.parse_args()

    server = Server((args.host, args.port), Handler)
    server.mode = args.mode
    server.delay = args.delay
    server.dump = args.dump
    print(
        f"[mock] listening on http://{args.host}:{args.port} "
        f"(default mode {args.mode}; prefix a path with one of {', '.join(MODES)})",
        file=sys.stderr,
        flush=True,
    )
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
