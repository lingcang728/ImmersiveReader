from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass
from typing import Any

CHUNK_PLAN_VERSION = 1


@dataclass(frozen=True)
class ChunkSpec:
    index: int
    path: str
    source_start: float
    source_end: float

    def as_dict(self) -> dict[str, Any]:
        return {
            "index": self.index,
            "path": self.path,
            "source_start": round(self.source_start, 3),
            "source_end": round(self.source_end, 3),
        }


@dataclass(frozen=True)
class ChunkPlan:
    chunks: tuple[ChunkSpec, ...]

    @property
    def version(self) -> int:
        return CHUNK_PLAN_VERSION

    def as_dict(self) -> dict[str, Any]:
        return {"version": self.version, "chunks": [chunk.as_dict() for chunk in self.chunks]}

    def signature(self) -> str:
        payload = json.dumps(self.as_dict(), ensure_ascii=False, separators=(",", ":"))
        return hashlib.sha256(payload.encode("utf-8")).hexdigest()[:16]

    @classmethod
    def from_durations(cls, durations: list[float]) -> ChunkPlan:
        if not durations or any(duration <= 0 for duration in durations):
            raise ValueError("Chunk durations must be positive")
        chunks: list[ChunkSpec] = []
        source_start = 0.0
        for index, duration in enumerate(durations):
            source_end = source_start + float(duration)
            chunks.append(
                ChunkSpec(index, f"chunk_{index:05d}.wav", round(source_start, 3), round(source_end, 3))
            )
            source_start = source_end
        return cls(tuple(chunks))

    @classmethod
    def from_metadata(cls, value: Any) -> ChunkPlan:
        if not isinstance(value, dict) or value.get("version") != CHUNK_PLAN_VERSION:
            raise ValueError("Unsupported chunk plan version")
        raw_chunks = value.get("chunks")
        if not isinstance(raw_chunks, list) or not raw_chunks:
            raise ValueError("Chunk plan must contain chunks")
        chunks: list[ChunkSpec] = []
        previous_end = 0.0
        for expected_index, raw in enumerate(raw_chunks):
            if not isinstance(raw, dict):
                raise ValueError("Chunk plan entry must be an object")
            path = raw.get("path")
            if not isinstance(path, str) or path != f"chunk_{expected_index:05d}.wav":
                raise ValueError("Chunk plan path is not canonical")
            start = float(raw.get("source_start"))
            end = float(raw.get("source_end"))
            if start < 0 or end <= start or abs(start - previous_end) > 0.01:
                raise ValueError("Chunk plan source coordinates are not contiguous")
            chunks.append(ChunkSpec(expected_index, path, start, end))
            previous_end = end
        return cls(tuple(chunks))
