"""DeepSeek API client: adjustable concurrency limiter, retries, cost tracking."""
from __future__ import annotations

import hashlib
import json
import logging
import os
import threading
import time
import urllib.error
import urllib.request
from typing import Any

from deepseek_pricing import (
    DEEPSEEK_DEFAULT_MODEL,
    DEEPSEEK_MODEL_PRICING_PER_MILLION,
    PromptBudgetError,
    classify_upstream_error,
    deepseek_chat_completions_url,
    deepseek_thinking_config,
    is_retryable_http_error,
    normalize_deepseek_model,
    reserve_budget,
    settle_budget,
)


DEEPSEEK_PROMPT_TOKEN_LIMIT = 200_000


DEEPSEEK_PROMPT_TOKEN_HARD_LIMIT = 220_000


DEFAULT_OLLAMA_TRANSLATION_BATCH_SEGMENTS = 4


DEFAULT_DEEPSEEK_TRANSLATION_BATCH_SEGMENTS = 48


DEFAULT_OLLAMA_TRANSLATION_BATCH_CHARS = 3_000


DEFAULT_DEEPSEEK_TRANSLATION_BATCH_CHARS = 18_000


class DeepSeekApiLimiter:
    """Small adjustable process-local limiter shared by translation and polish."""

    def __init__(self, limit: int) -> None:
        self._limit = max(1, min(8, int(limit)))
        self._active = 0
        self._condition = threading.Condition()

    def set_limit(self, limit: int) -> None:
        with self._condition:
            self._limit = max(1, min(8, int(limit)))
            self._condition.notify_all()

    def acquire(self) -> None:
        with self._condition:
            while self._active >= self._limit:
                self._condition.wait()
            self._active += 1

    def release(self) -> None:
        with self._condition:
            self._active = max(0, self._active - 1)
            self._condition.notify_all()


DEEPSEEK_API_SEMAPHORE: DeepSeekApiLimiter | None = None


DEEPSEEK_API_SEMAPHORE_LOCK = threading.Lock()


DEEPSEEK_API_CONFIG_LIMIT: int | None = None


def deepseek_api_semaphore(config: dict[str, Any]) -> DeepSeekApiLimiter:
    global DEEPSEEK_API_CONFIG_LIMIT, DEEPSEEK_API_SEMAPHORE
    with DEEPSEEK_API_SEMAPHORE_LOCK:
        requested_limit: int | None = None
        pipeline_cfg = config.get("pipeline") if isinstance(config.get("pipeline"), dict) else None
        if pipeline_cfg is not None:
            requested_limit = pipeline_cfg.get("max_deepseek_api_requests", 4)
        elif "max_deepseek_api_requests" in config:
            requested_limit = config.get("max_deepseek_api_requests", 4)
        if requested_limit is not None:
            try:
                requested_limit = max(1, min(8, int(requested_limit)))
            except (TypeError, ValueError):
                requested_limit = 4
        if DEEPSEEK_API_SEMAPHORE is None:
            DEEPSEEK_API_SEMAPHORE = DeepSeekApiLimiter(requested_limit or 4)
            DEEPSEEK_API_CONFIG_LIMIT = requested_limit or 4
        elif requested_limit is not None:
            if requested_limit != DEEPSEEK_API_CONFIG_LIMIT:
                DEEPSEEK_API_SEMAPHORE.set_limit(requested_limit)
                DEEPSEEK_API_CONFIG_LIMIT = requested_limit
        return DEEPSEEK_API_SEMAPHORE


