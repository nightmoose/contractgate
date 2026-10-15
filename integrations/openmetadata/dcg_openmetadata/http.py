"""Tiny JSON-over-HTTP helper built on urllib (no third-party deps)."""

from __future__ import annotations

import json
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Optional


class HttpError(Exception):
    def __init__(self, method: str, url: str, status: int, body: str):
        super().__init__(f"{method} {url} -> HTTP {status}: {body[:500]}")
        self.method = method
        self.url = url
        self.status = status
        self.body = body


def request(
    method: str,
    url: str,
    headers: Optional[dict] = None,
    body: Any = None,
    raw: bool = False,
    timeout: float = 30.0,
) -> Any:
    """Send a request. Returns parsed JSON (or text when raw=True / non-JSON)."""
    hdrs = {"Accept": "application/json"}
    hdrs.update(headers or {})
    data = None
    if body is not None:
        if isinstance(body, (bytes, str)):
            data = body.encode() if isinstance(body, str) else body
        else:
            data = json.dumps(body).encode()
            hdrs.setdefault("Content-Type", "application/json")
    req = urllib.request.Request(url, data=data, headers=hdrs, method=method)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            text = resp.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        raise HttpError(method, url, e.code, e.read().decode("utf-8", "replace")) from None
    if raw or not text:
        return text
    try:
        return json.loads(text)
    except ValueError:
        return text


def quote(segment: str) -> str:
    """Percent-encode a single path segment (FQNs contain dots and quotes)."""
    return urllib.parse.quote(segment, safe="")
