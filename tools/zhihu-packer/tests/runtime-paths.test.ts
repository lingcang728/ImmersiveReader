import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveArchiveOutputDir,
  resolveBrowserCacheDir,
  resolveBrowserExecutable,
  resolveDatabasePath,
  resolveProfileDir,
} from "../src/runtime-paths.ts";

test("prefers explicit archive output then environment configuration", () => {
  assert.equal(
    resolveArchiveOutputDir({ cwd: "C:/tool", explicit: "D:/custom", environment: {} }),
    "D:\\custom",
  );
  assert.equal(
    resolveArchiveOutputDir({
      cwd: "C:/tool",
      environment: { IMMERSIVE_ZHIHU_OUTPUT: "D:/library/知乎" },
    }),
    "D:\\library\\知乎",
  );
});

test("derives the Zhihu shelf from the shared library root", () => {
  assert.equal(
    resolveArchiveOutputDir({
      cwd: "C:/tool",
      environment: { IMMERSIVE_LIBRARY_ROOT: "D:/library" },
    }),
    "D:\\library\\知乎",
  );
});

test("keeps legacy local paths when integration variables are absent", () => {
  assert.equal(resolveArchiveOutputDir({ cwd: "C:/tool", environment: {} }), "C:\\tool\\output");
  assert.equal(resolveDatabasePath({ cwd: "C:/tool", environment: {} }), "C:\\tool\\zhihu-packer.db");
  assert.equal(resolveProfileDir({ cwd: "C:/tool", environment: {} }), "C:\\tool\\.browser-profile");
  assert.equal(resolveBrowserCacheDir({ cwd: "C:/tool", environment: {} }), "C:\\tool\\.browser-cache");
});

test("keeps the managed profile and browser cache on their explicit roots", () => {
  assert.equal(
    resolveProfileDir({ cwd: "C:/tool", environment: { IMMERSIVE_ZHIHU_PROFILE: "D:/data/Private/ZhihuProfile" } }),
    "D:\\data\\Private\\ZhihuProfile",
  );
  assert.equal(
    resolveBrowserCacheDir({ cwd: "C:/tool", environment: { IMMERSIVE_ZHIHU_BROWSER_CACHE: "D:/cache/Zhihu/BrowserCache" } }),
    "D:\\cache\\Zhihu\\BrowserCache",
  );
});

test("uses the managed Chromium executable when configured", () => {
  assert.equal(
    resolveBrowserExecutable({ IMMERSIVE_CHROMIUM_EXECUTABLE: " C:/runtime/chromium/msedge.exe " }),
    "C:/runtime/chromium/msedge.exe",
  );
  assert.equal(resolveBrowserExecutable({}), undefined);
});