def estimate_text_tokens(text: str) -> int:
    if not text:
        return 0
    return max(1, (len(text.encode("utf-8")) + 2) // 3)


def deepseek_prompt_limit(config: dict[str, Any], hard: bool = False) -> int:
    key = "hard_prompt_token_limit" if hard else "prompt_token_limit"
    default = DEEPSEEK_PROMPT_TOKEN_HARD_LIMIT if hard else DEEPSEEK_PROMPT_TOKEN_LIMIT
    try:
        return max(1_000, int(config.get(key, default)))
    except (TypeError, ValueError):
        return default


def assert_deepseek_prompt_budget(prompt: str, config: dict[str, Any]) -> None:
    estimated = estimate_text_tokens(prompt)
    limit = deepseek_prompt_limit(config, hard=True)
    if estimated > limit:
        raise PromptBudgetError(f"DeepSeek prompt estimated at {estimated} tokens, above safety limit {limit}.")


def text_hash(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8", errors="replace")).hexdigest()[:16]


def provider_name(config: dict[str, Any]) -> str:
    return str(config.get("backend", config.get("provider", "ollama"))).strip().lower()


def has_api_entry(config: dict[str, Any]) -> bool:
    if not str(config.get("base_url") or "").strip():
        return False
    env_name = str(config.get("api_key_env") or "").strip()
    return bool(env_name or str(config.get("api_key") or "").strip())


def effective_provider_name(config: dict[str, Any]) -> str:
    provider = provider_name(config)
    if provider == "deepseek" and not has_api_entry(config):
        raise RuntimeError("DeepSeek API backend is selected, but no base_url/API key entry is configured.")
    return provider


def positive_int(value: Any, default: int, minimum: int = 1) -> int:
    try:
        return max(minimum, int(value))
    except (TypeError, ValueError):
        return default


def translation_batch_segments(config: dict[str, Any]) -> int:
    default = (
        DEFAULT_DEEPSEEK_TRANSLATION_BATCH_SEGMENTS
        if provider_name(config) == "deepseek"
        else DEFAULT_OLLAMA_TRANSLATION_BATCH_SEGMENTS
    )
    return positive_int(config.get("batch_segments"), default)


def translation_batch_char_limit(config: dict[str, Any]) -> int:
    default = (
        DEFAULT_DEEPSEEK_TRANSLATION_BATCH_CHARS
        if provider_name(config) == "deepseek"
        else DEFAULT_OLLAMA_TRANSLATION_BATCH_CHARS
    )
    return positive_int(config.get("max_batch_chars"), default, minimum=500)


def model_label(config: dict[str, Any], fallback: str = "qwen3.5:9b") -> str:
    provider = effective_provider_name(config)
    model = normalize_deepseek_model(config.get("model")) if provider == "deepseek" else str(config.get("model") or fallback)
    return f"{provider}/{model}"


def resolve_api_key(config: dict[str, Any], default_env: str) -> str:
    env_name = str(config.get("api_key_env") or default_env).strip()
    if env_name and os.environ.get(env_name):
        return str(os.environ[env_name]).strip()
    return str(config.get("api_key") or "").strip()


def estimate_deepseek_cost(usage: dict[str, Any], config: dict[str, Any]) -> float:
    model = normalize_deepseek_model(config.get("model"))
    pricing = dict(DEEPSEEK_MODEL_PRICING_PER_MILLION.get(model, DEEPSEEK_MODEL_PRICING_PER_MILLION[DEEPSEEK_DEFAULT_MODEL]))
    pricing.update(config.get("pricing_per_million_tokens") or {})

    prompt_tokens = int(usage.get("prompt_tokens") or 0)
    completion_tokens = int(usage.get("completion_tokens") or 0)
    cache_hit = int(usage.get("prompt_cache_hit_tokens") or usage.get("cache_hit_tokens") or 0)
    cache_miss = int(usage.get("prompt_cache_miss_tokens") or 0)
    uncached_input = cache_miss if cache_miss else max(0, prompt_tokens - cache_hit)

    return (
        cache_hit * float(pricing.get("cache_hit_input", pricing["input"]))
        + uncached_input * float(pricing["input"])
        + completion_tokens * float(pricing["output"])
    ) / 1_000_000


def record_deepseek_usage(
    state: dict[str, Any],
    purpose: str,
    model: str,
    usage: dict[str, Any],
    elapsed_seconds: float,
    config: dict[str, Any],
) -> None:
    cost = estimate_deepseek_cost(usage, config)
    api_usage = state.setdefault("api_usage", {}).setdefault(
        "deepseek",
        {
            "requests": 0,
            "elapsed_seconds": 0.0,
            "prompt_tokens": 0,
            "completion_tokens": 0,
            "total_tokens": 0,
            "estimated_cost_usd": 0.0,
            "by_purpose": {},
        },
    )
    api_usage["requests"] = int(api_usage.get("requests") or 0) + 1
    api_usage["elapsed_seconds"] = round(float(api_usage.get("elapsed_seconds") or 0) + elapsed_seconds, 3)
    api_usage["prompt_tokens"] = int(api_usage.get("prompt_tokens") or 0) + int(usage.get("prompt_tokens") or 0)
    api_usage["completion_tokens"] = int(api_usage.get("completion_tokens") or 0) + int(usage.get("completion_tokens") or 0)
    api_usage["total_tokens"] = int(api_usage.get("total_tokens") or 0) + int(usage.get("total_tokens") or 0)
    api_usage["estimated_cost_usd"] = round(float(api_usage.get("estimated_cost_usd") or 0) + cost, 8)
    api_usage["last_model"] = model

    bucket = api_usage.setdefault("by_purpose", {}).setdefault(
        purpose,
        {"requests": 0, "elapsed_seconds": 0.0, "prompt_tokens": 0, "completion_tokens": 0, "estimated_cost_usd": 0.0},
    )
    bucket["requests"] = int(bucket.get("requests") or 0) + 1
    bucket["elapsed_seconds"] = round(float(bucket.get("elapsed_seconds") or 0) + elapsed_seconds, 3)
    bucket["prompt_tokens"] = int(bucket.get("prompt_tokens") or 0) + int(usage.get("prompt_tokens") or 0)
    bucket["completion_tokens"] = int(bucket.get("completion_tokens") or 0) + int(usage.get("completion_tokens") or 0)
    bucket["estimated_cost_usd"] = round(float(bucket.get("estimated_cost_usd") or 0) + cost, 8)


class DeepSeekLengthTruncatedError(RuntimeError):
    """API 返回 finish_reason=length，输出被截断"""
    pass


def _dynamic_throttle_deepseek() -> None:
    """频繁 429 时临时降低并发到 1"""
    with DEEPSEEK_API_SEMAPHORE_LOCK:
        if DEEPSEEK_API_SEMAPHORE is not None:
            DEEPSEEK_API_SEMAPHORE.set_limit(1)
    logging.getLogger(__name__).warning("DeepSeek API concurrency throttled to 1 due to repeated 429s")


def deepseek_chat_completion(
    prompt: str,
    config: dict[str, Any],
    response_format: Any | None = None,
    _root_config: dict[str, Any] | None = None,
) -> tuple[str, dict[str, Any], float]:
    api_key = resolve_api_key(config, "DEEPSEEK_API_KEY")
    if not api_key:
        raise RuntimeError("DeepSeek API key is missing. Set DEEPSEEK_API_KEY or save it in config.")
    assert_deepseek_prompt_budget(prompt, config)

    sem = deepseek_api_semaphore(_root_config or config)
    sem.acquire()
    try:
        model = str(config.get("model") or DEEPSEEK_DEFAULT_MODEL)
        payload: dict[str, Any] = {
            "model": model,
            "messages": [{"role": "user", "content": prompt}],
            "temperature": float(config.get("temperature", 0)),
            "stream": False,
            "thinking": deepseek_thinking_config(config),
        }
        max_tokens = config.get("max_tokens", config.get("num_predict"))
        if max_tokens is not None:
            payload["max_tokens"] = int(max_tokens)
        if response_format is not None:
            payload["response_format"] = {"type": "json_object"}

        data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        timeout = int(config.get("timeout_seconds", 180))
        max_retries = 3
        last_exc: BaseException | None = None
        for attempt in range(max_retries + 1):
            req = urllib.request.Request(
                deepseek_chat_completions_url(config.get("base_url")),
                data=data,
                headers={"Content-Type": "application/json", "Authorization": f"Bearer {api_key}"},
                method="POST",
            )
            started = time.perf_counter()
            try:
                with urllib.request.urlopen(req, timeout=timeout) as response:
                    body = json.loads(response.read().decode("utf-8"))
            except urllib.error.HTTPError as exc:
                last_exc = exc
                classified = classify_upstream_error(exc, "DeepSeek API")
                if classified and classified.code == "RATE_LIMITED":
                    delay = float(min(120, max(1, classified.retry_after_seconds or (2 ** attempt + 1))))
                    if attempt >= 2:
                        _dynamic_throttle_deepseek()
                    logging.getLogger(__name__).warning(
                        "DeepSeek API rate limit 429, waiting %.1fs (attempt %s/%s)",
                        delay, attempt + 1, max_retries
                    )
                    if attempt < max_retries:
                        time.sleep(delay)
                        continue
                    raise classified from exc
                if is_retryable_http_error(exc):
                    if attempt < max_retries:
                        delay = 2 ** attempt
                        logging.getLogger(__name__).warning(
                            "DeepSeek API retryable error %s, retrying (%s/%s) in %ss",
                            exc.code, attempt + 1, max_retries, delay,
                        )
                        time.sleep(delay)
                        continue
                if classified:
                    raise classified from exc
                detail = exc.read().decode("utf-8", errors="replace")[:500]
                raise RuntimeError(f"DeepSeek API error {exc.code}: {detail}") from exc
            except urllib.error.URLError as exc:
                last_exc = exc
                if is_retryable_http_error(exc):
                    if attempt < max_retries:
                        delay = 2 ** attempt
                        logging.getLogger(__name__).warning(
                            "DeepSeek API network error, retrying (%s/%s) in %ss: %s",
                            attempt + 1, max_retries, delay, exc.reason,
                        )
                        time.sleep(delay)
                        continue
                classified = classify_upstream_error(exc, "DeepSeek API")
                if classified:
                    raise classified from exc
                raise RuntimeError(f"DeepSeek API network error: {exc.reason}") from exc

            elapsed = time.perf_counter() - started
            choices = body.get("choices") or []
            if not choices:
                raise RuntimeError("DeepSeek API returned no choices.")
            
            finish_reason = choices[0].get("finish_reason", "")
            if finish_reason == "length":
                raise DeepSeekLengthTruncatedError(
                    f"DeepSeek output truncated (finish_reason=length), "
                    f"prompt_tokens={body.get('usage', {}).get('prompt_tokens', '?')}, "
                    f"completion_tokens={body.get('usage', {}).get('completion_tokens', '?')}"
                )
            
            message = choices[0].get("message") or {}
            return str(message.get("content") or "").strip(), dict(body.get("usage") or {}), elapsed

        raise RuntimeError(f"DeepSeek API failed after {max_retries + 1} attempts") from last_exc
    finally:
        sem.release()
