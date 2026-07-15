import assert from "node:assert/strict";
import test from "node:test";

import {
  ContractParseError,
  calculateOverallProgress,
  parseManifest,
  parseReadingState,
  validateReadingState,
} from "../src/index.ts";

const validManifest = {
  schemaVersion: 1,
  bookId: "zhihu:author-1",
  title: "一本文集",
  source: "zhihu",
  sourceId: "author-1",
  generatedAt: "2026-07-10T00:00:00.000Z",
  updatedAt: "2026-07-10T00:00:00.000Z",
  chapters: [
    {
      id: "answer:1",
      path: "001.md",
      title: "第一篇",
      date: "2026-07-01",
      voteCount: 12,
      wordCount: 800,
    },
    {
      id: "answer:2",
      path: "sub/002.md",
      title: "第二篇",
      voteCount: 0,
      wordCount: 1200,
    },
  ],
};

test("parses a valid manifest without reordering chapters", () => {
  const manifest = parseManifest(validManifest);
  assert.deepEqual(
    manifest.chapters.map((chapter) => chapter.id),
    ["answer:1", "answer:2"],
  );
});

test("rejects duplicate chapter ids and unsafe paths", () => {
  const duplicate = structuredClone(validManifest);
  duplicate.chapters[1] = { ...duplicate.chapters[1], id: "answer:1" };
  assert.throws(() => parseManifest(duplicate), ContractParseError);

  const traversal = structuredClone(validManifest);
  traversal.chapters[0] = { ...traversal.chapters[0], path: "../secret.md" };
  assert.throws(() => parseManifest(traversal), ContractParseError);

  const backslash = structuredClone(validManifest);
  backslash.chapters[0] = { ...backslash.chapters[0], path: "sub\\001.md" };
  assert.throws(() => parseManifest(backslash), ContractParseError);
});

test("rejects unsupported schemas and incomplete manifests", () => {
  assert.throws(
    () => parseManifest({ ...validManifest, schemaVersion: 2 }),
    ContractParseError,
  );
  const incomplete = structuredClone(validManifest);
  Reflect.deleteProperty(incomplete, "title");
  assert.throws(() => parseManifest(incomplete), ContractParseError);
});

test("rejects duplicate reading ids and unknown fields", () => {
  assert.throws(
    () =>
      parseReadingState({
        schemaVersion: 1,
        current: "answer:2",
        position: 0.4,
        read: ["answer:1", "answer:1"],
        updated: "2026-07-10T01:00:00.000Z",
      }),
    ContractParseError,
  );
  assert.throws(
    () => parseManifest({ ...validManifest, unexpected: true }),
    ContractParseError,
  );
});

test("validates reading ids against the manifest", () => {
  const state = parseReadingState({
    schemaVersion: 1,
    current: "answer:2",
    position: 0.4,
    read: ["answer:1"],
    updated: "2026-07-10T01:00:00.000Z",
  });
  validateReadingState(state, parseManifest(validManifest));
  assert.equal(calculateOverallProgress(validManifest.chapters.length, state), 0.7);
});

test("rejects invalid progress values", () => {
  assert.throws(
    () =>
      parseReadingState({
        schemaVersion: 1,
        current: "answer:1",
        position: 1.01,
        read: [],
        updated: "2026-07-10T01:00:00.000Z",
      }),
    ContractParseError,
  );
});

test("rejects non-canonical dates and fractional counts", () => {
  assert.throws(
    () => parseManifest({ ...validManifest, generatedAt: "2026-07-10" }),
    ContractParseError,
  );
  const fractional = structuredClone(validManifest);
  fractional.chapters[0] = { ...fractional.chapters[0], wordCount: 1.5 };
  assert.throws(() => parseManifest(fractional), ContractParseError);
});
