"""Three-sided contract parity check.

Every fixture listed in ``packages/contracts/fixtures/expectations.json`` and
``settings-expectations.json`` is checked against:

  1. **schema** — the JSON schema in ``packages/contracts/schemas`` (via the
     ``jsonschema`` package, in this process),
  2. **TS** — the TypeScript validators in ``packages/contracts/src/index.ts``
     (via ``node``; falls back to the compiled ``dist/index.js`` when type
     stripping is unavailable),
  3. **Rust** — the same tables are consumed by the cargo suite
     (``contracts::tests::shared_fixtures_match_schema_and_ts_verdicts`` and
     ``settings::tests::shared_settings_fixtures_match_schema_verdicts``), so
     any verdict drift in the Rust implementation fails there. Pass
     ``--with-rust`` to also run those tests from here.

The tables include negative fixtures (unsafe paths, explicit nulls, missing
required fields, non-canonical dates, unknown fields, non-integer numerics),
not just valid samples. ``provenance``/``publish-transaction`` fixtures run
the schema + Rust legs only — the TS library has no parser for them, and the
Rust readers are intentionally more tolerant than the schemas (all-Option
fields, no ``deny_unknown_fields``) so old journals still load; only fixtures
where every leg's verdict agrees are listed.

``publication``/``reader-locator`` (EPUB) run all three legs — JSON schema,
TS ``parsePublication``/``parseReaderLocator``, and Rust
``serde_json`` + ``validate_publication``/``validate_reader_locator`` inside
the same parity test. Cross-field rules a schema cannot express (nav
chapterId ⊆ spine) are covered by dedicated Rust/TS unit tests rather than
fixture rows.

Cross-process task contracts (01-F5) are covered the same way:

  - ``task-event`` — the ``acquisition://task-event`` wire shape; the Rust
    leg deserializes ``crate::tasks::TaskEvent``.
  - ``worker-fatal`` — the podcast worker's terminal fatal NDJSON line. On
    top of the fixture table, ``run_worker_fatal_producer_leg`` imports the
    real emitter (``transcribe_task._exit_fatal_payload``) and validates its
    payloads against the schema, so a producer that drifts fails here even
    when no fixture changed.

Exit code is non-zero on any disagreement.
"""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
from pathlib import Path

from jsonschema import Draft202012Validator, FormatChecker


ROOT = Path(__file__).resolve().parents[1]
CONTRACTS_ROOT = ROOT / "packages" / "contracts"
SCHEMA_ROOT = CONTRACTS_ROOT / "schemas"
FIXTURE_ROOT = CONTRACTS_ROOT / "fixtures"
CONTRACT_SCHEMA = {
    "manifest": "manifest.schema.json",
    "reading": "reading.schema.json",
    "publication": "publication.schema.json",
    "reader-locator": "reader-locator.schema.json",
}
# Settings fixtures are covered by schema + Rust legs only — the TS library
# has no settings parser. The four CONTRACT_SCHEMA contracts all have TS
# validators (parseManifest / parseReadingState / parsePublication /
# parseReaderLocator), so they also run the TS leg.
TS_CONTRACTS = frozenset(CONTRACT_SCHEMA)


def load_json(path: Path) -> object:
    return json.loads(path.read_text(encoding="utf-8"))


def validator(path: Path) -> Draft202012Validator:
    return Draft202012Validator(load_json(path), format_checker=FormatChecker())


def schema_verdict(schema: Path, fixture: Path) -> bool:
    """True when the fixture validates against the schema."""
    return not list(validator(schema).iter_errors(load_json(fixture)))


TS_RUNNER = """
import { readFileSync } from "node:fs";
const [moduleUrl, fixturesDir, entriesJson] = process.argv.slice(1);
const {
  parseManifest,
  parsePublication,
  parseReaderLocator,
  parseReadingState,
  validateReadingState,
} = await import(moduleUrl);
const manifest = parseManifest(
  JSON.parse(readFileSync(`${fixturesDir}/manifest.valid.json`, "utf8")),
);
const verdicts = {};
for (const entry of JSON.parse(entriesJson)) {
  try {
    const data = JSON.parse(readFileSync(`${fixturesDir}/${entry.fixture}`, "utf8"));
    if (entry.contract === "manifest") {
      parseManifest(data);
    } else if (entry.contract === "publication") {
      parsePublication(data);
    } else if (entry.contract === "reader-locator") {
      parseReaderLocator(data);
    } else {
      const state = parseReadingState(data);
      validateReadingState(state, manifest);
    }
    verdicts[entry.fixture] = "valid";
  } catch {
    verdicts[entry.fixture] = "invalid";
  }
}
console.log(JSON.stringify(verdicts));
"""


def run_ts_verdicts(entries: list[dict]) -> dict[str, str]:
    """Return fixture -> 'valid'|'invalid' verdicts from the TS validators."""
    node = shutil.which("node")
    if node is None:
        raise RuntimeError("node executable not found on PATH")
    fixtures_arg = FIXTURE_ROOT.as_posix()
    entries_arg = json.dumps(entries)
    candidates = [
        CONTRACTS_ROOT / "src" / "index.ts",
        CONTRACTS_ROOT / "dist" / "index.js",
    ]
    errors = []
    for candidate in candidates:
        if not candidate.is_file():
            errors.append(f"{candidate.name}: missing")
            continue
        result = subprocess.run(
            [
                node,
                "--input-type=module",
                "-e",
                TS_RUNNER,
                candidate.resolve().as_uri(),
                fixtures_arg,
                entries_arg,
            ],
            capture_output=True,
            text=True,
            cwd=ROOT,
            check=False,
        )
        if result.returncode == 0:
            return json.loads(result.stdout)
        errors.append(
            f"{candidate.name}: {result.stderr.strip() or result.stdout.strip()}"
        )
    raise RuntimeError("TS validator leg failed: " + "; ".join(errors))


