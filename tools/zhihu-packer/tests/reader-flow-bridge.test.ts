import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";

const appSource = fs.readFileSync(path.resolve("src/reader/ui/app.ts"), "utf-8");
const templatePath = path.resolve("dist/reader-template.html");

test("reader scroll path posts versioned immersive-reader-flow activity messages", () => {
  assert.match(appSource, /notifyParentReadingActivity/);
  assert.match(appSource, /source:\s*['"]immersive-reader-flow['"]/);
  assert.match(appSource, /version:\s*1/);
  assert.match(appSource, /type:\s*['"]reading-activity['"]/);
  assert.match(appSource, /window\.parent\.postMessage/);
});

test("reading-activity is emitted inside the scroll RAF throttle", () => {
  // Ensure the bridge rides the existing RAF, not a separate unthrottled listener.
  const scrollBlock = appSource.slice(
    appSource.indexOf("window.addEventListener('scroll'"),
    appSource.indexOf("this.paletteInput.addEventListener")
  );
  assert.match(scrollBlock, /requestAnimationFrame/);
  assert.match(scrollBlock, /notifyParentReadingActivity/);
  assert.match(scrollBlock, /handleScrollThrottled/);
});

test("flow font-scale bridge shares clamp range and ctrl-wheel path", () => {
  assert.match(appSource, /type:\s*['"]font-scale-change['"]/);
  assert.match(appSource, /type === 'set-font-scale'/);
  assert.match(appSource, /fontScaleMin = 0\.8/);
  assert.match(appSource, /fontScaleMax = 1\.5/);
  assert.match(appSource, /fontScaleStep = 0\.05/);
  assert.match(appSource, /captureViewportAnchor/);
  assert.match(appSource, /passive:\s*false/);
  assert.match(appSource, /e\.ctrlKey/);
});

test("reader template constrains long paths urls and code inside cards", () => {
  const template = fs.readFileSync(path.resolve("src/reader-template.html"), "utf-8");
  const cardRule = template.match(/\.article-card\s*\{([^}]*)\}/)?.[1] ?? "";
  assert.match(template, /min-width:\s*0/);
  assert.match(template, /overflow-wrap:\s*anywhere/);
  assert.match(template, /\.article-body pre[\s\S]*overflow-x:\s*auto/);
  assert.match(cardRule, /overflow:\s*hidden/);
});

test("long article emphasis never scales the vertical axis or leaves stale animations", () => {
  const template = fs.readFileSync(path.resolve("src/reader-template.html"), "utf-8");
  const cardRule = template.match(/\.article-card\s*\{([^}]*)\}/)?.[1] ?? "";
  const activeCardRule = template.match(/\.article-card\.active\s*\{([^}]*)\}/)?.[1] ?? "";
  const activeMethod = appSource.slice(
    appSource.indexOf("private updateActiveArticle"),
    appSource.indexOf("private moveMenuBubble"),
  );

  assert.match(cardRule, /transform:\s*scaleX\(0\.98\)/);
  assert.match(activeCardRule, /transform:\s*scaleX\(1\.01\)/);
  assert.doesNotMatch(cardRule, /transform:\s*scale\(/);
  assert.doesNotMatch(activeCardRule, /transform:\s*scale\(/);
  assert.match(activeMethod, /prevActive[\s\S]*getAnimations\(\)[\s\S]*animation\.cancel\(\)/);
  assert.match(activeMethod, /translateY\(\$\{translateYVal\}px\) scaleX\(0\.985\)/);
  assert.match(activeMethod, /translateY\(0\) scaleX\(1\.01\)/);
});

test("compiled reader template includes the message bridge after compile-reader", () => {
  assert.ok(fs.existsSync(templatePath), "dist/reader-template.html should exist after compile-reader");
  const compiled = fs.readFileSync(templatePath, "utf-8");
  assert.match(compiled, /immersive-reader-flow/);
  assert.match(compiled, /reading-activity/);
  assert.match(compiled, /postMessage/);
  assert.match(compiled, /notifyParentReadingActivity/);
  assert.match(compiled, /font-scale-change/);
  assert.match(compiled, /set-font-scale/);
});
