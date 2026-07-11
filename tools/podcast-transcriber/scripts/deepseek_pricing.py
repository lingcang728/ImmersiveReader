from __future__ import annotations

import urllib.error
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
