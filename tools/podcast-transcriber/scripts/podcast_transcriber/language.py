"""Per-segment language classification for mixed Chinese/English podcasts."""

from __future__ import annotations

import re
from typing import Any

LANGUAGE_CLASS_THRESHOLD = 0.65
_URL_RE = re.compile(r"https?://\S+|www\.\S+", re.IGNORECASE)
_NON_SCRIPT_RE = re.compile(
    r"[\d\s"  # digits / whitespace
    r"\u2000-\u206F"  # general punctuation
    r"\u3000-\u303F"  # CJK symbols
    r"\uFF00-\uFFEF"  # fullwidth forms
    r"!\"#$%&'()*+,\-./:;<=>?@\[\\\]^_`{|}~"
    r"。，、；：？！…—·「」『』（）【】《》〈〉]+"
)
_CJK_RE = re.compile(r"[\u3400-\u9fff\u3040-\u30ff\uac00-\ud7af]")
_LATIN_RE = re.compile(r"[A-Za-z]")


def strip_for_language_classification(text: str) -> str:
    cleaned = _URL_RE.sub(" ", text or "")
    cleaned = _NON_SCRIPT_RE.sub(" ", cleaned)
    return cleaned.strip()


def classify_segment_language(text: str) -> str | None:
    """Return zh / en / mixed, or None when the fragment is too short to decide."""
    sample = strip_for_language_classification(text)
    if not sample:
        return None
    cjk = len(_CJK_RE.findall(sample))
    latin = len(_LATIN_RE.findall(sample))
    total = cjk + latin
    if total == 0:
        return None
    # Extremely short fragments inherit later.
    if total < 4 and max(cjk, latin) < 3:
        return None
    cjk_ratio = cjk / total
    latin_ratio = latin / total
    if cjk_ratio >= LANGUAGE_CLASS_THRESHOLD:
        return "zh"
    if latin_ratio >= LANGUAGE_CLASS_THRESHOLD:
        return "en"
    if cjk >= 2 and latin >= 2:
        return "mixed"
    if cjk > latin:
        return "zh"
    if latin > cjk:
        return "en"
    return "mixed"


def normalize_file_language(detected_language: Any) -> str | None:
    lang = str(detected_language or "").strip().lower().split("-")[0]
    if lang in {"zh", "en"}:
        return lang
    return None


def assign_language_classes(
    segments: list[dict[str, Any]],
    detected_language: Any = None,
) -> list[dict[str, Any]]:
    """Attach recoverable languageClass on each segment (in place) and return them."""
    file_lang = normalize_file_language(detected_language)
    provisional: list[str | None] = []
    for segment in segments:
        existing = segment.get("languageClass")
        if existing in {"zh", "en", "mixed"}:
            provisional.append(str(existing))
            continue
        provisional.append(classify_segment_language(str(segment.get("text", ""))))

    # Inherit from neighbors for short / unknown fragments.
    for index, value in enumerate(provisional):
        if value is not None:
            continue
        left = next((provisional[i] for i in range(index - 1, -1, -1) if provisional[i]), None)
        right = next((provisional[i] for i in range(index + 1, len(provisional)) if provisional[i]), None)
        provisional[index] = left or right or file_lang or "en"

    for segment, language_class in zip(segments, provisional):
        segment["languageClass"] = language_class or file_lang or "en"
    return segments


def segment_needs_translation(segment: dict[str, Any]) -> bool:
    if not str(segment.get("text", "")).strip():
        return False
    language_class = segment.get("languageClass")
    if language_class not in {"zh", "en", "mixed"}:
        language_class = classify_segment_language(str(segment.get("text", ""))) or "en"
    return language_class in {"en", "mixed"}
