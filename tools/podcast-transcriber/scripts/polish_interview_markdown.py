from __future__ import annotations

import argparse
import contextlib
import json
import logging
import os
import re
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

_EXTERNAL_DEEPSEEK_SEMAPHORE: Any | None = None
_LOCAL_DEEPSEEK_SEMAPHORE_LOCK = threading.Lock()

def set_deepseek_semaphore(sem: Any) -> None:
    """允许外部注入全局 DeepSeek 信号量"""
    global _EXTERNAL_DEEPSEEK_SEMAPHORE
    _EXTERNAL_DEEPSEEK_SEMAPHORE = sem


_POLISH_PROGRESS_REPORTER: Any | None = None
LAST_POLISH_SUMMARY: dict[str, Any] = {}


def set_polish_progress_reporter(reporter: Any) -> None:
    """允许外部注入润色进度回调 reporter(done, total)，用于 GUI 实时显示「润色中」进度"""
    global _POLISH_PROGRESS_REPORTER
    _POLISH_PROGRESS_REPORTER = reporter


def _report_polish_progress(done: int, total: int) -> None:
    if _POLISH_PROGRESS_REPORTER is None:
        return
    try:
        _POLISH_PROGRESS_REPORTER(int(done), int(total))
    except Exception:
        pass

def get_deepseek_semaphore(config: dict[str, Any]) -> Any:
    global _EXTERNAL_DEEPSEEK_SEMAPHORE
    if _EXTERNAL_DEEPSEEK_SEMAPHORE is not None:
        return _EXTERNAL_DEEPSEEK_SEMAPHORE
    with _LOCAL_DEEPSEEK_SEMAPHORE_LOCK:
        # 兜底：如果外部没有注入，在此创建
        # 尝试寻找 config 中可能的位置
        pipeline_cfg = config.get("pipeline") or config
        limit = pipeline_cfg.get("max_deepseek_api_requests", 4)
        try:
            limit = max(1, min(8, int(limit)))
        except (TypeError, ValueError):
            limit = 4
        _EXTERNAL_DEEPSEEK_SEMAPHORE = threading.Semaphore(limit)
        return _EXTERNAL_DEEPSEEK_SEMAPHORE

