from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path


def test_managed_data_and_model_roots_override_source_paths(tmp_path: Path) -> None:
    data_root = tmp_path / "data"
    model_root = tmp_path / "models"
    environment = os.environ.copy()
    environment["IMMERSIVE_PODCAST_DATA_ROOT"] = str(data_root)
    environment["IMMERSIVE_PODCAST_MODEL_ROOT"] = str(model_root)
    script = (
        "from podcast_transcriber.common import CONFIG_PATH, INBOX, MODELS_DIR, OUTPUT, WORK;"
        "print(CONFIG_PATH);print(INBOX);print(OUTPUT);print(WORK);print(MODELS_DIR)"
    )

    result = subprocess.run(
        [sys.executable, "-c", script],
        cwd=Path(__file__).resolve().parents[1] / "scripts",
        env=environment,
        check=True,
        capture_output=True,
        text=True,
    )

    assert result.stdout.splitlines() == [
        str(data_root / "config.json"),
        str(data_root / "input"),
        str(data_root / "output"),
        str(data_root / "work"),
        str(model_root),
    ]


def test_managed_model_root_rehomes_absolute_config_references(tmp_path: Path) -> None:
    model_root = tmp_path / "models"
    (model_root / "local-model").mkdir(parents=True)
    environment = os.environ.copy()
    environment["IMMERSIVE_PODCAST_MODEL_ROOT"] = str(model_root)
    script = (
        "from transcribe_podcasts import asr_config;"
        "c={'asr':{'model':r'C:\\legacy\\models\\local-model',"
        "'fallback_models':[r'D:\\archive\\models\\backup-model','medium']}};"
        "a=asr_config(c);print(a['model']);print(*a['fallback_models'],sep='\\n')"
    )

    result = subprocess.run(
        [sys.executable, "-c", script],
        cwd=Path(__file__).resolve().parents[1] / "scripts",
        env=environment,
        check=True,
        capture_output=True,
        text=True,
    )

    assert result.stdout.splitlines() == [
        str(model_root / "local-model"),
        r"D:\archive\models\backup-model",
        "medium",
    ]
