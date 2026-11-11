#!/usr/bin/env python3
"""Tiny local 'ask the docs' HTTP endpoint. Not deployed; no hourly cost.

  python scripts/serve.py          # http://127.0.0.1:8787/ask
  curl -s localhost:8787/ask -H 'content-type: application/json' \
    -d '{"question":"What is make demo?","kb":"fixed","model":"lite"}'
"""
from __future__ import annotations

import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlparse

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import ask as ask_mod  # noqa: E402
from tf_outputs import load as load_outputs  # noqa: E402

import boto3

REGION = "us-east-1"
HOST = "127.0.0.1"
PORT = 8787


class Handler(BaseHTTPRequestHandler):
    runtime = None
    outputs = None

    def log_message(self, fmt, *args):
        sys.stderr.write("%s - %s\n" % (self.address_string(), fmt % args))

    def _send(self, code: int, body: dict):
        raw = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_GET(self):
        path = urlparse(self.path).path
        if path in ("/", "/health"):
            self._send(200, {"ok": True, "kbs": list(self.outputs["knowledge_bases"])})
            return
        self._send(404, {"error": "not found"})

    def do_POST(self):
        path = urlparse(self.path).path
        if path != "/ask":
            self._send(404, {"error": "not found"})
            return
        length = int(self.headers.get("content-length") or 0)
        try:
            data = json.loads(self.rfile.read(length) or b"{}")
        except json.JSONDecodeError:
            self._send(400, {"error": "invalid json"})
            return
        question = (data.get("question") or "").strip()
        if not question:
            self._send(400, {"error": "question is required"})
            return
        kb = data.get("kb") or "fixed"
        model = data.get("model") or "lite"
        k = int(data.get("k") or 5)
        if kb not in self.outputs["knowledge_bases"] or model not in ask_mod.MODELS:
            self._send(400, {"error": "kb must be fixed|semantic and model lite|pro"})
            return
        kb_id = self.outputs["knowledge_bases"][kb]["knowledge_base_id"]
        gen = ask_mod.generate(self.runtime, kb_id, question, ask_mod.MODELS[model], k=k)
        self._send(
            200,
            {
                "question": question,
                "kb": kb,
                "model": ask_mod.MODELS[model],
                "answer": (gen.get("output") or {}).get("text", ""),
                "latency_s": round(gen["_latency_s"], 3),
            },
        )


def main() -> int:
    Handler.outputs = load_outputs()
    Handler.runtime = boto3.client("bedrock-agent-runtime", region_name=REGION)
    server = ThreadingHTTPServer((HOST, PORT), Handler)
    print(f"ask-the-docs listening on http://{HOST}:{PORT}/ask", flush=True)
    server.serve_forever()
    return 0


if __name__ == "__main__":
    sys.exit(main())
