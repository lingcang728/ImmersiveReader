from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))

from sidecar_protocol import (
    READY_PROTOCOL_VERSION,
    format_ready_line,
    has_bearer_token,
    ready_payload,
    resolve_sidecar_port,
)


def test_builds_versioned_ready_payload_with_dynamic_port() -> None:
    assert ready_payload("podcast", 4242, 43210) == {
        "engine": "podcast",
        "protocolVersion": READY_PROTOCOL_VERSION,
        "pid": 4242,
        "port": 43210,
    }
    assert json.loads(format_ready_line("podcast", 4242, 43210)) == ready_payload("podcast", 4242, 43210)
    assert format_ready_line("podcast", 4242, 43210).endswith("\n")


def test_accepts_os_assigned_port_zero_and_rejects_invalid_values() -> None:
    assert resolve_sidecar_port("0") == 0
    assert resolve_sidecar_port("43210") == 43210
    for value in ("-1", "65536", "not-a-port"):
        with pytest.raises(ValueError, match="port"):
            resolve_sidecar_port(value)


def test_accepts_only_the_exact_bearer_token() -> None:
    assert has_bearer_token("Bearer secret-token", "secret-token")
    assert not has_bearer_token("bearer secret-token", "secret-token")
    assert not has_bearer_token("Bearer secret-token-extra", "secret-token")
    assert not has_bearer_token(None, "secret-token")
