from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import polish_interview_markdown as pim  # noqa: E402
import transcribe_podcasts as tp  # noqa: E402
from podcast_transcriber.language import (  # noqa: E402
    assign_language_classes,
    classify_segment_language,
    segment_needs_translation,
)


def test_classify_chinese_english_mixed_and_short() -> None:
    assert classify_segment_language("大家好，欢迎收听本期节目") == "zh"
    assert classify_segment_language("Welcome back to the show everyone") == "en"
    assert classify_segment_language("今天我们 talk about AI 的未来") == "mixed"
    assert classify_segment_language("ok") is None  # too short
    assert classify_segment_language("https://example.com/path") is None


def test_assign_language_classes_inherits_neighbors() -> None:
    segments = [
        {"id": 1, "text": "This is a full English sentence about models."},
        {"id": 2, "text": "ok"},
        {"id": 3, "text": "Another English paragraph continues here."},
        {"id": 4, "text": "这是一段完整的中文说明内容。"},
    ]
    assign_language_classes(segments, "en")
    assert segments[0]["languageClass"] == "en"
    assert segments[1]["languageClass"] == "en"  # inherited
    assert segments[2]["languageClass"] == "en"
    assert segments[3]["languageClass"] == "zh"


def test_force_translate_false_disables_service(monkeypatch) -> None:
    monkeypatch.setenv("PODCAST_TRANSCRIBER_FORCE_TRANSLATE", "0")
    config = {"translation": {"enabled": True, "backend": "deepseek", "auto_when_detected_languages": ["en"]}}
    segments = [{"text": "Hello world from the podcast studio.", "languageClass": "en"}]
    assert tp.is_translation_enabled(config) is False
    assert tp.should_translate_for_language(config, "en", segments) is False


def test_force_translate_true_skips_pure_chinese(monkeypatch) -> None:
    monkeypatch.setenv("PODCAST_TRANSCRIBER_FORCE_TRANSLATE", "1")
    config = {"translation": {"enabled": False, "backend": "deepseek"}}
    zh_only = [{"text": "这是一段纯中文播客内容，不需要翻译。", "languageClass": "zh"}]
    assert tp.is_translation_enabled(config) is True
    assert tp.should_translate_for_language(config, "zh", zh_only) is False
    mixed = [
        {"text": "这是中文段落。", "languageClass": "zh"},
        {"text": "This is English and needs translation.", "languageClass": "en"},
    ]
    assert tp.should_translate_for_language(config, "zh", mixed) is True


def test_chinese_blocks_never_count_as_missing_translation() -> None:
    segments = [
        {"text": "这是中文。", "languageClass": "zh"},
        {"text": "English needs a translation.", "languageClass": "en"},
    ]
    assert tp.has_missing_translations(segments) is True
    segments[1]["translation"] = "英文需要翻译。"
    assert tp.has_missing_translations(segments) is False
    assert segment_needs_translation(segments[0]) is False


def test_final_markdown_zh_en_mixed_and_chapter_headers() -> None:
    data = {"source_file": "mixed-show.mp3", "detected_language": "en"}
    turns = [
        {
            "speaker": "主持人",
            "start": 5.0,
            "end": 12.0,
            "original": "Welcome to the show.",
            "translation": "欢迎收听本期节目。",
            "is_sponsor": False,
            "needs_polish": False,
            "languageClass": "en",
        },
        {
            "speaker": "嘉宾",
            "start": 20.0,
            "end": 28.0,
            "original": "今天我们聊聊技术。",
            "translation": "",
            "is_sponsor": False,
            "needs_polish": False,
            "languageClass": "zh",
        },
        {
            "speaker": "主持人",
            "start": 610.0,
            "end": 620.0,
            "original": "Next chapter continues here.",
            "translation": "下一章从这里继续。",
            "is_sponsor": False,
            "needs_polish": False,
            "languageClass": "en",
        },
    ]
    config = {"markdown": {"llm_polish": {"enabled": False}, "fail_on_quality_errors": True}}
    markdown = pim.render_final_markdown(data, turns, config)

    assert markdown.startswith("# mixed-show\n")
    assert "### 00:00:00" in markdown
    assert "### 00:10:00" in markdown
    # English before Chinese for en blocks.
    en_pos = markdown.index("Welcome to the show.")
    zh_pos = markdown.index("欢迎收听本期节目。")
    assert en_pos < zh_pos
    assert "今天我们聊聊技术。" in markdown
    assert "Original:" not in markdown
    assert "Translation:" not in markdown
    assert "[TRANSLATION_MISSING]" not in markdown
    assert "翻译缺失" not in markdown
    assert "**采访者" not in markdown
    assert "source_path" not in markdown


def test_final_markdown_pure_chinese_no_missing_marker() -> None:
    data = {"source_file": "chinese-only.mp3", "detected_language": "zh"}
    turns = [
        {
            "speaker": "主持人",
            "start": 0.0,
            "end": 4.0,
            "original": "大家好，欢迎收听。",
            "translation": "",
            "is_sponsor": False,
            "needs_polish": False,
            "languageClass": "zh",
        }
    ]
    config = {"markdown": {"llm_polish": {"enabled": False}}}
    markdown = pim.render_final_markdown(data, turns, config)
    assert "大家好，欢迎收听。" in markdown
    assert "### 00:00:00" in markdown
    assert "翻译缺失" not in markdown


def test_merge_does_not_cross_language_or_ten_minute_boundary() -> None:
    turns = [
        {
            "speaker": "主持人",
            "start": 0.0,
            "end": 2.0,
            "original": "Hello there.",
            "translation": "你好。",
            "languageClass": "en",
            "is_sponsor": False,
            "needs_polish": False,
        },
        {
            "speaker": "主持人",
            "start": 3.0,
            "end": 5.0,
            "original": "中文段落。",
            "translation": "",
            "languageClass": "zh",
            "is_sponsor": False,
            "needs_polish": False,
        },
        {
            "speaker": "主持人",
            "start": 601.0,
            "end": 605.0,
            "original": "Later English.",
            "translation": "稍后英文。",
            "languageClass": "en",
            "is_sponsor": False,
            "needs_polish": False,
        },
    ]
    blocks = pim.merge_same_speaker_blocks(turns, max_combined_chars=2000)
    assert len(blocks) == 3
    assert [block["languageClass"] for block in blocks] == ["en", "zh", "en"]
