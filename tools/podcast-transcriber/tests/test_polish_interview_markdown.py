from __future__ import annotations

import logging
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

import polish_interview_markdown as pim  # noqa: E402
import transcribe_podcasts as tp  # noqa: E402


def test_final_quality_counts_only_missing_en_translations() -> None:
    turns = [
        {
            "speaker": "主持人",
            "start": 0.0,
            "end": 8.0,
            "original": "This is one original block.",
            "translation": "这是第一段。\n\n这是同一个说话块里的第二段。",
            "is_sponsor": False,
            "needs_polish": False,
            "languageClass": "en",
        }
    ]
    rendered_blocks = pim.merge_same_speaker_blocks(turns)
    markdown = (
        "# sample\n\n"
        "### 00:00:00\n\n"
        "This is one original block.\n\n"
        "这是第一段。\n\n"
        "这是同一个说话块里的第二段。\n"
    )

    errors = pim.final_quality_errors(markdown, turns, True, 700, rendered_blocks=rendered_blocks)

    assert "Chinese and English block counts are badly mismatched" not in errors
    assert not any("missing translation" in error for error in errors)


def test_chinese_turn_builder_uses_chinese_speaker_rules() -> None:
    segments = [
        {"start": 0.0, "end": 1.0, "text": "大家好，我是肖文杰"},
        {"start": 1.0, "end": 2.0, "text": "我是小亚"},
        {"start": 2.0, "end": 4.0, "text": "最近有一家科技公司的表现非常抢眼"},
        {"start": 4.0, "end": 6.0, "text": "但是你今天这个标题是什么意思"},
        {"start": 6.0, "end": 9.0, "text": "没有在强行建立关联，就是这两家公司确实有一个重要关联"},
    ]

    turns = pim.build_turns(segments, "zh")
    speakers = {turn["speaker"] for turn in turns}

    assert "主持人" in speakers
    assert "嘉宾" in speakers
    assert speakers != {"说话人待校对"}
    assert all(len(turn["original"]) < 260 for turn in turns)
    assert all(turn.get("languageClass") == "zh" for turn in turns)


def test_disabled_deepseek_polish_does_not_require_api_key() -> None:
    data = {"source_file": "sample.wav", "detected_language": "zh"}
    turns = [
        {
            "speaker": "主持人",
            "start": 0.0,
            "end": 2.0,
            "original": "这是一段中文内容。",
            "translation": "",
            "is_sponsor": False,
            "needs_polish": True,
            "languageClass": "zh",
        }
    ]
    config = {
        "markdown": {
            "llm_polish": {
                "enabled": False,
                "backend": "deepseek",
                "base_url": "https://api.deepseek.com",
                "api_key": "",
                "api_key_env": "",
            }
        }
    }

    markdown = pim.render_final_markdown(data, turns, config)

    assert "这是一段中文内容。" in markdown
    assert "### 00:00:00" in markdown


def test_batch_polish_applies_to_english_translations(monkeypatch) -> None:
    def fake_deepseek(prompt, config, response_format=None):
        return '{"results":[{"id":0,"text":"批量润色后的中文译文。"}]}', {}, 0.01

    monkeypatch.setattr(pim, "deepseek_chat_completion", fake_deepseek)
    monkeypatch.setattr(pim, "record_deepseek_polish_usage", lambda *args, **kwargs: None)

    data = {"source_file": "english.wav", "detected_language": "en"}
    turns = [
        {
            "speaker": "主持人",
            "start": 0.0,
            "end": 3.0,
            "original": "This is the original.",
            "translation": "这是原始译文。",
            "is_sponsor": False,
            "needs_polish": True,
            "languageClass": "en",
        }
    ]
    config = {
        "markdown": {
            "llm_polish": {
                "enabled": True,
                "backend": "deepseek",
                "base_url": "https://api.deepseek.com",
                "api_key": "test-key",
                "only_suspect_blocks": False,
                "max_blocks_per_file": 1,
            }
        }
    }

    markdown = pim.render_final_markdown(data, turns, config)

    assert "This is the original." in markdown
    assert "批量润色后的中文译文。" in markdown
    en_pos = markdown.index("This is the original.")
    zh_pos = markdown.index("批量润色后的中文译文。")
    assert zh_pos < en_pos


