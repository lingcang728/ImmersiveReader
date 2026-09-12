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
    # Logical ownership range in source coordinates: a transcribed segment is
    # attributed to this chunk only when source_start <= segment.start < source_end.
    source_start: float
    source_end: float
    # Physical audio span written to the chunk wav. May extend past the logical
    # range by the configured overlap so sentences spanning a boundary are
    # captured complete in at least one chunk.
    audio_start: float | None = None
    audio_end: float | None = None

    def resolved_audio_start(self) -> float:
        return self.source_start if self.audio_start is None else self.audio_start

    def resolved_audio_end(self) -> float:
        return self.source_end if self.audio_end is None else self.audio_end

    def as_dict(self) -> dict[str, Any]:
        data = {
            "index": self.index,
            "path": self.path,
            "source_start": round(self.source_start, 3),
            "source_end": round(self.source_end, 3),
        }
        audio_start = self.resolved_audio_start()
        audio_end = self.resolved_audio_end()
        if abs(audio_start - self.source_start) > 0.0005 or abs(audio_end - self.source_end) > 0.0005:
            data["audio_start"] = round(audio_start, 3)
            data["audio_end"] = round(audio_end, 3)
        return data


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
    def from_boundaries(
        cls,
        boundaries: list[float],
        overlap_seconds: float = 0.0,
        duration: float | None = None,
    ) -> ChunkPlan:
        """Build a plan from logical boundary times with optional audio overlap.

        ``boundaries`` must start at 0 and end at the source duration; each chunk
        physically covers ``[start - overlap, end + overlap]`` clamped to the
        source range while keeping logical ownership contiguous.
        """
        if len(boundaries) < 2:
            raise ValueError("Chunk boundaries must contain at least start and end")
        points = [round(float(point), 3) for point in boundaries]
        if points[0] < 0 or any(points[i] <= points[i - 1] for i in range(1, len(points))):
            raise ValueError("Chunk boundaries must be strictly increasing")
        overlap = max(0.0, float(overlap_seconds or 0.0))
        total = points[-1] if duration is None else float(duration)
        chunks: list[ChunkSpec] = []
        for index in range(len(points) - 1):
            source_start, source_end = points[index], points[index + 1]
            audio_start = max(0.0, source_start - overlap)
            audio_end = min(total, source_end + overlap)
            chunks.append(
                ChunkSpec(
                    index,
                    f"chunk_{index:05d}.wav",
                    source_start,
                    source_end,
                    round(audio_start, 3),
                    round(audio_end, 3),
                )
            )
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
            audio_start = raw.get("audio_start")
            audio_end = raw.get("audio_end")
            audio_start = start if audio_start is None else float(audio_start)
            audio_end = end if audio_end is None else float(audio_end)
            if audio_start < -0.01 or audio_end <= audio_start or audio_start > start + 0.01 or audio_end < end - 0.01:
                raise ValueError("Chunk plan audio coordinates must cover the logical range")
            chunks.append(ChunkSpec(expected_index, path, start, end, audio_start, audio_end))
            previous_end = end
        return cls(tuple(chunks))
