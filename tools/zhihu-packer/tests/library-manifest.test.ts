import assert from "node:assert/strict";
import test from "node:test";

import {
  buildZhihuManifest,
  inferOrphanChapter,
  type ArchivedItem,
} from "../src/library-manifest.ts";

const archivedItems: readonly ArchivedItem[] = [
  {
    id: "answer:1",
    authorId: "author-1",
    authorName: "作者",
    title: "标题含有 | 和 ] 不应丢失",
    createdTime: 1_720_000_000,
    voteCount: 99,
    outputPath: "output/作者/001.md",
  },
  {
    id: "article:2",
    authorId: "author-1",
    authorName: "作者",
    title: "第二篇",
    createdTime: 1_710_000_000,
    voteCount: 3,
    outputPath: "output/作者/002.md",
  },
];

test("builds an ordered manifest from successful database items", () => {
  const manifest = buildZhihuManifest({
    authorId: "author-1",
    authorName: "作者",
    generatedAt: "2026-07-10T00:00:00.000Z",
    items: archivedItems,
    inferredChapters: [],
  });

  assert.equal(manifest.bookId, "zhihu:author-1");
  assert.deepEqual(
    manifest.chapters.map((chapter) => chapter.id),
    ["answer:1", "article:2"],
  );
  assert.equal(manifest.chapters[0]?.title, "标题含有 | 和 ] 不应丢失");
  assert.equal(manifest.chapters[0]?.path, "001.md");
});

test("adds inferred orphan markdown without treating index.md as a chapter", () => {
  assert.equal(inferOrphanChapter("index.md", "# 索引"), null);
  const inferred = inferOrphanChapter("附录/2026-04-08-未匹配.md", "# 未匹配标题\n\n正文内容");
  assert.ok(inferred);
  assert.equal(inferred.path, "附录/2026-04-08-未匹配.md");
  assert.equal(inferred.title, "未匹配标题");
  assert.equal(inferred.date, "2026-04-08");
  assert.equal(inferred.metadataStatus, "inferred");
  assert.match(inferred.id, /^inferred:[a-f0-9]{16}$/);
});

test("deduplicates inferred files already represented by database items", () => {
  const inferred = inferOrphanChapter("001.md", "# 重复");
  assert.ok(inferred);
  const manifest = buildZhihuManifest({
    authorId: "author-1",
    authorName: "作者",
    generatedAt: "2026-07-10T00:00:00.000Z",
    items: archivedItems,
    inferredChapters: [inferred],
  });
  assert.equal(manifest.chapters.length, 2);
});
