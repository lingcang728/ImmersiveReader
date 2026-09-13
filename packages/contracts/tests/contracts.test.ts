import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  ContractParseError,
  calculateOverallProgress,
  isSafeRelativePath,
  parseManifest,
  parseReadingState,
  resolveCurrent,
  validateReadingState,
} from "../src/index.ts";

import type { BookManifest } from "../src/index.ts";

// P3-29: the suite consumes the shared fixtures from disk — never inline
// copies — so the TS tests, the JSON schemas, and the Rust suite always run
// the same data. Canonical run: `node --test tests/*.test.ts` (identical test
// files in CI via the tsx loader); `tsc -p tsconfig.test.json` typechecks
// this file.

const loadFixture = (name: string): unknown =>
  JSON.parse(readFileSync(new URL(`../fixtures/${name}`, import.meta.url), "utf8"));

// Mutable view of a fixture for single-violation mutations.
type MutableManifest = {
  chapters?: Record<string, unknown>[];
  [key: string]: unknown;
};

const manifestFixture = (): MutableManifest =>
  structuredClone(loadFixture("manifest.valid.json")) as MutableManifest;

const readingFixture = (): Record<string, unknown> =>
  structuredClone(loadFixture("reading.valid.json")) as Record<string, unknown>;

const parseFixtureManifest = (): BookManifest => parseManifest(loadFixture("manifest.valid.json"));

test("parses the shared valid manifest fixtures without reordering chapters", () => {
  const manifest = parseFixtureManifest();
  assert.deepEqual(
    manifest.chapters.map((chapter) => chapter.id),
    ["answer:fixture-1"],
  );
  const minimal = parseManifest(loadFixture("manifest.valid.optional-omitted.json"));
  assert.equal(minimal.sourceId, undefined);
  assert.equal(minimal.chapters[0]?.date, undefined);
});

test("rejects duplicate chapter ids and unsafe paths", () => {
  const duplicate = manifestFixture();
  duplicate.chapters?.push({ ...duplicate.chapters[0] });
  assert.throws(() => parseManifest(duplicate), ContractParseError);

  const traversal = manifestFixture();
  traversal.chapters![0]!["path"] = "../secret.md";
  assert.throws(() => parseManifest(traversal), ContractParseError);

  const backslash = manifestFixture();
  backslash.chapters![0]!["path"] = "sub\\001.md";
  assert.throws(() => parseManifest(backslash), ContractParseError);
});

test("rejects fixtures JSON Schema cannot express", () => {
  // manifest.invalid.path-collision.json is intentionally NOT in
  // expectations.json: JSON Schema cannot express case-insensitive
  // cross-item path uniqueness, so registering it would fail the schema leg.
  assert.throws(
    () => parseManifest(loadFixture("manifest.invalid.path-collision.json")),
    ContractParseError,
  );
  // manifest.invalid.datetime-date.json is likewise excluded: the schema's
  // `format: date-time` check is lexical only (unlike `format: date`), so a
  // calendar-impossible date inside a date-time passes the schema leg while
  // TS and Rust both reject it.
  assert.throws(
    () => parseManifest(loadFixture("manifest.invalid.datetime-date.json")),
    ContractParseError,
  );

  const collision = manifestFixture();
  collision.chapters?.push({ ...collision.chapters[0], id: "answer:fixture-2", path: "001.MD" });
  assert.throws(() => parseManifest(collision), ContractParseError);
});

test("rejects unsupported schemas and incomplete manifests", () => {
  const upgraded = manifestFixture();
  upgraded["schemaVersion"] = 2;
  assert.throws(() => parseManifest(upgraded), ContractParseError);

  const incomplete = manifestFixture();
  Reflect.deleteProperty(incomplete, "title");
  assert.throws(() => parseManifest(incomplete), ContractParseError);
});

test("rejects duplicate reading ids and unknown fields", () => {
  const duplicate = readingFixture();
  duplicate["read"] = ["answer:fixture-1", "answer:fixture-1"];
  assert.throws(() => parseReadingState(duplicate), ContractParseError);

  const unknown = manifestFixture();
  unknown["unexpected"] = true;
  assert.throws(() => parseManifest(unknown), ContractParseError);
});

test("validates reading ids against the manifest fixture", () => {
  const manifest = parseFixtureManifest();
  const state = parseReadingState(loadFixture("reading.valid.json"));
  validateReadingState(state, manifest);
  // current is unread, position 0.5, one chapter → 0.5.
  assert.equal(calculateOverallProgress(manifest, state), 0.5);
});

test("rejects invalid progress values", () => {
  const invalid = readingFixture();
  invalid["position"] = 1.01;
  assert.throws(() => parseReadingState(invalid), ContractParseError);
});

test("rejects non-canonical dates and fractional counts", () => {
  const badDateTime = manifestFixture();
  badDateTime["generatedAt"] = "2026-07-10";
  assert.throws(() => parseManifest(badDateTime), ContractParseError);

  const fractional = manifestFixture();
  fractional.chapters![0]!["wordCount"] = 1.5;
  assert.throws(() => parseManifest(fractional), ContractParseError);

  // `Date.parse` rolls 2026-02-30 over to March — the validator must still
  // reject it as a calendar date (mirrors chrono::NaiveDate).
  const impossible = manifestFixture();
  impossible.chapters![0]!["date"] = "2026-02-30";
  assert.throws(() => parseManifest(impossible), ContractParseError);
});