def run_worker_fatal_producer_leg() -> list[str]:
    """Validate the real fatal-line emitter against worker-fatal.schema.json.

    Fixtures alone cannot keep a *producer* honest — this leg imports the
    podcast worker's fatal builder and validates what it would actually
    print (the exit-code table and the unknown-code fallback).
    ``transcribe_task`` is import-safe: its heavy pipeline imports live
    inside ``main()``.
    """
    scripts_dir = ROOT / "tools" / "podcast-transcriber" / "scripts"
    if str(scripts_dir) not in sys.path:
        sys.path.insert(0, str(scripts_dir))
    try:
        import transcribe_task
    except Exception as error:  # report, don't crash the gate
        return [f"worker-fatal producer leg: transcribe_task import failed: {error}"]
    check = validator(SCHEMA_ROOT / "worker-fatal.schema.json")
    summary = {
        "results": [
            {"file": "ep1.mp3", "status": "failed", "error": "decoder exploded"},
            {"file": "ep2.mp3", "status": "ok"},
        ],
        "failures": ["normalize timed out"],
    }
    failures: list[str] = []
    for exit_code in (1, 2, 3, 4, 99):
        payload = transcribe_task._exit_fatal_payload(
            exit_code, summary, stage="transcribing"
        )
        errors = list(check.iter_errors(payload))
        if errors:
            failures.append(
                "worker-fatal producer leg: "
                f"_exit_fatal_payload({exit_code}) violates schema: "
                f"{errors[0].message}"
            )
    return failures


def run_rust_leg() -> bool:
    """Run the cargo parity tests that consume the same expectation tables."""
    cargo = shutil.which("cargo") or shutil.which("cargo.exe")
    if cargo is None:
        print("rust leg: cargo not found — skipped")
        return True
    manifest = ROOT / "apps" / "desktop" / "src-tauri" / "Cargo.toml"
    tests = [
        "shared_fixtures_match_schema_and_ts_verdicts",
        "shared_settings_fixtures_match_schema_verdicts",
    ]
    ok = True
    for test in tests:
        result = subprocess.run(
            [cargo, "test", "--manifest-path", str(manifest), test],
            cwd=ROOT,
            check=False,
        )
        if result.returncode != 0:
            print(f"rust leg: cargo test {test} failed")
            ok = False
    return ok


def main() -> int:
    with_rust = "--with-rust" in sys.argv[1:]
    expectations: list[dict] = load_json(FIXTURE_ROOT / "expectations.json")
    settings_expectations: list[dict] = load_json(
        FIXTURE_ROOT / "settings-expectations.json"
    )

    # Schema leg — every fixture in both tables.
    failures: list[str] = []
    schema_verdicts: dict[str, bool] = {}
    for entry in [*expectations, *settings_expectations]:
        fixture = entry["fixture"]
        schema_name = entry.get("schema") or CONTRACT_SCHEMA.get(entry["contract"])
        if schema_name is None:
            failures.append(f"{fixture}: unknown contract {entry.get('contract')}")
            continue
        verdict = schema_verdict(SCHEMA_ROOT / schema_name, FIXTURE_ROOT / fixture)
        schema_verdicts[fixture] = verdict
        expected = entry["expect"] == "valid"
        if verdict != expected:
            failures.append(
                f"{fixture}: schema verdict {'valid' if verdict else 'invalid'} "
                f"!= expected {entry['expect']}"
            )

    # TS leg — every manifest/reading fixture in expectations.json.
    ts_entries = [e for e in expectations if e["contract"] in TS_CONTRACTS]
    try:
        ts_verdicts = run_ts_verdicts(ts_entries)
    except RuntimeError as error:
        print(f"TS leg failed: {error}", file=sys.stderr)
        return 2
    for entry in ts_entries:
        fixture = entry["fixture"]
        expected = entry["expect"] == "valid"
        verdict = ts_verdicts.get(fixture) == "valid"
        if verdict != expected:
            failures.append(
                f"{fixture}: TS verdict {'valid' if verdict else 'invalid'} "
                f"!= expected {entry['expect']}"
            )
        elif fixture in schema_verdicts and schema_verdicts[fixture] != verdict:
            failures.append(f"{fixture}: schema and TS verdicts disagree")

    # Producer leg — the podcast worker's fatal emitter against its schema.
    # A fixture table can only prove the *spec*; this proves the code that
    # speaks it still does.
    producer_failures = run_worker_fatal_producer_leg()
    failures.extend(producer_failures)
    if not producer_failures:
        print("worker-fatal producer leg: _exit_fatal_payload payloads valid")

    for entry in [*expectations, *settings_expectations]:
        fixture = entry["fixture"]
        ts = ts_verdicts.get(fixture, "-") if entry.get("contract") in TS_CONTRACTS else "n/a"
        schema = (
            "valid" if schema_verdicts.get(fixture) else "invalid"
            if fixture in schema_verdicts
            else "?"
        )
        print(f"{fixture}: expect={entry['expect']} schema={schema} ts={ts}")

    if failures:
        for failure in failures:
            print(f"FAIL {failure}", file=sys.stderr)
        return 1

    rust_note = "deferred to cargo test (same tables)"
    if with_rust:
        if not run_rust_leg():
            return 1
        rust_note = "passed via cargo test"
    print(
        f"contract parity: {len(expectations) + len(settings_expectations)} fixtures "
        f"checked (schema + TS + expectations); rust leg {rust_note}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
