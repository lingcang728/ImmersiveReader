import assert from "node:assert/strict";
import test from "node:test";

import { renderReaderHtml, safeSerializeJson } from "../src/reader-builder.ts";

test("serializes injected reader data without closing the script element", () => {
  const serialized = safeSerializeJson([{ title: "</script><script>alert(1)</script>" }]);
  assert.doesNotMatch(serialized, /<\/script>/i);
  assert.match(serialized, /\\u003c/);
});

test("renders a reader with ordered article data and an escaped title", () => {
  const template = [
    "<html><head><title>沉浸式 Markdown 阅读器</title></head><body>",
    "<!-- ARTICLES_DOM_PLACEHOLDER -->",
    "<script id=\"articles-json\">/* ARTICLES_JSON_PLACEHOLDER */</script>",
    "</body></html>",
  ].join("");
  const html = renderReaderHtml(template, [
    {
      articleId: "answer:1",
      filename: "001.md",
      relativePath: "001.md",
      title: "标题 <一>",
      date: "2026-07-01",
      upvoteCount: 2,
      htmlContent: "<p>正文</p>",
      pinyinAbbr: "bty",
      author: "作者",
      summary: "正文",
      wordCount: 2,
      frontMatter: { path: "001.md" },
    },
  ], "作者文集");

  assert.match(html, /<title>作者文集<\/title>/);
  assert.match(html, /标题 &lt;一&gt;/);
  assert.match(html, /"articleId":"answer:1"/);
});
