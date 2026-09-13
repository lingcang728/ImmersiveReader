from __future__ import annotations

import io
import json
import sys
import urllib.error
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from deepseek_pricing import (  # noqa: E402
    PodcastBudgetExceededError,
    PodcastLocalError,
    PodcastUpstreamError,
    classify_upstream_error,
    deepseek_thinking_config,
    parse_retry_after,
    release_budget,
    reserve_budget,
    settle_budget,
)
from podcast_transcriber.deepseek import effective_provider_name  # noqa: E402


def test_deepseek_v4_thinking_mode_follows_config() -> None:
    assert deepseek_thinking_config({"think": False}) == {"type": "disabled"}
    assert deepseek_thinking_config({"think": "false"}) == {"type": "disabled"}
    assert deepseek_thinking_config({"think": True}) == {"type": "enabled"}


def test_deepseek_uses_managed_default_env_when_legacy_config_leaves_name_blank() -> None:
    assert effective_provider_name(
        {
            "backend": "deepseek",
            "base_url": "https://api.deepseek.com",
            "api_key_env": "",
        }
    ) == "deepseek"


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


def test_classifies_server_error_as_upstream() -> None:
    unavailable = classify_upstream_error(http_error(503, {"Retry-After": "4"}))
    assert unavailable is not None
    assert unavailable.code == "UPSTREAM_UNAVAILABLE"
    assert unavailable.retry_after_seconds == 4
    assert not isinstance(unavailable, PodcastLocalError)


def test_transport_and_local_errors_never_get_upstream_codes() -> None:
    # A URLError means no upstream HTTP response ever existed → LOCAL_*.
    timeout = classify_upstream_error(urllib.error.URLError(TimeoutError("timed out")))
    assert timeout is not None
    assert timeout.code == "LOCAL_TIMEOUT"
    assert isinstance(timeout, PodcastLocalError)
    # Still propagates through `except PodcastUpstreamError` sites.
    assert isinstance(timeout, PodcastUpstreamError)

    refused = classify_upstream_error(urllib.error.URLError(ConnectionRefusedError("nope")))
    assert refused is not None
    assert refused.code == "LOCAL_NETWORK"

    bare_timeout = classify_upstream_error(TimeoutError("read stalled"))
    assert bare_timeout is not None
    assert bare_timeout.code == "LOCAL_TIMEOUT"

    # Bare OSError (file IO / disk / local socket) must not be UPSTREAM_*.
    local_io = classify_upstream_error(OSError(28, "No space left on device"))
    assert local_io is not None
    assert local_io.code == "LOCAL_IO"

    reset = classify_upstream_error(ConnectionResetError("reset by peer"))
    assert reset is not None
    assert reset.code == "LOCAL_NETWORK"


def test_budget_error_carries_approve_budget_action() -> None:
    error = PodcastBudgetExceededError("over")
    assert error.code == "BUDGET_CONFIRMATION_REQUIRED"
    assert error.required_action == "approve_budget"


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


def test_budget_failed_call_releases_reservation_without_spend(tmp_path, monkeypatch) -> None:
    """usage=None (exception path) releases the reservation — it must not book
    the N-times worst-case reservation as spend (P2-28)."""
    ledger = tmp_path / "budget.json"
    monkeypatch.setenv("PODCAST_TRANSCRIBER_BUDGET_LIMIT_CNY", "100")
    monkeypatch.setenv("PODCAST_TRANSCRIBER_BUDGET_STATE_PATH", str(ledger))
    config = {"model": "deepseek-v4-flash", "max_tokens": 16}

    reservation = reserve_budget("short prompt", config, retry_attempts=4)
    assert reservation > 0
    state = json.loads(ledger.read_text())
    assert float(state["reserved_cny"]) > 0

    settle_budget(reservation, None, config)
    state = json.loads(ledger.read_text())
    assert float(state["reserved_cny"]) == 0.0
    assert float(state["spent_cny"]) == 0.0
    assert int(state.get("requests") or 0) == 0