from deepseek_pricing import (  # noqa: E402
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
from podcast_transcriber.common import (  # noqa: E402
    CONFIG_PATH,
    OUT_FINAL_MARKDOWN,
    OUT_JSON,
    STATE_DIR,
    WORK,
)

# Managed task cache final dir (Cache/.../Tasks/{id}/output). Never write under Data root.
OUT_INTERVIEW = WORK / "internal" / "markdown_interview"
DEEPSEEK_POLISH_USAGE_PATH = STATE_DIR / "deepseek_polish_usage.json"
DEEPSEEK_PROMPT_TOKEN_LIMIT = 200_000
DEEPSEEK_PROMPT_HARD_LIMIT = 220_000


COMMON_REPLACEMENTS = {
    "OpenEye": "OpenAI",
    "Open AI": "OpenAI",
    "OpenEd": "OpenAI",
    "OpenEdge": "OpenAI",
    "OpenApp": "OpenAI",
    "OPI": "OpenAI",
    "小俊": "小珺",
    "张小军": "张小珺",
    "我是小军": "我是小珺",
    "姚顺语": "姚顺宇",
    "姚顺宇": "姚顺雨",
    "姚舜宇": "姚顺雨",
    "杨胜宇": "姚顺雨",
    "谢在明": "谢赛宁",
    "谢赞宁": "谢赛宁",
    "谢再宁": "谢赛宁",
    "謝賽寧": "谢赛宁",
    "楊麗昆": "杨立昆",
    "圖領獎": "图灵奖",
    "Jamina": "Gemini",
    "Gamna": "Gemini",
    "Dram9": "Gemini",
    "Authorpic": "Anthropic",
    "Enthropic": "Anthropic",
    "Anthorpe": "Anthropic",
    "Antarpe": "Anthropic",
    "Antharpic": "Anthropic",
    "Anthopic": "Anthropic",
    "Ansopic": "Anthropic",
    "A2P": "Anthropic",
    "Antropic": "Anthropic",
    "Cloud 3.7": "Claude 3.7",
    "Cloud 4.5": "Claude 4.5",
    "Stanf": "Stanford",
    "普丁斯顿": "普林斯顿",
    "普林当": "普林斯顿",
    "腰班": "姚班",
    "摇班": "姚班",
    "Deep Mind": "DeepMind",
    "Deepman": "DeepMind",
    "拆GPT": "ChatGPT",
    "XGBT": "ChatGPT",
    "GBT": "GPT",
    "Burt": "BERT",
    "AGA": "AGI",
    "Graph School": "grad school",
    "bringstomp": "brainstorm",
    "work to get whack": "word2vec",
    "work to whack": "word2vec",
    "Deeply Link": "deep learning",
    "deeplaining": "deep learning",
    "PHT": "PhD",
    "Different Bat": "different bet",
    "BetOn": "bet on",
    "multi-model": "multimodal",
    "skilling law": "scaling law",
    "Skilling Law": "Scaling Law",
    "Large English Model": "large language model",
    "Logical English Model": "large language model",
    "Large Change Model": "large language model",
    "word model": "world model",
    "overlife": "overlap",
    "Startup": "startup",
    "make that": "make bet",
    "Mita": "Meta",
    "非恶米": "非厄米",
    "算质": "特质",
    "做事，戏": "做事细",
    "蠢玩": "纯玩",
    "车车": "扯扯",
    "开Shop": "catch up",
    "实现这种": "实行这种",
    "Anthropic做一个公司来说": "Anthropic作为一个公司来说",
    "打发本了还就不一样": "打法本来就不一样",
    "我得我得": "我得",
    "预设念": "预训练",
}

COMMON_REGEX_REPLACEMENTS: tuple[tuple[str, str], ...] = (
    (r"OpenA(?![A-Za-z])", "OpenAI"),
    (r"GPT\s*([1234])(?![0-9A-Za-z])", r"GPT-\1"),
    (r"\bOpenAI\s*Tropic\b", "OpenAI、Anthropic"),
    (r"\bChat\s*GPT\b", "ChatGPT"),
    (r"\bDeep\s*Mind\b", "DeepMind"),
    (r"\bword\s+model\b", "world model"),
)

POLISH_SUSPICIOUS_PATTERNS: tuple[str, ...] = (
    "OpenA",
    "OpenEd",
    "OpenEdge",
    "OpenApp",
    "Antropic",
    "Anthorpe",
    "姚舜宇",
    "姚顺语",
    "杨胜宇",
    "谢在明",
    "谢赞宁",
    "普丁斯顿",
    "普林当",
    "腰班",
    "摇班",
    "拆GPT",
    "XGBT",
    "GBT",
    "Graph School",
    "bringstomp",
    "work to get whack",
    "work to whack",
    "Deeply Link",
    "deeplaining",
    "Large English Model",
    "Logical English Model",
    "Large Change Model",
    "word model",
    "AGA",
    "skilling law",
    "Skilling Law",
)

TRADITIONAL_CJK_RE = re.compile(r"[們個對為於這裡剛團隊規模獎獲楊麗圖領]")

TRADITIONAL_TO_SIMPLIFIED = str.maketrans(
    {
        "剛": "刚",
        "圖": "图",
        "靈": "灵",
        "領": "领",
        "獎": "奖",
        "楊": "杨",
        "麗": "丽",
        "們": "们",
        "團": "团",
        "隊": "队",
        "規": "规",
        "為": "为",
        "筆": "笔",
        "資": "资",
        "訴": "诉",
        "個": "个",
        "選": "选",
        "來": "来",
        "訪": "访",
        "談": "谈",
        "創": "创",
        "業": "业",
        "與": "与",
        "這": "这",
        "裡": "里",
        "對": "对",
        "間": "间",
        "夠": "够",
        "開": "开",
        "圍": "围",
        "繞": "绕",
        "腦": "脑",
        "樹": "树",
        "滿": "满",
        "補": "补",
        "覺": "觉",
        "樣": "样",
        "實": "实",
        "現": "现",
        "獲": "获",
        "關": "关",
        "係": "系",
        "學": "学",
        "術": "术",
        "體": "体",
        "並": "并",
        "將": "将",
    }
)


QUESTION_MARKERS = (
    "吗",
    "呢",
    "么",
    "为什么",
    "怎么",
    "是不是",
    "能不能",
    "要不要",
    "多久",
    "多频繁",
    "什么样",
    "你觉得",
    "你认为",
    "你可以",
    "你能",
    "你会",
    "对你来说",
    "给大家",
    "介绍一下",
    "评价一下",
)

HOST_HINTS = (
    "你",
    "您",
    "大家",
    "我们节目",
    "给大家",
    "我发现",
    "我想问",
    "接下来",
    "今天的节目",
    "这里是节目",
    "今天的嘉宾",
    "我们的谈话",
    "语言及世界工作室",
    "我们工作室",
)

GUEST_HINTS = (
    "我觉得",
    "对我来说",
    "我以前",
    "我本科",
    "我们本科",
    "我选择",
    "我没",
    "我可能",
    "我之前",
    "我们在硅谷",
    "去了",
    "加入了",
)

SHORT_HOST_INTERJECTIONS = {
    "需要脑子",
    "需要什么",
    "玩啥",
    "不关键",
    "你怎么知道的",
    "为什么",
}

HOST_CONTINUATION_PREFIXES = (
    "你",
    "你们",
    "您",
    "而且",
    "还是",
    "还",
    "多",
    "它是",
    "他是",
    "能给",
    "这个",
    "今天",
    "就是聊聊",
    "聊聊",
    "那怎么",
    "比如说你",
)

GUEST_ANSWER_PREFIXES = (
    "我",
    "我们",
    "对",
    "可以",
    "可能",
    "因为",
    "就是",
    "其实",
    "然后",
)

EN_HOST_STARTERS = (
    "welcome to ",
    "i'm andrew",
    "i am andrew",
    "and now for my discussion",
    "you're ",
    "you are ",
    "for those ",
    "maybe you could",
    "could you",
    "can you",
    "what ",
    "why ",
    "how ",
    "which ",
    "do you",
    "does ",
    "did ",
    "is there",
    "are there",
    "i'd love",
    "i would love",
    "i'd like",
    "i would like",
    "my understanding",
    "i wonder",
    "i want to",
    "if people",
    "you think",
)

EN_GUEST_STARTERS = (
    "thank you",
    "great to be here",
    "sure",
    "yes",
    "yeah",
    "the scope",
    "we take",
    "we clip",
    "we care",
    "historically",
    "so i'm",
    "all i do",
    "deep brain stimulation is",
    "my perspective",
    "i probably",
    "i do feel",
    "as a neurosurgeon",
    "well,",
    "well ",
)

SPONSOR_PATTERNS = (
    "sponsor",
    "functionhealth.com",
    "function provides",
    "function not only",
    "with function",
    "function membership",
    "drinkag1.com",
    "r-o-r-r-a",
    "rora.com",
    "rora water",
    "ag1",
    "rorra",
)

MIXED_SPONSOR_TAILS = (
    (" if people can ", "如果人们"),
    (" what is the status of ", "现状"),
)

BAD_FINAL_PATTERNS = (
    r"\{['\"][^'\"]+['\"]\s*:",
    r"\[TRANSLATION_MISSING\]",
    r"翻译缺失",
    r"应保留或译为",
    r"Here (?:is|are) the translation",
    r"Translated text",
    r"(?im)^Original:\s*$",
    r"(?im)^Translation:\s*$",
    r"(?i)[A-Za-z]:\\[^\s]+",  # absolute Windows paths
    r"(?i)/Users/[^\s]+",
    r"(?i)faster-whisper",
    r"(?i)models[/\\]",
)

ZH_RESPONSE_STARTERS = (
    "对",
    "是",
    "没错",
    "嗯",
    "可以",
    "其实",
    "因为",
    "然后",
    "我觉得",
    "我认为",
    "我想",
    "我们",
)

ZH_HOST_STARTERS = (
    "但是你",
    "那你",
    "所以你",
    "你",
    "你们",
    "您",
    "那是不是",
    "也就是说",
    "我想问",
    "我发现",
    "接下来",
)

EN_DANGLING_ENDINGS = (
    " and",
    " but",
    " or",
    " that",
    " than",
    " because",
    " if",
    " when",
    " where",
    " which",
    " with",
    " for",
    " from",
    " to",
    " into",
    " of",
    " a",
    " the",
    " our",
    " my",
    " your",
    " their",
    " could this be",
    " spatial precision",
    " little",
)

EN_CONTINUATION_STARTERS = (
    "isn't ",
    "is not ",
    "aren't ",
    "are not ",
    "doesn't ",
    "do not ",
    "didn't ",
    "could ",
    "would ",
    "should ",
    "can ",
    "may ",
    "might ",
    "where ",
    "which ",
    "that ",
    "than ",
    "because ",
    "if ",
    "when ",
    "we're ",
    "we are ",
    "i'm ",
    "i am ",
    "you are ",
    "you're ",
    "help ",
    "bit ",
    "for ",
    "of ",
    "to ",
    "into ",
)

logger = logging.getLogger(__name__)


def load_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8-sig"))


def save_text(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def save_json(path: Path, data: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = json.dumps(data, ensure_ascii=False, indent=2)
    last_error: Exception | None = None
    for attempt in range(8):
        tmp = path.with_name(f"{path.name}.{os.getpid()}.{time.time_ns()}.tmp")
        try:
            tmp.write_text(payload, encoding="utf-8")
            os.replace(tmp, path)
            return
        except OSError as exc:
            last_error = exc
            try:
                if tmp.exists():
                    tmp.unlink()
            except OSError:
                pass
            time.sleep(0.2 * (attempt + 1))
    if last_error:
        raise last_error


def load_config() -> dict[str, Any]:
    if CONFIG_PATH.exists():
        return load_json(CONFIG_PATH)
    return {}


def provider_name(config: dict[str, Any]) -> str:
    return str(config.get("backend", config.get("provider", "ollama"))).strip().lower()


def has_api_entry(config: dict[str, Any], default_env: str = "DEEPSEEK_API_KEY") -> bool:
    if not str(config.get("base_url") or "").strip():
        return False
    env_name = str(config.get("api_key_env") or default_env).strip()
    return bool(env_name or str(config.get("api_key") or "").strip())


def effective_provider_name(config: dict[str, Any]) -> str:
    provider = provider_name(config)
    if provider == "deepseek" and not has_api_entry(config):
        raise RuntimeError("DeepSeek API backend is selected, but no base_url/API key entry is configured.")
    return provider


def resolve_api_key(config: dict[str, Any], default_env: str) -> str:
    env_name = str(config.get("api_key_env") or default_env).strip()
    if env_name and os.environ.get(env_name):
        return str(os.environ[env_name]).strip()
    return str(config.get("api_key") or "").strip()


def estimate_text_tokens(text: str) -> int:
    if not text:
        return 0
    return max(1, (len(text.encode("utf-8")) + 2) // 3)


def deepseek_prompt_limit(config: dict[str, Any], hard: bool = False) -> int:
    default = DEEPSEEK_PROMPT_HARD_LIMIT if hard else DEEPSEEK_PROMPT_TOKEN_LIMIT
    key = "hard_prompt_token_limit" if hard else "prompt_token_limit"
    try:
        return max(1_000, int(config.get(key, default)))
    except (TypeError, ValueError):
        return default


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


_POLISH_USAGE_LOCK = threading.Lock()
DEEPSEEK_POLISH_USAGE_LOCK_PATH = STATE_DIR / "deepseek_polish_usage.lock"


@contextlib.contextmanager
def exclusive_file_lock(path: Path):
    path.parent.mkdir(parents=True, exist_ok=True)
    handle = path.open("a+b")
    try:
        if os.name == "nt":
            import msvcrt  # noqa: E402

            if handle.seek(0, os.SEEK_END) == 0:
                handle.write(b"\0")
                handle.flush()
            handle.seek(0)
            msvcrt.locking(handle.fileno(), msvcrt.LK_LOCK, 1)
        else:
            import fcntl  # noqa: E402

            fcntl.flock(handle.fileno(), fcntl.LOCK_EX)
        yield
    finally:
        try:
            if os.name == "nt":
                import msvcrt  # noqa: E402

                handle.seek(0)
                msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                import fcntl  # noqa: E402

                fcntl.flock(handle.fileno(), fcntl.LOCK_UN)
        finally:
            handle.close()

def record_deepseek_polish_usage(model: str, usage: dict[str, Any], elapsed_seconds: float, config: dict[str, Any]) -> None:
    with _POLISH_USAGE_LOCK:
        with exclusive_file_lock(DEEPSEEK_POLISH_USAGE_LOCK_PATH):
            data = load_json(DEEPSEEK_POLISH_USAGE_PATH) if DEEPSEEK_POLISH_USAGE_PATH.exists() else {}
            cost = estimate_deepseek_cost(usage, config)
            data["requests"] = int(data.get("requests") or 0) + 1
            data["elapsed_seconds"] = round(float(data.get("elapsed_seconds") or 0) + elapsed_seconds, 3)
            data["prompt_tokens"] = int(data.get("prompt_tokens") or 0) + int(usage.get("prompt_tokens") or 0)
            data["completion_tokens"] = int(data.get("completion_tokens") or 0) + int(usage.get("completion_tokens") or 0)

            data["prompt_cache_hit_tokens"] = int(data.get("prompt_cache_hit_tokens") or 0) + int(usage.get("prompt_cache_hit_tokens") or 0)
            data["prompt_cache_miss_tokens"] = int(data.get("prompt_cache_miss_tokens") or 0) + int(usage.get("prompt_cache_miss_tokens") or 0)

            data["total_tokens"] = int(data.get("total_tokens") or 0) + int(usage.get("total_tokens") or 0)
            data["estimated_cost_usd"] = round(float(data.get("estimated_cost_usd") or 0) + cost, 8)
            data["last_model"] = model
            save_json(DEEPSEEK_POLISH_USAGE_PATH, data)


class DeepSeekLengthTruncatedError(RuntimeError):
    """API 返回 finish_reason=length，输出被截断"""
    pass


def _dynamic_throttle_deepseek_polish(config: dict[str, Any]) -> None:
    """频繁 429 时临时降低并发到 1"""
    sem = get_deepseek_semaphore(config)
    if hasattr(sem, "set_limit"):
        sem.set_limit(1)
    else:
        with _LOCAL_DEEPSEEK_SEMAPHORE_LOCK:
            global _EXTERNAL_DEEPSEEK_SEMAPHORE
            _EXTERNAL_DEEPSEEK_SEMAPHORE = threading.Semaphore(1)
    logging.getLogger(__name__).warning("DeepSeek API concurrency throttled to 1 due to repeated 429s in polish stage")


def deepseek_chat_completion(prompt: str, config: dict[str, Any], response_format: Any | None = None) -> tuple[str, dict[str, Any], float]:
    api_key = resolve_api_key(config, "DEEPSEEK_API_KEY")
    if not api_key:
        raise RuntimeError("DeepSeek API key is missing. Set DEEPSEEK_API_KEY or save it in config.")
    estimated = estimate_text_tokens(prompt)
    limit = deepseek_prompt_limit(config, hard=True)
    if estimated > limit:
        raise PromptBudgetError(f"DeepSeek prompt estimated at {estimated} tokens, above safety hard limit {limit}.")
    
    sem = get_deepseek_semaphore(config)
    sem.acquire()
    try:
        model = normalize_deepseek_model(config.get("model"))
        payload: dict[str, Any] = {
            "model": model,
            "messages": [{"role": "user", "content": prompt}],
            "temperature": float(config.get("temperature", 0.1)),
            "stream": False,
            "thinking": deepseek_thinking_config(config),
        }
        max_tokens = config.get("max_tokens", config.get("num_predict"))
        if max_tokens is not None:
            payload["max_tokens"] = int(max_tokens)
        if response_format is not None:
            payload["response_format"] = {"type": "json_object"}
        data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        timeout = int(config.get("timeout_seconds", 300))
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
                        _dynamic_throttle_deepseek_polish(config)
                    
                    logging.getLogger(__name__).warning(
                        "DeepSeek API rate limit 429 in polish, waiting %.1fs (attempt %s/%s)",
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
                    f"DeepSeek output truncated in polish (finish_reason=length), "
                    f"prompt_tokens={body.get('usage', {}).get('prompt_tokens', '?')}, "
                    f"completion_tokens={body.get('usage', {}).get('completion_tokens', '?')}"
                )
            
            return str((choices[0].get("message") or {}).get("content") or "").strip(), dict(body.get("usage") or {}), elapsed

        raise RuntimeError(f"DeepSeek API failed after {max_retries + 1} attempts") from last_exc
    finally:
        sem.release()


def format_hms(seconds: float | int | None) -> str:
    if seconds is None:
        return "未知"
    total = max(0, int(seconds))
    h, rem = divmod(total, 3600)
    m, s = divmod(rem, 60)
    return f"{h:02d}:{m:02d}:{s:02d}"


def _is_chinese_dominant(text: str) -> bool:
    """Check if text is Chinese-dominant (CJK chars >= Latin chars)."""
    cjk_count = len(re.findall(r"[\u3400-\u9fff]", text))
    latin_count = len(re.findall(r"[A-Za-z]", text))
    return cjk_count >= latin_count and cjk_count > 5


def clean_text(text: str) -> str:
    text = text.strip()
    for bad, good in COMMON_REPLACEMENTS.items():
        text = text.replace(bad, good)
    for pattern, replacement in COMMON_REGEX_REPLACEMENTS:
        text = re.sub(pattern, replacement, text, flags=re.IGNORECASE)
    text = re.sub(r"\s+", " ", text)
    # Only convert to Chinese punctuation when text is Chinese-dominant
    if _is_chinese_dominant(text):
        text = text.translate(TRADITIONAL_TO_SIMPLIFIED)
        text = text.replace(" ,", "，").replace(",", "，")
        text = text.replace("。 。", "。")
    text = text.replace("靠谱一下", "科普一下")
    text = text.replace("特质就是科普", "特质就是靠谱")
    return text.strip()


def looks_like_question(text: str) -> bool:
    if any(marker in text for marker in QUESTION_MARKERS):
        if "我觉得" in text and "你觉得" not in text:
            return False
        return True
    return False


def looks_like_english_question(text: str) -> bool:
    compact = re.sub(r"\s+", " ", text.strip().lower())
    if not compact:
        return False
    if "?" in compact:
        return True
    return compact.startswith(EN_HOST_STARTERS)


def contains_cjk(text: str) -> bool:
    return bool(re.search(r"[\u4e00-\u9fff]", text))


def inferred_language(segments: list[dict[str, Any]], detected_language: str | None = None) -> str:
    lang = str(detected_language or "").lower().split("-")[0]
    if lang in {"en", "zh"}:
        return lang
    sample = " ".join(str(segment.get("text", "")) for segment in segments[:80])
    cjk_chars = len(re.findall(r"[\u4e00-\u9fff]", sample))
    ascii_words = len(re.findall(r"\b[a-zA-Z]{2,}\b", sample))
    return "zh" if cjk_chars > ascii_words else "en"


def normalize_english_fragment(text: str) -> str:
    return re.sub(r"\s+", " ", text.strip().lower()).rstrip()


def ends_like_unfinished_english(text: str) -> bool:
    compact = normalize_english_fragment(text)
    if not compact:
        return False
    if compact[-1:] in ".?!":
        return False
    if compact.endswith((",", ";", ":")):
        return True
    return compact.endswith(EN_DANGLING_ENDINGS)


def starts_like_english_continuation(text: str) -> bool:
    compact = normalize_english_fragment(text)
    if not compact:
        return False
    if compact.startswith(EN_CONTINUATION_STARTERS):
        return True
    if re.match(r"^(is|are|was|were|be|been|being|has|have|had)\b", compact):
        return True
    return False


def should_continue_previous_turn(current: dict[str, Any] | None, original: str, gap: float, previous_speaker: str = "") -> bool:
    if current is None or gap < -0.25 or gap > 5.0:
        return False
    current_speaker = current.get("speaker", "")
    if previous_speaker and current_speaker and previous_speaker != current_speaker:
        # Different speakers detected by inference — block continuation merge.
        # This is a safety guard against overly aggressive English continuation logic.
        return False
    previous_original = original_join(current.get("original_parts", []))
    if not previous_original:
        return False
    if ends_like_unfinished_english(previous_original) and starts_like_english_continuation(original):
        return True
    if ends_like_unfinished_english(previous_original) and gap <= 1.2 and starts_like_english_continuation(original):
        return True
    if len(original.split()) <= 4 and starts_like_english_continuation(original) and not looks_like_english_question(original):
        return True
    return False


def is_sponsor_text(text: str) -> bool:
    lower = text.lower()
    return any(pattern in lower for pattern in SPONSOR_PATTERNS)


def sponsor_window_seconds(text: str) -> float:
    lower = text.lower()
    if "rora" in lower or "rorra" in lower or "r-o-r-r-a" in lower:
        return 0.0
    return 90.0


def split_mixed_sponsor_segments(segments: list[dict[str, Any]]) -> list[dict[str, Any]]:
    expanded: list[dict[str, Any]] = []
    for segment in segments:
        original = clean_text(str(segment.get("text", "")))
        translation = clean_text(str(segment.get("translation", ""))) if segment.get("translation") else ""
        lower = f" {original.lower()} "
        split_at: int | None = None
        translation_split_at: int | None = None
        if is_sponsor_text(original):
            for original_marker, translation_marker in MIXED_SPONSOR_TAILS:
                marker_at = lower.find(original_marker)
                if marker_at > 20:
                    split_at = max(0, marker_at - 1)
                    if translation and translation_marker:
                        found_translation_at = translation.find(translation_marker)
                        if found_translation_at > 20:
                            translation_split_at = found_translation_at
                    break
        if split_at is None:
            expanded.append(segment)
            continue

        before_original = original[:split_at].strip()
        after_original = original[split_at:].strip()
        if not before_original or not after_original:
            expanded.append(segment)
            continue

        before_translation = translation
        after_translation = ""
        if translation_split_at is not None:
            before_translation = translation[:translation_split_at].strip()
            after_translation = translation[translation_split_at:].strip()

        start = float(segment.get("start", 0))
        end = float(segment.get("end", start))
        ratio = min(0.85, max(0.15, len(before_original) / max(1, len(original))))
        mid = round(start + (end - start) * ratio, 3)

        before = dict(segment)
        before["text"] = before_original
        if translation:
            before["translation"] = before_translation
        before["start"] = start
        before["end"] = mid

        after = dict(segment)
        after["text"] = after_original
        if translation:
            after["translation"] = after_translation
        after["start"] = mid
        after["end"] = end

        expanded.extend([before, after])
    return expanded


MIXED_DIALOGUE_MARKERS = (
    "Hello 顺宇",
    "Hello顺宇",
    "广蜜你也来",
    "大家好 我叫",
    "大家好我叫",
    "大家好 我是",
    "大家好我是",
)


def split_mixed_dialogue_segments(segments: list[dict[str, Any]]) -> list[dict[str, Any]]:
    expanded: list[dict[str, Any]] = []
    for segment in segments:
        original = clean_text(str(segment.get("text", "")))
        if not original:
            expanded.append(segment)
            continue

        split_points = {0, len(original)}
        for marker in MIXED_DIALOGUE_MARKERS:
            start = original.find(marker)
            if start > 12:
                split_points.add(start)

        ordered = sorted(split_points)
        if len(ordered) <= 2:
            expanded.append(segment)
            continue

        start_time = float(segment.get("start", 0))
        end_time = float(segment.get("end", start_time))
        duration = max(0.0, end_time - start_time)
        parts: list[tuple[int, int]] = []
        for left, right in zip(ordered, ordered[1:], strict=True):
            part = original[left:right].strip(" ，")
            if part:
                parts.append((left, right))
        if len(parts) <= 1:
            expanded.append(segment)
            continue

        for left, right in parts:
            item = dict(segment)
            item["text"] = original[left:right].strip(" ，")
            item["start"] = round(start_time + duration * (left / max(1, len(original))), 3)
            item["end"] = round(start_time + duration * (right / max(1, len(original))), 3)
            expanded.append(item)
    return expanded


def infer_speaker_from_original(text: str, previous: str, after_host_question: bool) -> str:
    compact = re.sub(r"\s+", " ", text.strip().lower())
    if not compact:
        return previous or "说话人待校对"
    if is_sponsor_text(compact):
        return "主持人"
    if compact.startswith("which is ") and "?" not in compact:
        return previous or "说话人待校对"
    if (
        compact.startswith(("which ", "that ", "and ", "but ", "or "))
        and "?" not in compact
        and not re.match(r"^(which|what|how|why)\s+(do|does|did|are|is|can|could|would|should)\b", compact)
    ):
        return previous or "说话人待校对"
    if (
        previous == "嘉宾"
        and compact.startswith(("which ", "that ", "and ", "but ", "or "))
        and "?" not in compact
        and not re.match(r"^(which|what|how|why)\s+(do|does|did|are|is|can|could|would|should)\b", compact)
    ):
        return "嘉宾"
    if compact.startswith(EN_HOST_STARTERS) or looks_like_english_question(compact):
        return "主持人"
    if compact.startswith(EN_GUEST_STARTERS):
        return "嘉宾"
    if after_host_question:
        return "嘉宾"
    if previous == "嘉宾" and not looks_like_english_question(compact):
        return "嘉宾"
    if previous == "主持人" and len(compact.split()) <= 6:
        return "说话人待校对"
    return previous or "说话人待校对"


def infer_speaker(text: str, previous: str, after_host_question: bool, intro: bool = False) -> str:
    compact = re.sub(r"\s+", "", text)
    if compact.startswith(("大家好我叫", "大家好我是", "大家好，我叫", "大家好，我是")):
        return "嘉宾" if intro or previous == "主持人" else previous or "嘉宾"
    if compact.startswith(("我是姚", "我是谢")) and (intro or previous == "主持人"):
        return "嘉宾"
    if compact.startswith(
        (
            "欢迎收听",
            "今天的嘉宾",
            "这是一档",
            "我们希望和你一起",
            "我们的谈话",
            "前不久我刚刚创立",
            "我们工作室",
            "为什么我们相信",
            "我看了你的",
            "是因为我看了你的",
            "我对你这个人",
            "你能不能",
            "你可以",
            "你之前",
            "你是",
            "那怎么",
            "就是聊聊你的",
            "聊聊你的",
            "除了我还有",
            "Hello顺宇",
            "广蜜你也来",
            "拥有最早的记忆",
            "来给我们讲",
        )
    ):
        return "主持人"
    if compact.startswith(("大家好", "欢迎", "这里是", "今天")):
        return "主持人"
    if compact.startswith("我是"):
        if previous == "主持人" and intro:
            return "嘉宾"
        return previous or "主持人"
    if compact.startswith(ZH_HOST_STARTERS):
        return "主持人"
    if compact.startswith(("你", "你们", "您")):
        return "主持人"
    if compact.startswith(("我想问", "我发现")):
        return "主持人"
    if compact.startswith(("我觉得", "我没", "对我来说", "可能", "可以", "因为", "其实", "然后因为")):
        return "嘉宾"
    if intro and previous == "嘉宾" and compact.startswith(("然后现在在", "现在在")):
        return "嘉宾"
    if "我其实" in compact and "你" not in compact:
        return "嘉宾"
    if previous == "嘉宾" and "你" not in compact and compact.startswith(("不", "而是", "或者", "是", "什么叫做")):
        return "嘉宾"
    if compact in SHORT_HOST_INTERJECTIONS:
        return "主持人"
    if looks_like_question(text):
        return "主持人"
    if after_host_question:
        if compact.startswith(HOST_CONTINUATION_PREFIXES) and not compact.startswith(GUEST_ANSWER_PREFIXES):
            return "主持人"
        if compact.endswith(("对吧", "吗", "呢", "么")):
            return "主持人"
        return "嘉宾"
    guest_score = sum(1 for hint in GUEST_HINTS if hint in text)
    host_score = sum(1 for hint in HOST_HINTS if hint in text)
    if guest_score > host_score:
        return "嘉宾"
    if host_score > guest_score and previous != "嘉宾":
        return "主持人"
    if compact.startswith(ZH_RESPONSE_STARTERS):
        return "嘉宾" if previous == "主持人" else previous or "嘉宾"
    return previous or "嘉宾"


def sentence_join(parts: list[str]) -> str:
    text = "".join(parts)
    text = re.sub(r"\s+", " ", text)
    text = re.sub(r"([，。！？；：])+", lambda m: m.group(1), text)
    text = text.replace("，。", "。").replace("。。", "。")
    if text and text[-1] not in "。！？：；」』”）)":
        text += "。"
    return text


def original_join(parts: list[str]) -> str:
    text = " ".join(part.strip() for part in parts if part.strip())
    text = re.sub(r"\s+", " ", text)
    text = re.sub(r"\s+([,.;:!?])", r"\1", text)
    text = re.sub(r"(?<=[\u4e00-\u9fff])\s+(?=[\u4e00-\u9fff])", "", text)
    return text.strip()


def chinese_original_join(parts: list[str]) -> str:
    cleaned: list[str] = []
    for part in parts:
        item = clean_text(part).replace(",", "，").strip(" ，")
        if item:
            cleaned.append(item)
    if not cleaned:
        return ""
    text = "，".join(cleaned)
    text = re.sub(r"，([。！？；：])", r"\1", text)
    text = re.sub(r"([。！？；：])，", r"\1", text)
    text = re.sub(r"，{2,}", "，", text)
    if text and text[-1] not in "。！？：；」』”）)":
        text += "。"
    return text


def paragraphize(text: str, max_chars: int = 700) -> list[str]:
    text = text.strip()
    if not text:
        return []

    raw_paragraphs = re.split(r"\n\s*\n", text)
    result: list[str] = []
    for para in raw_paragraphs:
        para = re.sub(r"\s*\n\s*", "", para).strip()
        if not para:
            continue
        if len(para) <= max_chars:
            result.append(para)
        else:
            sentences = re.split(r"(?<=[。！？；])", para)
            current = ""
            for sentence in sentences:
                sentence = sentence.strip()
                if not sentence:
                    continue
                if current and len(current) + len(sentence) > max_chars:
                    result.append(current)
                    current = sentence
                else:
                    current += sentence
            if current:
                result.append(current)
    return result or [text]


def ollama_generate(prompt: str, model: str, timeout: int = 300, llm_config: dict[str, Any] | None = None) -> str:
    url = "http://127.0.0.1:11434/api/generate"
    if llm_config:
        url = str(llm_config.get("ollama_url", url))
    payload = {
        "model": model,
        "prompt": prompt,
        "stream": False,
        "keep_alive": llm_config.get("keep_alive", "30m") if llm_config else "30m",
        "think": bool(llm_config.get("think", False)) if llm_config else False,
        "options": {
            "temperature": float(llm_config.get("temperature", 0.1)) if llm_config else 0.1,
            "num_ctx": int(llm_config.get("num_ctx", 8192)) if llm_config else 8192,
            "num_predict": int(llm_config.get("num_predict", 2048)) if llm_config else 2048,
            "num_thread": int(llm_config.get("num_thread", 4)) if llm_config else 4,
        },
    }
    data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
    req = urllib.request.Request(
        url,
        data=data,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=timeout) as response:
        body = json.loads(response.read().decode("utf-8"))
    return str(body.get("response", "")).strip()


def compact_context_excerpt(text: str, limit: int = 700) -> str:
    text = re.sub(r"\s+", " ", text).strip()
    if len(text) <= limit:
        return text
    head = max(120, limit // 2)
    tail = max(120, limit - head)
    return f"{text[:head].rstrip()} ... {text[-tail:].lstrip()}"


def polish_text_with_llm(text: str, speaker: str, llm_config: dict[str, Any], context: str = "") -> str:
    if not text.strip():
        return text
    backend = effective_provider_name(llm_config)
    model = llm_config.get("model", DEEPSEEK_DEFAULT_MODEL if backend == "deepseek" else "qwen3.5:9b")
    timeout = int(llm_config.get("timeout_seconds", 300))
    max_text_chars = int(llm_config.get("max_text_chars", 12_000))
    if len(text) > max_text_chars:
        logger.warning("Skipping LLM polish for %s: text length %s exceeds max_text_chars=%s", speaker, len(text), max_text_chars)
        return text
    context_block = f"\n上下文（只供判断语义和说话人关系，不要输出）：\n{context.strip()}\n" if context.strip() else ""
    prompt = (
        "/no_think\n"
        "你是中文播客口语整理编辑。请只整理“当前文本”，写成像真人播客逐字稿编辑后的自然中文。\n"
        "硬性规则：\n"
        "1. 不要总结，不要新增事实，不要删除信息点。\n"
        "2. 根据上下文判断这是采访者还是受访者的表达，保留原说话人的语气、立场、问答关系和信息顺序。\n"
        "3. 不要把采访者的问题改成受访者回答，也不要把受访者回答改成采访者追问。\n"
        "4. 保留人名、公司名、产品名、地名、技术术语、数字、金额、年份、百分比和专有名词；只有上下文非常明确时才修正明显错字。\n"
        "5. 去掉明显口癖、无意义重复和机械直译腔，例如“you know / sort of / basically / copy”一类不承载信息的填充词，但不要删掉真实观点。\n"
        "6. 修复断句、标点、上下句衔接和不自然用词；英文音频译文要像中文播客稿，不要像机翻。\n"
        "7. 如果同一个说话人讲得太长，可以按话题、语义转折或论点层次分段；段落之间用一个空行隔开。\n"
        "8. 如果上下文显示当前文本承接前后对话，分段时要保留这种承接关系。\n"
        "9. 不要输出说话人标签、标题、列表、解释、注释、Markdown 引用符号或任何额外说明。\n"
        "10. 输出必须只包含润色后的当前文本，不要复述上下文。\n"
        f"{context_block}\n"
        f"当前说话人：{speaker}\n"
        f"当前文本：{text}"
    )
    estimated = estimate_text_tokens(prompt)
    limit = deepseek_prompt_limit(llm_config) if backend == "deepseek" else int(llm_config.get("num_ctx", 8192))
    if estimated > limit:
        logger.warning("Skipping LLM polish for %s: prompt estimate %s exceeds limit %s", speaker, estimated, limit)
        return text
    if backend == "deepseek":
        reservation = reserve_budget(prompt, llm_config)
        try:
            result, usage, elapsed = deepseek_chat_completion(prompt, llm_config)
        except Exception:
            settle_budget(reservation, None, llm_config)
            raise
        settle_budget(reservation, usage, llm_config)
        record_deepseek_polish_usage(str(model), usage, elapsed, llm_config)
        logger.info(
            "DeepSeek polish request completed in %.2fs, tokens=%s, estimated_cost=$%.6f",
            elapsed,
            usage.get("total_tokens", "?"),
            estimate_deepseek_cost(usage, llm_config),
        )
    elif backend == "ollama":
        result = ollama_generate(prompt, str(model), timeout=timeout, llm_config=llm_config)
    else:
        raise RuntimeError(f"Unsupported polish backend: {backend}")
    return result or text


_MAX_CONSECUTIVE_POLISH_ERRORS = 3

_LLM_CONFIG_LOCK = threading.Lock()


def maybe_polish_text_with_llm(text: str, speaker: str, llm_config: dict[str, Any], context: str = "") -> str:
    if not bool(llm_config.get("enabled", False)) or not text.strip():
        return text
    with _LLM_CONFIG_LOCK:
        if llm_config.get("_disabled_after_error", False):
            return text
    try:
        result = polish_text_with_llm(text, speaker, llm_config, context=context)
        with _LLM_CONFIG_LOCK:
            llm_config["_consecutive_errors"] = 0
        return result
    except PromptBudgetError:
        raise
    except Exception as exc:
        with _LLM_CONFIG_LOCK:
            count = int(llm_config.get("_consecutive_errors", 0)) + 1
            llm_config["_consecutive_errors"] = count
            if count >= _MAX_CONSECUTIVE_POLISH_ERRORS:
                llm_config["_disabled_after_error"] = True
                logger.warning(
                    "LLM polish disabled after %s consecutive failures for %s: %s",
                    count, speaker, exc,
                )
            else:
                logger.warning("LLM polish failed for %s (%s/%s): %s", speaker, count, _MAX_CONSECUTIVE_POLISH_ERRORS, exc)
        if bool(llm_config.get("fallback_on_error", True)):
            return text
        raise


def polish_source_context(data: dict[str, Any]) -> str:
    title = Path(data.get("source_file", "podcast")).stem
    if not title:
        return ""
    return (
        f"标题/文件名：{title}\n"
        "标题中的人名、公司名、产品名、机构名和术语优先作为专名校对锚点。"
    )


def block_text_for_polish(block: dict[str, Any], is_english: bool) -> str:
    language_class = block.get("languageClass")
    prefer_translation = language_class in {"en", "mixed"} if language_class else is_english
    if prefer_translation:
        translated = sentence_join([t for t in block.get("translations", []) if t])
        if translated:
            return translated
    originals = [t for t in block.get("originals", []) if t]
    if language_class == "zh":
        return chinese_original_join(originals)
    return original_join(originals)


def polish_need_score(block: dict[str, Any], is_english: bool) -> int:
    text = block_text_for_polish(block, is_english)
    if not text.strip():
        return 0
    score = 0
    if bool(block.get("needs_polish")):
        score += 30
    lower = text.lower()
    for pattern in POLISH_SUSPICIOUS_PATTERNS:
        if pattern.lower() in lower:
            score += 35
    if TRADITIONAL_CJK_RE.search(text):
        score += 18
    if len(text) > (900 if is_english else 320):
        score += 10
    if len(re.findall(r"[A-Za-z]{3,}", text)) >= 6 and contains_cjk(text):
        score += 8
    if text.count("我觉得") >= 3 or text.count("然后") >= 4:
        score += 8
    if re.search(r"[A-Za-z]+[一-龥]|[一-龥][A-Za-z]+", text):
        score += 8
    return score


def select_polish_targets(
    blocks: list[dict[str, Any]],
    is_english: bool,
    use_llm: bool,
    only_suspect_llm_blocks: bool,
    llm_config: dict[str, Any],
) -> list[tuple[int, dict[str, Any]]]:
    if not use_llm:
        return []
    configured_limit = int(llm_config.get("max_blocks_per_file", 8))
    if configured_limit <= 0:
        configured_limit = len(blocks)
    if not is_english and only_suspect_llm_blocks:
        configured_limit = max(configured_limit, int(llm_config.get("min_chinese_suspect_blocks", 32)))

    scored: list[tuple[int, int, dict[str, Any]]] = []
    for idx, block in enumerate(blocks):
        language_class = str(block.get("languageClass") or "")
        block_is_english = language_class in {"en", "mixed"} if language_class else is_english
        score = polish_need_score(block, block_is_english)
        if only_suspect_llm_blocks and score <= 0:
            continue
        if not only_suspect_llm_blocks and score <= 0:
            score = 1
        scored.append((score, idx, block))

    scored.sort(key=lambda item: (-item[0], item[1]))
    selected = scored[:configured_limit]
    selected.sort(key=lambda item: item[1])
    logger.info(
        "LLM polish selected %s/%s blocks (only_suspect=%s, limit=%s)",
        len(selected),
        len(blocks),
        only_suspect_llm_blocks,
        configured_limit,
    )
    return [(idx, block) for _, idx, block in selected]


def _build_polish_batches(
    polish_targets: list[tuple[int, dict[str, Any]]],
    batch_blocks_limit: int,
    max_batch_chars: int,
    is_english: bool,
) -> list[list[tuple[int, dict[str, Any]]]]:
    batches: list[list[tuple[int, dict[str, Any]]]] = []
    current: list[tuple[int, dict[str, Any]]] = []
    current_chars = 0
    
    for item in polish_targets:
        idx, block = item
        text = block_text_for_polish(block, is_english)
        
        block_len = len(text)
        would_exceed_count = len(current) >= batch_blocks_limit
        would_exceed_chars = current_chars + block_len > max_batch_chars
        
        if current and (would_exceed_count or would_exceed_chars):
            batches.append(current)
            current = []
            current_chars = 0
        
        current.append(item)
        current_chars += block_len
        
    if current:
        batches.append(current)
    return batches


def polish_blocks_batch(
    blocks: list[dict[str, Any]],
    labels: dict[str, str],
    llm_config: dict[str, Any],
    is_english: bool,
    all_blocks: list[dict[str, Any]],
    block_indices: list[int],
    global_context: str = "",
) -> dict[int, str]:
    """批量润色多个 block，要求模型按 id 返回 JSON。
    
    Returns: {block_index: polished_text}
    """
    items = []
    for _i, (block, idx) in enumerate(zip(blocks, block_indices, strict=True)):
        speaker = label_for_speaker(block["speaker"], labels)
        text = block_text_for_polish(block, is_english)
        if not text.strip():
            continue
        
        context_parts = []
        if idx > 0:
            prev = all_blocks[idx - 1]
            prev_label = label_for_speaker(prev["speaker"], labels)
            prev_text = (sentence_join([t for t in prev.get("translations", []) if t])
                        if is_english else original_join([t for t in prev.get("originals", []) if t]))
            if prev_text:
                context_parts.append(f"上一段｜{prev_label}：{compact_context_excerpt(prev_text)}")
        if idx < len(all_blocks) - 1:
            nxt = all_blocks[idx + 1]
            nxt_label = label_for_speaker(nxt["speaker"], labels)
            nxt_text = (sentence_join([t for t in nxt.get("translations", []) if t])
                       if is_english else original_join([t for t in nxt.get("originals", []) if t]))
            if nxt_text:
                context_parts.append(f"下一段｜{nxt_label}：{compact_context_excerpt(nxt_text)}")
        
        items.append({
            "id": idx,
            "speaker": speaker,
            "text": text,
            "context": "\n".join(context_parts),
        })
        
    if not items:
        return {}
        
    payload_json = json.dumps(items, ensure_ascii=False)
    global_context_block = f"全局校对锚点：\n{global_context.strip()}\n\n" if global_context.strip() else ""
    prompt = (
        "/no_think\n"
        "你是中文播客口语整理编辑。请逐条整理以下播客文本 blocks，把整理润色后的结果填入 JSON 结果中。请整理成自然中文逐字稿。\n"
        f"{global_context_block}"
        "硬性规则（每条 block 都适用）：\n"
        "1. 不要总结，不要新增事实，不要删除信息点。\n"
        "2. 根据 speaker 与 context 判断采访者/受访者的问答关系，但不要输出说话人标签。\n"
        "3. 标题/文件名里出现的专名优先作为校对锚点；可修正明显听写错字，例如人名、公司名、产品名、学校名、模型名、技术术语。\n"
        "4. 不能确定的专名保持原样，不要凭空创造新人物或新机构。\n"
        "5. 去掉明显口癖和无意义重复，例如“you know / sort of / basically”一类填充词。\n"
        "6. 修复断句、标点、衔接和不自然用词；英文音频译文要自然，不要有翻译腔。\n"
        "7. 如果文本太长，可以按话题或论点分段，段落间用一个空行隔开。\n"
        "8. 不要在最终文本里输出说话人标签、标题、列表、解释、注释或额外说明。\n"
        "9. 提供的 context 信息只供理解语境，绝对不要把 context 的内容写入到最终文本中。\n\n"
        "返回一个合法的 JSON 格式。结构如下：\n"
        '{"results": [{"id": 0, "text": "这里是润色后的文本"}]}\n'
        "必须确保返回结果中的 id 与输入完全一致，不要遗漏或增加任何 block。\n\n"
        f"待整理的 Blocks：\n{payload_json}"
    )
    
    backend = effective_provider_name(llm_config)
    model = llm_config.get("model", DEEPSEEK_DEFAULT_MODEL if backend == "deepseek" else "qwen3.5:9b")
    
    if backend == "deepseek":
        reservation = reserve_budget(prompt, llm_config)
        try:
            result_text, usage, elapsed = deepseek_chat_completion(prompt, llm_config, response_format={"type": "json_object"})
        except Exception:
            settle_budget(reservation, None, llm_config)
            raise
        settle_budget(reservation, usage, llm_config)
        record_deepseek_polish_usage(str(model), usage, elapsed, llm_config)
        logger.info(
            "DeepSeek batch polish completed in %.2fs, tokens=%s, estimated_cost=$%.6f",
            elapsed,
            usage.get("total_tokens", "?"),
            estimate_deepseek_cost(usage, llm_config),
        )
    else:
        raise RuntimeError(f"Batch polish is not supported for backend: {backend}")
        
    try:
        parsed = json.loads(result_text)
        if isinstance(parsed, list):
            results_list = parsed
        elif isinstance(parsed, dict):
            results_list = parsed.get("results", [])
        else:
            results_list = []
        return {int(item["id"]): str(item["text"]).strip() for item in results_list if "id" in item and "text" in item}
    except (json.JSONDecodeError, KeyError, TypeError, ValueError) as exc:
        logger.warning("Batch polish JSON parse failed, falling back to individual polish: %s", exc)
        return {}


def should_add_break(text: str) -> bool:
    return bool(re.search(r"[。！？；]$", text))


def build_turns(segments: list[dict[str, Any]], detected_language: str | None = None) -> list[dict[str, Any]]:
    from podcast_transcriber.language import assign_language_classes, classify_segment_language  # noqa: E402

    turns: list[dict[str, Any]] = []
    current: dict[str, Any] | None = None
    previous_speaker = "主持人"
    after_host_question = False
    sponsor_until = -1.0
    prepared = assign_language_classes([dict(segment) for segment in segments], detected_language)
    lang = inferred_language(prepared, detected_language)
    is_english = lang == "en"

    def joined_original(parts: list[str], language_class: str) -> str:
        if language_class == "zh":
            return chinese_original_join(parts)
        return original_join(parts)

    def finish_current() -> None:
        nonlocal current
        if not current:
            return
        language_class = str(current.get("languageClass") or "en")
        current["original"] = joined_original(current.pop("original_parts"), language_class)
        current["translation"] = sentence_join(current.pop("translation_parts"))
        turns.append(current)
        current = None

    for segment in split_mixed_dialogue_segments(split_mixed_sponsor_segments(prepared)):
        original = clean_text(str(segment.get("text", "")))
        translation = clean_text(str(segment.get("translation", ""))) if segment.get("translation") else ""
        if not original:
            continue

        language_class = str(
            segment.get("languageClass")
            or classify_segment_language(original)
            or ("en" if is_english else "zh")
        )
        start = float(segment.get("start", 0))
        end = float(segment.get("end", start))
        section_bucket = int(start // 600) * 600
        sponsor_hit = is_sponsor_text(original)
        lower_for_sponsor = original.lower()
        if lower_for_sponsor.startswith(
            (
                "sort of a work",
                "this is a",
                "if people can",
                "maybe even",
                "it seems like",
                "i've always thought",
                "the problem",
                "they've tried",
            )
        ):
            sponsor_hit = False
            sponsor_until = -1.0
        if sponsor_hit and start > sponsor_until:
            if lower_for_sponsor.startswith(("again, that's", "again that's", "to learn more", "go to ", "r-o-r-r-a.com")):
                sponsor_until = end
            else:
                sponsor_until = start + sponsor_window_seconds(original)
        is_sponsor_segment = sponsor_hit or start <= sponsor_until
        segment_is_english = language_class in {"en", "mixed"}
        if is_sponsor_segment:
            speaker = "主持人"
        elif segment_is_english:
            speaker = infer_speaker_from_original(original, previous_speaker, after_host_question)
        else:
            speaker = infer_speaker(original, previous_speaker, after_host_question, intro=start < 45)

        max_chars = (
            700
            if language_class in {"en", "mixed"}
            else 420
        )
        gap = 0 if current is None else start - float(current["end"])
        continued_previous = (
            segment_is_english
            and (not sponsor_hit)
            and current is not None
            and speaker == current.get("speaker")
            and current.get("languageClass") == language_class
            and int(float(current["start"]) // 600) * 600 == section_bucket
            and should_continue_previous_turn(current, original, gap, previous_speaker)
        )
        if continued_previous:
            speaker = str(current["speaker"])
            is_sponsor_segment = bool(current.get("is_sponsor", is_sponsor_segment))
        current_len = (
            len(joined_original(current.get("original_parts", []), str(current.get("languageClass") or language_class)))
            if current
            else 0
        )
        new_question_after_long_turn = (not segment_is_english) and speaker == "主持人" and looks_like_question(original) and current_len > 80
        new_response_after_question = (not segment_is_english) and after_host_question and speaker == "嘉宾" and current is not None and current.get("speaker") == "主持人"
        force_new = (
            current is None
            or current["speaker"] != speaker
            or current.get("is_sponsor") != is_sponsor_segment
            or current.get("languageClass") != language_class
            or int(float(current["start"]) // 600) * 600 != section_bucket
            or current_len > max_chars
            or gap > (3.5 if segment_is_english else 2.2)
            or new_question_after_long_turn
            or new_response_after_question
        )

        if force_new:
            finish_current()
            current = {
                "speaker": speaker,
                "start": start,
                "end": end,
                "original_parts": [original],
                "translation_parts": [translation] if translation else [],
                "is_sponsor": is_sponsor_segment,
                "needs_polish": False,
                "languageClass": language_class,
            }
        else:
            assert current is not None
            current["original_parts"].append(original)
            if translation:
                current.setdefault("translation_parts", []).append(translation)
            current["end"] = end
            if continued_previous:
                current["needs_polish"] = True

        previous_speaker = speaker
        if speaker == "主持人" and (looks_like_english_question(original) if segment_is_english else looks_like_question(original)):
            after_host_question = True
        elif speaker == "嘉宾" and (len(original.split()) > 3 if segment_is_english else len(original) > 8):
            after_host_question = False

    finish_current()
    return merge_tiny_turns(turns)


def merge_tiny_turns(turns: list[dict[str, Any]], gap_threshold: float = 3.0) -> list[dict[str, Any]]:
    merged: list[dict[str, Any]] = []
    for turn in turns:
        gap = turn["start"] - merged[-1]["end"] if merged else float("inf")
        same_language = merged and merged[-1].get("languageClass") == turn.get("languageClass")
        same_section = (
            merged
            and int(float(merged[-1]["start"]) // 600) * 600 == int(float(turn["start"]) // 600) * 600
        )
        if (
            merged
            and same_language
            and same_section
            and merged[-1]["speaker"] == turn["speaker"]
            and merged[-1].get("is_sponsor") == turn.get("is_sponsor")
            and len(turn.get("original", "")) < 40
            and len(merged[-1].get("original", "")) < 600
            and gap <= gap_threshold
        ):
            language_class = str(turn.get("languageClass") or "en")
            join_fn = chinese_original_join if language_class == "zh" else original_join
            merged[-1]["original"] = join_fn([merged[-1].get("original", ""), turn.get("original", "")])
            merged[-1]["translation"] = sentence_join([merged[-1].get("translation", ""), turn.get("translation", "")])
            merged[-1]["end"] = turn["end"]
            merged[-1]["needs_polish"] = bool(merged[-1].get("needs_polish") or turn.get("needs_polish"))
        else:
            merged.append(turn)
    return merged


def merge_same_speaker_blocks(turns: list[dict[str, Any]], max_combined_chars: int = 1200) -> list[dict[str, Any]]:
    blocks: list[dict[str, Any]] = []
    for turn in turns:
        language_class = str(turn.get("languageClass") or "en")
        max_chars = 700 if language_class in {"en", "mixed"} else 420
        limit = min(max_combined_chars, max_chars)
        combined_original_len = (
            len(" ".join(blocks[-1].get("originals", []))) + len(turn.get("original", ""))
            if blocks
            else 0
        )
        same_section = (
            blocks
            and int(float(blocks[-1]["start"]) // 600) * 600 == int(float(turn["start"]) // 600) * 600
        )
        if (
            blocks
            and blocks[-1]["speaker"] == turn["speaker"]
            and blocks[-1].get("is_sponsor") == turn.get("is_sponsor")
            and blocks[-1].get("languageClass") == language_class
            and same_section
            and combined_original_len <= limit
        ):
            blocks[-1]["originals"].append(turn.get("original", ""))
            blocks[-1]["translations"].append(turn.get("translation", ""))
            blocks[-1]["end"] = turn["end"]
            blocks[-1]["needs_polish"] = bool(blocks[-1].get("needs_polish") or turn.get("needs_polish"))
        else:
            blocks.append(
                {
                    "speaker": turn["speaker"],
                    "start": turn["start"],
                    "end": turn["end"],
                    "originals": [turn.get("original", "")],
                    "translations": [turn.get("translation", "")],
                    "is_sponsor": turn.get("is_sponsor", False),
                    "needs_polish": bool(turn.get("needs_polish", False)),
                    "languageClass": language_class,
                }
            )
    return blocks


def render_interview_markdown(data: dict[str, Any], turns: list[dict[str, Any]]) -> str:
    title = Path(data.get("source_file", "podcast")).stem
    lines = [
        f"# {title}",
        "",
        "## 音频信息",
        "",
        f"- 原文件：{data.get('source_file', '')}",
        f"- 文件路径：{data.get('source_path', '')}",
        f"- 时长：{format_hms(data.get('duration_seconds'))}",
        f"- 转写模型：{data.get('model', '')}",
        f"- 设备：{data.get('device', '')} / {data.get('compute_type', '')}",
        "- 后处理：自动合并短句，按采访问答推断主持人/嘉宾，修正常见专名误识别",
        "- 注意：这不是声纹级说话人分离；如需严格说话人识别，后续应接入 WhisperX/pyannote diarization。",
        "",
        "## 采访稿",
        "",
    ]

    current_section = None
    for turn in turns:
        section = int(turn["start"] // 600) * 600
        if section != current_section:
            current_section = section
            lines.append(f"### {format_hms(section)}")
            lines.append("")
        lines.append(f"**{turn['speaker']}（{format_hms(turn['start'])}）：** {turn.get('original', '')}")
        if turn.get("translation"):
            lines.append("")
            lines.append(f"> {turn.get('translation', '')}")
        lines.append("")
    return "\n".join(lines)


def section_title_for_turn(turn: dict[str, Any], sponsor_mode: str) -> str:
    if turn.get("is_sponsor") and sponsor_mode == "section":
        original = turn.get("original", "").lower()
        if "ag1" in original:
            return "广告/赞助：AG1"
        if "rora" in original or "rorra" in original:
            return "广告/赞助：Rora"
        if "function" in original:
            return "广告/赞助：Function"
        return "广告/赞助"
    start = float(turn.get("start", 0))
    section_start = int(start // 600) * 600
    return format_hms(section_start)


def label_for_speaker(speaker: str, labels: dict[str, Any]) -> str:
    if speaker == "主持人":
        return str(labels.get("host", "采访者"))
    if speaker == "嘉宾":
        return str(labels.get("guest", "受访者"))
    return "说话人待校对"


def final_quality_errors(
    markdown: str,
    turns: list[dict[str, Any]],
    is_english: bool,
    max_paragraph_chars: int,
    rendered_blocks: list[dict[str, Any]] | None = None,
    require_speaker_roles: bool = False,
) -> list[str]:
    errors: list[str] = []
    for pattern in BAD_FINAL_PATTERNS:
        if re.search(pattern, markdown, flags=re.IGNORECASE):
            errors.append(f"final markdown contains forbidden pattern: {pattern}")
    # Missing translations only matter for en/mixed blocks — never Chinese-only.
    missing = [
        turn
        for turn in turns
        if turn.get("original")
        and not turn.get("is_sponsor")
        and str(turn.get("languageClass") or ("en" if is_english else "zh")) in {"en", "mixed"}
        and not turn.get("translation")
    ]
    if missing:
        errors.append(f"{len(missing)} turns are missing translation text")
    for paragraph in re.split(r"\n\s*\n", markdown):
        if len(paragraph) > max_paragraph_chars * 3:
            errors.append("final markdown contains an extremely long paragraph")
            break
    return errors


def render_final_markdown(data: dict[str, Any], turns: list[dict[str, Any]], config: dict[str, Any]) -> str:
    """Produce the managed final draft:

    # title
    ### HH:MM:SS
    English paragraph...

    Chinese translation...

    Chinese-only blocks emit polished Chinese only.
    """
    title = Path(data.get("source_file", "podcast")).stem
    final_config = config.get("markdown") or config.get("final_markdown") or {}
    labels = final_config.get("speaker_labels") or {}
    en_max_chars = int(final_config.get("max_paragraph_chars", 700))
    zh_max_chars = int(final_config.get("zh_max_paragraph_chars", 420))
    sponsor_mode = str(final_config.get("sponsor_mode", "drop")).lower()
    if sponsor_mode not in {"keep", "section", "drop"}:
        sponsor_mode = "drop"
    lang = str(data.get("detected_language") or "").lower().split("-")[0]
    is_english = lang == "en"
    render_turns = [turn for turn in turns if not (sponsor_mode == "drop" and turn.get("is_sponsor"))]
    for turn in render_turns:
        if not turn.get("languageClass"):
            from podcast_transcriber.language import classify_segment_language  # noqa: E402

            turn["languageClass"] = classify_segment_language(str(turn.get("original", ""))) or (
                "en" if is_english else "zh"
            )
    merge_limit = max(en_max_chars, zh_max_chars)
    blocks = (
        merge_same_speaker_blocks(render_turns, max_combined_chars=merge_limit)
        if final_config.get("merge_same_speaker", True)
        else [
            {
                "speaker": turn["speaker"],
                "start": turn["start"],
                "end": turn["end"],
                "originals": [turn.get("original", "")],
                "translations": [turn.get("translation", "")],
                "is_sponsor": turn.get("is_sponsor", False),
                "needs_polish": bool(turn.get("needs_polish", False)),
                "languageClass": turn.get("languageClass", "en" if is_english else "zh"),
            }
            for turn in render_turns
        ]
    )
    llm_config = dict(final_config.get("llm_polish") or {})
    use_llm = bool(llm_config.get("enabled", False))
    only_suspect_llm_blocks = bool(llm_config.get("only_suspect_blocks", True))
    global_polish_context = polish_source_context(data)

    # Polish uses Chinese text for zh blocks and translations for en/mixed.
    # Prefer translations for en/mixed blocks and originals for zh (via languageClass).
    polish_targets = select_polish_targets(
        blocks,
        any(str(block.get("languageClass")) in {"en", "mixed"} for block in blocks) or is_english,
        use_llm,
        only_suspect_llm_blocks,
        llm_config,
    )

    polished_texts: dict[int, str] = {}
    serial_polished = 0
    if use_llm and polish_targets:
        _report_polish_progress(0, len(polish_targets))
        backend = effective_provider_name(llm_config)
        if backend == "deepseek":
            batch_blocks_limit = int(llm_config.get("batch_blocks", 8))
            max_batch_chars = int(llm_config.get("max_batch_chars", 12000))
            batches = _build_polish_batches(polish_targets, batch_blocks_limit, max_batch_chars, True)
            for batch in batches:
                batch_blocks = [item[1] for item in batch]
                batch_indices = [item[0] for item in batch]
                try:
                    results = polish_blocks_batch(
                        batch_blocks,
                        labels,
                        llm_config,
                        True,
                        blocks,
                        batch_indices,
                        global_context=global_polish_context,
                    )
                    for idx, text in results.items():
                        if text:
                            polished_texts[idx] = text
                except Exception as e:
                    logger.warning("Batch polish request failed, falling back to individual polish: %s", e)
                _report_polish_progress(len(polished_texts), len(polish_targets))

    lines = [f"# {title}", ""]
    current_section: int | None = None

    def block_context(index: int) -> str:
        snippets: list[str] = []
        for label_name, ctx_index in (("上一段", index - 1), ("下一段", index + 1)):
            if ctx_index < 0 or ctx_index >= len(blocks):
                continue
            ctx_block = blocks[ctx_index]
            ctx_text = block_text_for_polish(ctx_block, str(ctx_block.get("languageClass")) in {"en", "mixed"})
            if ctx_text:
                snippets.append(f"{label_name}：{compact_context_excerpt(ctx_text)}")
        return "\n".join(snippets)

    for block_index, block in enumerate(blocks):
        language_class = str(block.get("languageClass") or "en")
        is_en_like = language_class in {"en", "mixed"}
        section = int(float(block.get("start", 0)) // 600) * 600
        if section != current_section:
            current_section = section
            lines.append(f"### {format_hms(section)}")
            lines.append("")

        original = original_join([item for item in block.get("originals", []) if item])
        if language_class == "zh":
            original = chinese_original_join([item for item in block.get("originals", []) if item])
        translated_text = sentence_join([item for item in block.get("translations", []) if item])

        is_target = any(idx == block_index for idx, _ in polish_targets)
        polish_label = label_for_speaker(str(block.get("speaker", "")), labels)
        if is_target:
            if block_index in polished_texts:
                if is_en_like and translated_text:
                    translated_text = polished_texts[block_index]
                else:
                    original = polished_texts[block_index]
            else:
                polish_context = "\n".join(
                    part for part in (global_polish_context, block_context(block_index)) if part
                )
                if is_en_like and translated_text:
                    translated_text = maybe_polish_text_with_llm(
                        translated_text, polish_label, llm_config, context=polish_context
                    )
                else:
                    original = maybe_polish_text_with_llm(
                        original, polish_label, llm_config, context=polish_context
                    )
                serial_polished += 1
                _report_polish_progress(len(polished_texts) + serial_polished, len(polish_targets))

        original = clean_text(original)
        if translated_text:
            translated_text = clean_text(translated_text)

        if is_en_like:
            # English (or mixed source) first, Chinese second.
            for paragraph in paragraphize(original, en_max_chars):
                lines.append(paragraph)
                lines.append("")
            if translated_text:
                for paragraph in paragraphize(translated_text, zh_max_chars):
                    lines.append(paragraph)
                    lines.append("")
            # Never emit missing placeholders for pure Chinese blocks; en/mixed
            # without translation is caught by final_quality_errors.
        else:
            for paragraph in paragraphize(original, zh_max_chars):
                lines.append(paragraph)
                lines.append("")

    processed_targets = len(polished_texts) + serial_polished
    global LAST_POLISH_SUMMARY
    LAST_POLISH_SUMMARY = {
        "blocks_total": len(blocks),
        "polish_targets": len(polish_targets),
        "polished_batch": len(polished_texts),
        "polished_serial": serial_polished,
        "polish_enabled": use_llm,
        "disabled_after_errors": bool(llm_config.get("_disabled_after_error", False)),
        "coverage_percent": round(processed_targets / len(polish_targets) * 100, 1)
        if polish_targets
        else (100.0 if use_llm else 0.0),
    }
    if use_llm and polish_targets:
        _report_polish_progress(processed_targets, len(polish_targets))

    markdown = "\n".join(lines).strip() + "\n"
    if bool(final_config.get("fail_on_quality_errors", True)):
        errors = final_quality_errors(
            markdown,
            render_turns,
            is_english,
            max(en_max_chars, zh_max_chars),
            rendered_blocks=blocks,
            require_speaker_roles=False,
        )
        if errors:
            raise RuntimeError("Final Markdown QA failed: " + "; ".join(errors))
    return markdown


def process_json(json_path: Path, final_only: bool = False) -> Path:
    config = load_config()
    data = load_json(json_path)
    turns = build_turns(data.get("segments", []), data.get("detected_language"))
    safe_name = Path(data.get("source_file", json_path.stem)).stem
    # Always publish into the managed cache output root so desktop publish can find it.
    final_dir = OUT_FINAL_MARKDOWN
    final_dir.mkdir(parents=True, exist_ok=True)
    final_path = final_dir / f"{safe_name}.md"
    save_text(final_path, render_final_markdown(data, turns, config))

    out_path = final_path
    if not final_only:
        interview_path = OUT_INTERVIEW / f"{safe_name}.interview.md"
        save_text(interview_path, render_interview_markdown(data, turns))

    state_path = STATE_DIR / f"{data.get('task_id', json_path.stem)}.interview.json"
    state = {
        "source_json": str(json_path),
        "output": str(final_path),
        "segments": len(data.get("segments", [])),
        "turns": len(turns),
    }
    save_json(state_path, state)
    return out_path


def main() -> int:
    parser = argparse.ArgumentParser(description="Generate readable interview-style Markdown from segments JSON.")
    parser.add_argument("json_file", nargs="?", type=Path, help="Path to *.segments.json. Defaults to all files in work/internal/json/.")
    parser.add_argument("--final-only", action="store_true", help="Only write output/*.md and skip legacy interview output.")
    args = parser.parse_args()

    files = [args.json_file] if args.json_file else sorted(OUT_JSON.glob("*.segments.json"))
    if not files:
        print(f"No segments JSON files found in {OUT_JSON}")
        return 0

    for path in files:
        out = process_json(path, final_only=args.final_only)
        print(out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
