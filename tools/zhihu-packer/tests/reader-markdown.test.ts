import assert from "node:assert/strict";
import test from "node:test";

import { cleanReaderMarkdown } from "../src/reader/core/markdown-cleaner.ts";

test("removes the archive title only when it matches the chapter title", () => {
  const markdown = [
    '<h1 id="answer-1">重复标题</h1>',
    '<div class="meta">作者 · 2026-04-08</div>',
    "",
    "正文第一段。",
  ].join("\n");

  assert.equal(cleanReaderMarkdown(markdown, "重复标题"), "正文第一段。");
});

test("keeps a leading h1 that does not match the chapter title", () => {
  const markdown = [
    '<h1 id="answer-1">正文里的一级标题</h1>',
    "",
    "正文第一段。",
  ].join("\n");

  assert.equal(
    cleanReaderMarkdown(markdown, "章节标题"),
    '<h1 id="answer-1">正文里的一级标题</h1>\n\n正文第一段。',
  );
});

test("keeps ordinary markdown headings and removes YAML front matter", () => {
  const markdown = "---\ntitle: 示例\n---\n# 正文标题\n\n正文";

  assert.equal(cleanReaderMarkdown(markdown), "# 正文标题\n\n正文");
});