test("resolveCurrent tolerates and repairs a dangling current like the Rust load path", () => {
  const manifest = parseFixtureManifest();
  const raw = readingFixture();
  raw["current"] = "removed:chapter";
  const state = parseReadingState(raw);

  // validateReadingState stays strict — same as Rust validate_reading.
  assert.throws(() => validateReadingState(state, manifest), ContractParseError);

  // Repair points at the first unread chapter and resets the position.
  const resolved = resolveCurrent(state, manifest);
  assert.equal(resolved.current, "answer:fixture-1");
  assert.equal(resolved.position, 0);
  assert.doesNotThrow(() => validateReadingState(resolved, manifest));

  // Everything read → falls back to the first chapter (Rust
  // `or_else(chapters.first())`).
  const allRead = parseReadingState({ ...raw, read: ["answer:fixture-1"] });
  assert.equal(resolveCurrent(allRead, manifest).current, "answer:fixture-1");

  // A live current is returned untouched.
  const untouched = resolveCurrent(parseReadingState(readingFixture()), manifest);
  assert.equal(untouched.current, "answer:fixture-1");
  assert.equal(untouched.position, 0.5);
});

test("overall progress converges on the Rust formula for dirty read arrays", () => {
  const twoChapter = manifestFixture();
  twoChapter.chapters?.push({
    id: "answer:fixture-2",
    path: "002.md",
    title: "第二篇",
    voteCount: 0,
    wordCount: 1,
  });
  const manifest = parseManifest(twoChapter);
  const base = parseReadingState(readingFixture());

  // Foreign ids in `read` never count: read = [ghost] → 0 completed +
  // current position 0.5 → 0.25 of two chapters.
  const foreign = { ...base, read: ["ghost:9"], position: 0.5 };
  assert.equal(calculateOverallProgress(manifest, foreign), 0.25);

  // `read` is a union of chapter ids: duplicates count once.
  const duplicated = { ...base, read: ["answer:fixture-1", "answer:fixture-1"], position: 0 };
  assert.equal(calculateOverallProgress(manifest, duplicated), 0.5);

  // A current chapter absent from the manifest contributes nothing.
  const ghostCurrent = { ...base, current: "ghost:9", position: 0.9, read: [] as string[] };
  assert.equal(calculateOverallProgress(manifest, ghostCurrent), 0);

  // A current chapter already in `read` is not double-counted.
  const alreadyRead = {
    ...base,
    current: "answer:fixture-1",
    position: 0.5,
    read: ["answer:fixture-1"],
  };
  assert.equal(calculateOverallProgress(manifest, alreadyRead), 0.5);
});

test("path safety mirrors the shared segment ruleset", () => {
  for (const path of [
    // Existing rules — same table as the Rust test.
    "",
    "   ",
    "/abs.md",
    "C:abs.md",
    "c:/abs.md",
    "sub\\001.md",
    "a\0b.md",
    "a//b.md",
    "./a.md",
    "a/./b.md",
    "..",
    "../a.md",
    "a/../b.md",
    "a/",
    // P3-29 segment rules: reserved device basenames (case-insensitive,
    // before the first `.`).
    "CON.md",
    "con",
    "aux.txt",
    "NUL",
    "com1.md",
    "lpt9/x.md",
    "a/PrN.md",
    "con.anything.md",
    // Segments ending in `.` or ` ` are silently renamed by Win32.
    "a.",
    "a/b.",
    "a ",
    "a/b ",
    // Win32-forbidden characters anywhere in a segment.
    "a:b.md",
    "a?b.md",
    "a*b.md",
    "a|b.md",
    "a<b.md",
    "a>b.md",
  ]) {
    assert.equal(isSafeRelativePath(path), false, `must reject ${JSON.stringify(path)}`);
  }
  for (const path of [
    "001.md",
    "sub/002.md",
    ".hidden/001.md",
    "第一篇 .md",
    "console.md",
    "com10.md",
    "lpt0.txt",
    "a.b.c.md",
    "sub dir/001.md",
  ]) {
    assert.equal(isSafeRelativePath(path), true, `must accept ${JSON.stringify(path)}`);
  }
});

type FixtureExpectation = {
  readonly fixture: string;
  readonly contract: "manifest" | "reading" | "provenance" | "publish-transaction";
  readonly expect: "valid" | "invalid";
};

// This package only ships manifest/reading validators — provenance and
// publish-transaction fixtures are verified by the JSON schema (via
// scripts/verify_contract_parity.py) and the Rust parity suite instead.
const TS_PARITY_CONTRACTS = new Set(["manifest", "reading"]);

// P1-22 parity: the Rust suite (contracts.rs::shared_fixtures_match_*) runs this
// exact table against the same fixtures — the two implementations can never
// drift on accept/reject verdicts.
const expectations = loadFixture("expectations.json") as FixtureExpectation[];

test("shared fixtures produce the same verdicts as schema and Rust", () => {
  const manifest = parseFixtureManifest();
  for (const { fixture, contract, expect } of expectations) {
    if (!TS_PARITY_CONTRACTS.has(contract)) {
      continue;
    }
    const act = () => {
      const data = loadFixture(fixture);
      if (contract === "manifest") {
        return parseManifest(data);
      }
      const state = parseReadingState(data);
      validateReadingState(state, manifest);
      return state;
    };
    if (expect === "valid") {
      assert.doesNotThrow(act, fixture);
    } else {
      assert.throws(act, ContractParseError, fixture);
    }
  }
});