def test_batch_polish_applies_to_chinese_originals(monkeypatch) -> None:
    def fake_deepseek(prompt, config, response_format=None):
        return '{"results":[{"id":0,"text":"批量润色后的中文原文。"}]}', {}, 0.01

    monkeypatch.setattr(pim, "deepseek_chat_completion", fake_deepseek)
    monkeypatch.setattr(pim, "record_deepseek_polish_usage", lambda *args, **kwargs: None)

    data = {"source_file": "chinese.wav", "detected_language": "zh"}
    turns = [
        {
            "speaker": "主持人",
            "start": 0.0,
            "end": 3.0,
            "original": "这是没有润色的中文原文。",
            "translation": "",
            "is_sponsor": False,
            "needs_polish": True,
            "languageClass": "zh",
        }
    ]
    config = {
        "markdown": {
            "llm_polish": {
                "enabled": True,
                "backend": "deepseek",
                "base_url": "https://api.deepseek.com",
                "api_key": "test-key",
                "only_suspect_blocks": False,
                "max_blocks_per_file": 1,
            }
        }
    }

    markdown = pim.render_final_markdown(data, turns, config)

    assert "批量润色后的中文原文。" in markdown


def test_final_markdown_plain_paragraphs_without_speaker_labels() -> None:
    data = {"source_file": "english.wav", "detected_language": "en"}
    turns = [
        {
            "speaker": "主持人",
            "start": 0.0,
            "end": 3.0,
            "original": "This is the original question.",
            "translation": "这是中文译文。",
            "is_sponsor": False,
            "needs_polish": False,
            "languageClass": "en",
        },
        {
            "speaker": "嘉宾",
            "start": 3.0,
            "end": 6.0,
            "original": "This is the answer.",
            "translation": "这是回答译文。",
            "is_sponsor": False,
            "needs_polish": False,
            "languageClass": "en",
        },
    ]
    config = {
        "markdown": {
            "llm_polish": {"enabled": False},
        }
    }

    markdown = pim.render_final_markdown(data, turns, config)

    assert "这是中文译文。" in markdown
    assert "This is the original question." in markdown
    assert "**采访者" not in markdown
    assert "**受访者" not in markdown
    assert "说话人待校对" not in markdown
    assert "podcast-original" in markdown


def test_speaker_labels_disabled_skips_speaker_role_quality_gate() -> None:
    turns = [
        {
            "speaker": "主持人",
            "start": float(i),
            "end": float(i) + 1,
            "original": f"Sentence {i}.",
            "translation": f"句子{i}。",
            "is_sponsor": False,
            "needs_polish": False,
            "languageClass": "en",
        }
        for i in range(5)
    ]
    rendered_blocks = pim.merge_same_speaker_blocks(turns)

    errors = pim.final_quality_errors(
        "# t\n\n### 00:00:00\n\nSentence 0.\n\n句子0。\n",
        turns,
        True,
        700,
        rendered_blocks=rendered_blocks,
        require_speaker_roles=False,
    )

    assert "speaker inference produced fewer than two speaker roles" not in errors


def test_write_final_markdown_from_json_injects_configured_semaphore(tmp_path, monkeypatch) -> None:
    scripts_dir = tmp_path / "scripts"
    scripts_dir.mkdir()
    json_path = tmp_path / "sample.segments.json"
    json_path.write_text("{}", encoding="utf-8")
    (scripts_dir / "polish_interview_markdown.py").write_text(
        "\n".join(
            [
                "received_semaphore = None",
                "",
                "def set_deepseek_semaphore(sem):",
                "    global received_semaphore",
                "    received_semaphore = sem",
                "",
                "def process_json(path, final_only=False):",
                "    assert final_only is True",
                "    assert received_semaphore == 'shared-semaphore'",
                "    return path.with_suffix('.md')",
                "",
            ]
        ),
        encoding="utf-8",
    )
    config = {"pipeline": {"max_deepseek_api_requests": 6}}
    seen: dict[str, object] = {}

    def fake_deepseek_api_semaphore(received_config):
        seen["config"] = received_config
        return "shared-semaphore"

    monkeypatch.setattr(tp, "ROOT", tmp_path)
    monkeypatch.setattr(tp, "deepseek_api_semaphore", fake_deepseek_api_semaphore)

    output = tp.write_final_markdown_from_json(str(json_path), logging.getLogger("test"), config)

    assert seen["config"] is config
    assert output == str(json_path.with_suffix(".md"))
