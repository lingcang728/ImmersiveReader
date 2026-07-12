from __future__ import annotations

import urllib.error
import socket
from datetime import datetime, timezone
from email.utils import parsedate_to_datetime
import math
from typing import Any

DEEPSEEK_DEFAULT_BASE_URL = "https://api.deepseek.com"
DEEPSEEK_CHAT_COMPLETIONS_PATH = "/chat/completions"
DEEPSEEK_DEFAULT_MODEL = "deepseek-v4-flash"
DEEPSEEK_MODEL_PRICING_PER_MILLION: dict[str, dict[str, float]] = {
    "deepseek-v4-flash": {"input": 0.14, "cache_hit_input": 0.0028, "output": 0.28},
    "deepseek-v4-pro": {"input": 1.74, "cache_hit_input": 0.0174, "output": 3.48},
}
DEEPSEEK_MODELS: list[dict[str, Any]] = [
    {
        "id": model,
        "label": "DeepSeek V4 Flash" if model == "deepseek-v4-flash" else "DeepSeek V4 Pro",
        "pricing": pricing,
    }
    for model, pricing in DEEPSEEK_MODEL_PRICING_PER_MILLION.items()
]

RETRYABLE_HTTP_STATUS_CODES = {429, 500, 502, 503, 504}


class PromptBudgetError(RuntimeError):
    """Raised when a prompt exceeds the estimated token budget."""


class PodcastUpstreamError(RuntimeError):
    def __init__(
        self,
        code: str,
        message: str,
        retry_after_seconds: int | None = None,
        status: int | None = None,
    ) -> None:
        super().__init__(message)
        self.code = code
        self.retry_after_seconds = retry_after_seconds
        self.status = status


def parse_retry_after(value: Any, now: datetime | None = None) -> int | None:
    if value is None:
        return None
    raw = str(value).strip()
    if not raw:
        return None
    try:
        return max(0, int(math.ceil(float(raw))))
    except (TypeError, ValueError):
        pass
    try:
        target = parsedate_to_datetime(raw)
        if target.tzinfo is None:
            target = target.replace(tzinfo=timezone.utc)
        current = now or datetime.now(timezone.utc)
        return max(0, int(math.ceil((target - current).total_seconds())))
    except (TypeError, ValueError, OverflowError):
        return None


def classify_upstream_error(exc: BaseException, service: str = "Upstream") -> PodcastUpstreamError | None:
    if isinstance(exc, urllib.error.HTTPError):
        status = int(exc.code)
        retry_after = parse_retry_after(exc.headers.get("Retry-After") if getattr(exc, "headers", None) else None)
        if status == 401:
            code = "UPSTREAM_UNAUTHORIZED"
        elif status == 429:
            code = "RATE_LIMITED"
        elif status >= 500:
            code = "UPSTREAM_UNAVAILABLE"
        else:
            return None
        detail = ""
        try:
            detail = exc.read().decode("utf-8", errors="replace").strip()[:500]
        except (OSError, UnicodeError):
            detail = ""
        message = f"{service} HTTP {status}"
        if detail:
            message = f"{message}: {detail}"
        return PodcastUpstreamError(code, message, retry_after, status)
    if isinstance(exc, urllib.error.URLError):
        reason = getattr(exc, "reason", None)
        if isinstance(reason, (TimeoutError, socket.timeout)):
            return PodcastUpstreamError("UPSTREAM_TIMEOUT", f"{service} request timed out: {reason}")
        return PodcastUpstreamError("UPSTREAM_UNAVAILABLE", f"{service} network error: {reason}")
    if isinstance(exc, (TimeoutError, socket.timeout)):
        return PodcastUpstreamError("UPSTREAM_TIMEOUT", f"{service} request timed out: {exc}")
    if isinstance(exc, (ConnectionResetError, ConnectionRefusedError, OSError)):
        return PodcastUpstreamError("UPSTREAM_UNAVAILABLE", f"{service} network error: {exc}")
    return None


def is_retryable_http_error(exc: BaseException) -> bool:
    """Return True if the exception represents a transient error worth retrying."""
    if isinstance(exc, urllib.error.HTTPError):
        return exc.code in RETRYABLE_HTTP_STATUS_CODES
    if isinstance(exc, urllib.error.URLError):
        reason = getattr(exc, "reason", None)
        if isinstance(reason, (TimeoutError, ConnectionResetError, ConnectionRefusedError, OSError)):
            return True
        if reason is None:
            return True
    if isinstance(exc, (TimeoutError, ConnectionResetError, ConnectionRefusedError)):
        return True
    return False


def deepseek_chat_completions_url(base_url: Any) -> str:
    """Return the concrete ChatCompletions endpoint for DeepSeek."""
    raw = str(base_url or DEEPSEEK_DEFAULT_BASE_URL).strip().rstrip("/")
    if raw.endswith(DEEPSEEK_CHAT_COMPLETIONS_PATH):
        return raw
    return f"{raw}{DEEPSEEK_CHAT_COMPLETIONS_PATH}"


def normalize_deepseek_model(model: Any) -> str:
    value = str(model or "").strip()
    if value in DEEPSEEK_MODEL_PRICING_PER_MILLION:
        return value
    return DEEPSEEK_DEFAULT_MODEL
