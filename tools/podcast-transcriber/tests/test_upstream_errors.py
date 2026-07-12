from __future__ import annotations

import io
import socket
import sys
import urllib.error
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from deepseek_pricing import classify_upstream_error, parse_retry_after  # noqa: E402


def http_error(status: int, headers: dict[str, str] | None = None) -> urllib.error.HTTPError:
    return urllib.error.HTTPError(
        "https://example.invalid/chat/completions",
        status,
        "upstream",
        headers or {},
        io.BytesIO(b'{"error":"fixture"}'),
    )


def test_classifies_unauthorized_and_rate_limit_with_retry_after() -> None:
    unauthorized = classify_upstream_error(http_error(401))
    assert unauthorized is not None
    assert unauthorized.code == "UPSTREAM_UNAUTHORIZED"
    assert unauthorized.retry_after_seconds is None

    limited = classify_upstream_error(http_error(429, {"Retry-After": "7"}))
    assert limited is not None
    assert limited.code == "RATE_LIMITED"
    assert limited.retry_after_seconds == 7


def test_classifies_server_error_and_timeout() -> None:
    unavailable = classify_upstream_error(http_error(503, {"Retry-After": "4"}))
    assert unavailable is not None
    assert unavailable.code == "UPSTREAM_UNAVAILABLE"
    assert unavailable.retry_after_seconds == 4

    timeout = classify_upstream_error(urllib.error.URLError(socket.timeout("timed out")))
    assert timeout is not None
    assert timeout.code == "UPSTREAM_TIMEOUT"


def test_parse_retry_after_accepts_seconds_and_invalid_values() -> None:
    assert parse_retry_after("2.1") == 3
    assert parse_retry_after("not-a-delay") is None
