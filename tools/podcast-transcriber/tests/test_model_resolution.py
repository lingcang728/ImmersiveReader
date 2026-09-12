from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))

from podcast_transcriber import common  # noqa: E402


def _make_ct2_dir(root: Path, name: str) -> Path:
    directory = root / name
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "model.bin").write_bytes(b"weights")
    (directory / "config.json").write_text("{}", encoding="utf-8")
    return directory


def _models_root(tmp_path: Path, monkeypatch) -> Path:
    models = tmp_path / "models"
    models.mkdir(parents=True, exist_ok=True)
    monkeypatch.setenv("IMMERSIVE_PODCAST_MODEL_ROOT", str(models))
    monkeypatch.setattr(common, "MODELS_DIR", models)
    return models


def test_resolve_exact_basename_dir(tmp_path: Path, monkeypatch) -> None:
    models = _models_root(tmp_path, monkeypatch)
    local = _make_ct2_dir(models, "faster-whisper-large-v3-turbo-local")

    assert common.resolve_model_reference("faster-whisper-large-v3-turbo-local") == str(local)


def test_resolve_canonical_local_dir(tmp_path: Path, monkeypatch) -> None:
    models = _models_root(tmp_path, monkeypatch)
    local = _make_ct2_dir(models, "faster-whisper-large-v3-turbo-local")

    # The historical config value resolves to the vendored -local dir.
    assert common.resolve_model_reference("large-v3-turbo") == str(local)
    assert common.resolve_model_reference("faster-whisper-large-v3-turbo") == str(local)


def test_resolve_hf_cache_to_loadable_snapshot(tmp_path: Path, monkeypatch) -> None:
    models = _models_root(tmp_path, monkeypatch)
    _make_ct2_dir(models, "faster-whisper-large-v3-turbo-local")
    snapshot = _make_ct2_dir(
        models,
        "models--Systran--faster-whisper-large-v3/snapshots/edaa852ec7e1458",
    )

    # "large-v3" must reach the real large-v3 HF snapshot, not the turbo dir.
    assert common.resolve_model_reference("large-v3") == str(snapshot)
    assert common.resolve_model_reference("Systran/faster-whisper-large-v3") == str(snapshot)


def test_resolve_never_returns_hf_cache_root(tmp_path: Path, monkeypatch) -> None:
    models = _models_root(tmp_path, monkeypatch)
    # HF cache root without any snapshot content must not be returned —
    # it has no top-level model.bin and cannot be loaded directly.
    (models / "models--Systran--faster-whisper-medium").mkdir(parents=True)

    assert common.resolve_model_reference("medium") == "medium"


def test_resolve_substring_fallback_to_vendored_dir(tmp_path: Path, monkeypatch) -> None:
    models = _models_root(tmp_path, monkeypatch)
    local = _make_ct2_dir(models, "faster-whisper-large-v3-turbo-local")

    assert common.resolve_model_reference("turbo") == str(local)


def test_resolve_unresolvable_passes_through(tmp_path: Path, monkeypatch) -> None:
    _models_root(tmp_path, monkeypatch)
    _make_ct2_dir(tmp_path / "models", "faster-whisper-large-v3-turbo-local")

    assert common.resolve_model_reference("medium") == "medium"
    assert common.resolve_model_reference("") == ""


def test_resolve_skips_dirs_without_weights(tmp_path: Path, monkeypatch) -> None:
    models = _models_root(tmp_path, monkeypatch)
    # A "<name>-local" directory without model.bin is not loadable.
    (models / "tiny-local").mkdir(parents=True)

    assert common.resolve_model_reference("tiny") == "tiny"


def test_resolve_requires_managed_env(tmp_path: Path, monkeypatch) -> None:
    monkeypatch.delenv("IMMERSIVE_PODCAST_MODEL_ROOT", raising=False)
    monkeypatch.setattr(common, "MODELS_DIR", tmp_path)
    _make_ct2_dir(tmp_path, "faster-whisper-large-v3-turbo-local")

    # Without the managed model root env var the reference passes through.
    assert common.resolve_model_reference("large-v3-turbo") == "large-v3-turbo"