def test_budget_settle_bills_real_usage_not_reservation_multiple(tmp_path, monkeypatch) -> None:
    ledger = tmp_path / "budget.json"
    monkeypatch.setenv("PODCAST_TRANSCRIBER_BUDGET_LIMIT_CNY", "100")
    monkeypatch.setenv("PODCAST_TRANSCRIBER_BUDGET_STATE_PATH", str(ledger))
    config = {"model": "deepseek-v4-flash", "max_tokens": 2048}

    reservation = reserve_budget("x" * 300, config, retry_attempts=4)
    settle_budget(reservation, {"prompt_tokens": 10, "completion_tokens": 5}, config)
    state = json.loads(ledger.read_text())
    assert float(state["reserved_cny"]) == 0.0
    spent = float(state["spent_cny"])
    assert 0 < spent < reservation  # real usage, not the 4x worst case


def test_budget_dead_pid_reservation_is_swept(tmp_path, monkeypatch) -> None:
    """A killed worker leaks pid-tagged reservations; the next run reclaims them."""
    import os

    ledger = tmp_path / "budget.json"
    monkeypatch.setenv("PODCAST_TRANSCRIBER_BUDGET_LIMIT_CNY", "0.001")
    monkeypatch.setenv("PODCAST_TRANSCRIBER_BUDGET_STATE_PATH", str(ledger))
    config = {"model": "deepseek-v4-flash", "max_tokens": 16}

    # Simulate a killed process: a reservation owned by a dead PID. PID 2**22-1
    # (4194303) cannot exist on Windows (max pid is well below 2**22) and is
    # equally nonexistent on POSIX. The amount nearly saturates the limit, so
    # without the sweep the new reserve_budget below would be refused.
    dead_pid = 4_194_303
    assert dead_pid != os.getpid()
    ledger.write_text(
        json.dumps(
            {
                "spent_cny": 0.0,
                "reserved_cny": 0.00099,
                "requests": 0,
                "reservations": {"dead-1": {"pid": dead_pid, "amount": 0.00099}},
            }
        ),
        encoding="utf-8",
    )

    reservation = reserve_budget("short prompt", config, retry_attempts=1)
    assert reservation > 0  # sweep reclaimed the dead reservation → room exists
    state = json.loads(ledger.read_text())
    live = state["reservations"]
    assert "dead-1" not in live


def test_budget_spend_is_scoped_per_task_run(tmp_path, monkeypatch) -> None:
    """The approved limit is per task: spend booked under run A must not eat
    run B's approval, while the same run's spend stays cumulative."""
    ledger = tmp_path / "budget.json"
    monkeypatch.setenv("PODCAST_TRANSCRIBER_BUDGET_LIMIT_CNY", "0.00005")
    monkeypatch.setenv("PODCAST_TRANSCRIBER_BUDGET_STATE_PATH", str(ledger))
    config = {"model": "deepseek-v4-flash", "max_tokens": 16}

    # Run A spends against its budget.
    monkeypatch.setenv("PODCAST_TRANSCRIBER_RUN_ID", "task-a")
    reservation = reserve_budget("short prompt", config, retry_attempts=1)
    settle_budget(reservation, {"prompt_tokens": 1, "completion_tokens": 1}, config)

    # Run B — a different task with its own approval — still has headroom.
    monkeypatch.setenv("PODCAST_TRANSCRIBER_RUN_ID", "task-b")
    assert reserve_budget("short prompt", config, retry_attempts=1) > 0

    # Same task, new attempt: run A's spend is cumulative within the bucket.
    monkeypatch.setenv("PODCAST_TRANSCRIBER_RUN_ID", "task-a")
    try:
        reserve_budget("short prompt", config, retry_attempts=4)
    except PodcastBudgetExceededError as error:
        assert error.code == "BUDGET_CONFIRMATION_REQUIRED"
    else:
        raise AssertionError("run A must not regain budget already spent")

    state = json.loads(ledger.read_text())
    assert state["spent_by_run"]["task-a"] > 0
    assert state["spent_cny"] > 0  # global total retained for diagnostics


def test_release_budget_returns_reservation(tmp_path, monkeypatch) -> None:
    ledger = tmp_path / "budget.json"
    monkeypatch.setenv("PODCAST_TRANSCRIBER_BUDGET_LIMIT_CNY", "100")
    monkeypatch.setenv("PODCAST_TRANSCRIBER_BUDGET_STATE_PATH", str(ledger))
    config = {"model": "deepseek-v4-flash", "max_tokens": 16}

    reservation = reserve_budget("short prompt", config, retry_attempts=1)
    release_budget(reservation)
    state = json.loads(ledger.read_text())
    assert float(state["reserved_cny"]) == 0.0
    assert float(state["spent_cny"]) == 0.0
