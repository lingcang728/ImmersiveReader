import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";
import { marked } from "marked";
import createDOMPurify from "dompurify";

import { cleanReaderMarkdown } from "../src/reader/core/markdown-cleaner.ts";
import { renderMarkdown } from "../src/reader/core/markdown-renderer.ts";

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

test("renders podcast translations first and keeps matching original ids", async () => {
  const dom = new JSDOM("<!doctype html><body></body>");
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    marked,
    DOMPurify: createDOMPurify(dom.window),
  });

  const wrapper = await renderMarkdown(
    [
      "The first original paragraph.",
      "",
      "第一段中文译文。",
      "",
      "The second original paragraph.",
      "",
      "第二段中文译文。",
    ].join("\n"),
    "demo",
    "demo.md",
    new Map(),
  );
  const translations = Array.from(wrapper.querySelectorAll("p.podcast-translation"));
  const originals = Array.from(wrapper.querySelectorAll("blockquote.podcast-original"));

  assert.equal(translations.length, 2);
  assert.equal(originals.length, 2);
  assert.equal(translations[0]?.dataset.bilingualId, originals[0]?.dataset.bilingualId);
  assert.equal(translations[1]?.dataset.bilingualId, originals[1]?.dataset.bilingualId);
  assert.ok(
    (translations[1]?.compareDocumentPosition(originals[0]!) ?? 0) &
      dom.window.Node.DOCUMENT_POSITION_FOLLOWING,
  );
});
