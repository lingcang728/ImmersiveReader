from __future__ import annotations

import io
import json
import socket
import sys
import urllib.error
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from deepseek_pricing import (  # noqa: E402
    PodcastBudgetExceededError,
    classify_upstream_error,
    deepseek_thinking_config,
    parse_retry_after,
    reserve_budget,
    settle_budget,
)


def test_deepseek_v4_thinking_mode_follows_config() -> None:
    assert deepseek_thinking_config({"think": False}) == {"type": "disabled"}
    assert deepseek_thinking_config({"think": "false"}) == {"type": "disabled"}
    assert deepseek_thinking_config({"think": True}) == {"type": "enabled"}


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


def test_budget_reservation_is_persistent_and_retries_consume_headroom(tmp_path, monkeypatch) -> None:
    ledger = tmp_path / "budget.json"
    monkeypatch.setenv("PODCAST_TRANSCRIBER_BUDGET_LIMIT_CNY", "0.00005")
    monkeypatch.setenv("PODCAST_TRANSCRIBER_BUDGET_STATE_PATH", str(ledger))
    config = {"model": "deepseek-v4-flash", "max_tokens": 16}

    reservation = reserve_budget("short prompt", config, retry_attempts=1)
    assert reservation > 0
    settle_budget(reservation, {"prompt_tokens": 1, "completion_tokens": 1}, config)
    assert ledger.is_file()
    assert float(json.loads(ledger.read_text())["spent_cny"]) > 0

    try:
        reserve_budget("short prompt", config, retry_attempts=4)
    except PodcastBudgetExceededError as error:
        assert error.code == "BUDGET_CONFIRMATION_REQUIRED"
    else:
        raise AssertionError("retry reservation must not bypass the cumulative budget")
