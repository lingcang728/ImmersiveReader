from __future__ import annotations

import io
import json
import sys
import urllib.error
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from deepseek_pricing import (  # noqa: E402
    PROPAGATE_FATAL_ERRORS,
    PodcastBudgetExceededError,
    PodcastLocalError,
    PodcastSecretMissingError,
    PodcastUpstreamError,
    classify_upstream_error,
    deepseek_thinking_config,
    parse_retry_after,
    release_budget,
    reserve_budget,
    settle_budget,
)
from podcast_transcriber.deepseek import (  # noqa: E402
    deepseek_chat_completion,
    effective_provider_name,
)


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


def test_secret_missing_error_carries_configure_secret_action() -> None:
    """SECRET_MISSING must reach the host fatal line so the UI offers the
    key-input flow instead of a retry that always fails."""
    error = PodcastSecretMissingError("no key")
    assert error.code == "SECRET_MISSING"
    assert error.required_action == "configure_secret"
    assert isinstance(error, PROPAGATE_FATAL_ERRORS)


def test_deepseek_call_and_provider_fail_fast_on_missing_key(monkeypatch) -> None:
    """Missing key fails before any HTTP attempt with the typed secret error —
    both at the provider gate and at the chat-completion entry."""
    monkeypatch.delenv("DEEPSEEK_API_KEY", raising=False)
    bare = {"backend": "deepseek", "base_url": "https://api.deepseek.com", "api_key": "", "api_key_env": ""}
    try:
        deepseek_chat_completion("hello", bare)
    except PodcastSecretMissingError as error:
        assert error.code == "SECRET_MISSING"
    else:
        raise AssertionError("missing key must raise PodcastSecretMissingError, not RuntimeError")

    no_entry = {"backend": "deepseek", "base_url": "", "api_key": "", "api_key_env": ""}
    try:
        effective_provider_name(no_entry)
    except PodcastSecretMissingError as error:
        assert error.required_action == "configure_secret"
    else:
        raise AssertionError("missing API entry must raise PodcastSecretMissingError")


def test_unauthorized_upstream_suggests_configure_secret() -> None:
    unauthorized = classify_upstream_error(http_error(401))
    assert unauthorized is not None
    assert unauthorized.code == "UPSTREAM_UNAUTHORIZED"
    assert unauthorized.required_action == "configure_secret"


def test_preflight_fails_fast_when_deepseek_polish_key_missing(monkeypatch) -> None:
    """polish=deepseek + no key is guaranteed to fail — preflight must raise
    SECRET_MISSING before the ASR stage burns GPU time."""
    import logging

    import transcribe_podcasts as tp

    monkeypatch.delenv("DEEPSEEK_API_KEY", raising=False)
    monkeypatch.delenv("PODCAST_TRANSCRIBER_FORCE_POLISH", raising=False)
    monkeypatch.delenv("PODCAST_TRANSCRIBER_FORCE_TRANSLATE", raising=False)
    config = {
        "translation": {"enabled": False, "backend": "none"},
        "markdown": {
            "llm_polish": {
                "enabled": True,
                "backend": "deepseek",
                "base_url": "https://api.deepseek.com",
                "api_key": "",
                "api_key_env": "",
            }
        },
    }
    try:
        tp.preflight_checks(config, logging.getLogger("test"))
    except PodcastSecretMissingError as error:
        assert error.code == "SECRET_MISSING"
    else:
        raise AssertionError("missing polish key must fail preflight")


def test_preflight_passes_without_deepseek_backends(monkeypatch) -> None:
    """Ollama/disabled polish must not trip the DeepSeek credential gate."""
    import logging

    import transcribe_podcasts as tp

    monkeypatch.delenv("DEEPSEEK_API_KEY", raising=False)
    monkeypatch.delenv("PODCAST_TRANSCRIBER_FORCE_POLISH", raising=False)
    config = {
        # auto_start_ollama=False keeps the Ollama probe from spawning a real
        # `ollama serve` process inside the test runner.
        "translation": {"enabled": False, "backend": "none", "auto_start_ollama": False},
        "markdown": {"llm_polish": {"enabled": True, "backend": "ollama"}},
    }
    assert tp.preflight_checks(config, logging.getLogger("test")) is True


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
