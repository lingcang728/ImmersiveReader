import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";


const template = fs.readFileSync(path.resolve("src/reader-template.html"), "utf-8");

test("exposes keyboard and dialog semantics in the Reader shell", () => {
  assert.match(template, /<button[^>]+id="menu-trigger"[^>]+aria-expanded="false"/);
  assert.match(template, /id="search-overlay"[^>]+role="dialog"[^>]+aria-modal="true"/);
  assert.match(template, /id="sidebar"[^>]+aria-hidden="true"[^>]+inert/);
  assert.match(template, /\.menu-trigger-btn:focus-visible/);
});

test("provides Night Lamp error and reduced motion states", () => {
  assert.match(template, /\.landing-content\s*\{/);
  assert.match(template, /@media \(prefers-reduced-motion: reduce\)/);
});
