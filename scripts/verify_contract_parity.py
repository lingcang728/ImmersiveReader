from __future__ import annotations

import json
import sys
from pathlib import Path

from jsonschema import Draft202012Validator, FormatChecker


ROOT = Path(__file__).resolve().parents[1]
SCHEMA_ROOT = ROOT / "packages" / "contracts" / "schemas"
FIXTURE_ROOT = ROOT / "packages" / "contracts" / "fixtures"


def load_json(path: Path) -> object:
    return json.loads(path.read_text(encoding="utf-8"))


def validator(path: Path) -> Draft202012Validator:
    return Draft202012Validator(load_json(path), format_checker=FormatChecker())


def assert_valid(schema: Path, fixture: Path) -> None:
    errors = sorted(validator(schema).iter_errors(load_json(fixture)), key=lambda error: list(error.path))
    if errors:
        raise AssertionError(f"{fixture.name} should validate against {schema.name}: {errors[0].message}")


def assert_invalid(schema: Path, fixture: Path) -> None:
    if not list(validator(schema).iter_errors(load_json(fixture))):
        raise AssertionError(f"{fixture.name} should fail against {schema.name}")


def main() -> int:
    valid_cases = (
        ("manifest.schema.json", "manifest.valid.json"),
        ("reading.schema.json", "reading.valid.json"),
        ("settings.schema.json", "settings.v3.valid.json"),
        ("settings-legacy-v1.schema.json", "settings.legacy-v1.valid.json"),
        ("settings-legacy-v2.schema.json", "settings.legacy-v2.valid.json"),
    )
    invalid_cases = (
        ("manifest.schema.json", "manifest.invalid.unknown.json"),
        ("manifest.schema.json", "manifest.invalid.date.json"),
        ("manifest.schema.json", "manifest.invalid.empty.json"),
        ("reading.schema.json", "reading.invalid.duplicate.json"),
        ("reading.schema.json", "reading.invalid.unknown.json"),
    )
    for schema_name, fixture_name in valid_cases:
        assert_valid(SCHEMA_ROOT / schema_name, FIXTURE_ROOT / fixture_name)
    for schema_name, fixture_name in invalid_cases:
        assert_invalid(SCHEMA_ROOT / schema_name, FIXTURE_ROOT / fixture_name)
    print(f"contract schema parity: {len(valid_cases)} valid and {len(invalid_cases)} invalid fixtures passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
