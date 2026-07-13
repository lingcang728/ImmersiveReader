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

test("compiled reader template includes the message bridge after compile-reader", () => {
  assert.ok(fs.existsSync(templatePath), "dist/reader-template.html should exist after compile-reader");
  const compiled = fs.readFileSync(templatePath, "utf-8");
  assert.match(compiled, /immersive-reader-flow/);
  assert.match(compiled, /reading-activity/);
  assert.match(compiled, /postMessage/);
  assert.match(compiled, /notifyParentReadingActivity/);
});
