from __future__ import annotations

import json
import os

READY_PROTOCOL_VERSION = 1


def resolve_sidecar_port(value: str | None = None) -> int:
    raw = os.environ.get("IMMERSIVE_SIDECAR_PORT", "0") if value is None else value
    if not raw.strip().isdigit():
        raise ValueError(f"Invalid sidecar port: {raw}")
    port = int(raw)
    if port < 0 or port > 65535:
        raise ValueError(f"Invalid sidecar port: {raw}")
    return port


def ready_payload(engine: str, pid: int, port: int) -> dict[str, int | str]:
    if not engine or pid <= 0 or port <= 0 or port > 65535:
        raise ValueError("READY payload requires a positive PID and bound port")
    return {
        "engine": engine,
        "protocolVersion": READY_PROTOCOL_VERSION,
        "pid": pid,
        "port": port,
    }


def format_ready_line(engine: str, pid: int, port: int) -> str:
    return json.dumps(ready_payload(engine, pid, port), separators=(",", ":")) + "\n"


def write_ready(engine: str, pid: int, port: int) -> None:
    print(format_ready_line(engine, pid, port), end="", flush=True)
