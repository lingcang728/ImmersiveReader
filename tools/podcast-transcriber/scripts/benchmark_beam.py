"""beam_size 基准测试：用同一段音频分别以不同 beam_size 转写，对比速度与文本质量。

用法（把待测音频放到 input/ 或任意路径）：
    .\\.venv\\Scripts\\python.exe scripts\\benchmark_beam.py input\\样本.mp3
    .\\.venv\\Scripts\\python.exe scripts\\benchmark_beam.py input\\样本.mp3 --beams 2,3,5 --minutes 10

报告输出到 work/benchmark_beam_report.md，包含每档耗时与全文，便于逐段对比转写质量。
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))


def load_config() -> dict:
    return json.loads((ROOT / "config.json").read_text(encoding="utf-8-sig"))


def load_model(asr: dict):
    import transcribe_podcasts as tp
    from faster_whisper import WhisperModel

    tp.configure_nvidia_dll_paths()

    model_path = str(asr.get("model") or "large-v3-turbo")
    attempts = [("cuda", c) for c in asr.get("compute_type_preference") or ["int8_float16", "float16", "int8"]]
    attempts.append(("cpu", "int8"))
    last_exc: Exception | None = None
    for device, compute_type in attempts:
        try:
            model = WhisperModel(
                model_path,
                device=device,
                compute_type=compute_type,
                download_root=str(ROOT / "models"),
                cpu_threads=int(asr.get("cpu_threads") or 4),
            )
            print(f"模型加载成功: device={device}, compute_type={compute_type}")
            return model
        except Exception as exc:  # noqa: PERF203
            last_exc = exc
            print(f"加载失败 ({device}/{compute_type}): {exc}")
    raise RuntimeError(f"无法加载 Whisper 模型: {last_exc}")


def transcribe_with_beam(model, audio: Path, beam: int, limit_seconds: float) -> tuple[float, str, int]:
    started = time.perf_counter()
    segments, info = model.transcribe(str(audio), beam_size=beam, vad_filter=True)
    texts: list[str] = []
    for segment in segments:
        if segment.start > limit_seconds:
            break
        texts.append(segment.text.strip())
    elapsed = time.perf_counter() - started
    text = "\n".join(texts)
    return elapsed, text, len(texts)


def main() -> int:
    parser = argparse.ArgumentParser(description="beam_size 转写基准测试")
    parser.add_argument("audio", help="待测音频路径")
    parser.add_argument("--beams", default="2,3,5", help="逗号分隔的 beam_size 列表，默认 2,3,5")
    parser.add_argument("--minutes", type=float, default=10.0, help="每档只转写前 N 分钟（默认 10）")
    args = parser.parse_args()

    audio = Path(args.audio)
    if not audio.is_absolute():
        audio = ROOT / audio
    if not audio.exists():
        print(f"找不到音频文件: {audio}")
        return 1

    beams = [int(b) for b in args.beams.split(",") if b.strip()]
    limit_seconds = args.minutes * 60

    config = load_config()
    model = load_model(config.get("asr") or {})

    results = []
    for beam in beams:
        print(f"\n=== beam_size={beam}，转写前 {args.minutes} 分钟 ===")
        elapsed, text, segment_count = transcribe_with_beam(model, audio, beam, limit_seconds)
        chars = len(text.replace("\n", ""))
        print(f"耗时 {elapsed:.1f}s | 段数 {segment_count} | 字符数 {chars}")
        results.append({"beam": beam, "elapsed": elapsed, "segments": segment_count, "chars": chars, "text": text})

    report_lines = [
        "# beam_size 基准报告",
        "",
        f"- 音频: {audio.name}",
        f"- 范围: 前 {args.minutes} 分钟",
        f"- 模型: {config.get('asr', {}).get('model', '')}",
        "",
        "| beam_size | 耗时(s) | 段数 | 字符数 | 相对 beam=最小档 耗时 |",
        "|---|---|---|---|---|",
    ]
    base_elapsed = results[0]["elapsed"] if results else 1.0
    for r in results:
        report_lines.append(
            f"| {r['beam']} | {r['elapsed']:.1f} | {r['segments']} | {r['chars']} | {r['elapsed'] / base_elapsed:.2f}x |"
        )
    for r in results:
        report_lines += ["", f"## beam_size={r['beam']} 全文", "", r["text"], ""]

    report_path = ROOT / "work" / "benchmark_beam_report.md"
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text("\n".join(report_lines), encoding="utf-8")
    print(f"\n报告已写入: {report_path}")
    print("速度表在报告顶部；把各档全文拉到一起对比人名/术语/断句，质量差异一目了然。")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
