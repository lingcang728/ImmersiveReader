import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import {
  publishTaskStage,
  resetTaskIncoming,
  resolvePublishedTaskItemPath,
  taskIncomingRoot,
} from "../src/publish.ts";

test("publishes staged author content and preserves the previous revision", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zhihu-publish-"));
  const taskId = "task-1";
  const incoming = resetTaskIncoming(root, taskId);
  const author = path.join(incoming, "作者");
  fs.mkdirSync(author, { recursive: true });
  fs.writeFileSync(path.join(author, "new.md"), "new");

  const final = path.join(root, "作者");
  fs.mkdirSync(final, { recursive: true });
  fs.writeFileSync(path.join(final, "old.md"), "old");

  const result = publishTaskStage(root, taskId, "author-1");
  assert.equal(result.transaction.phase, "committed");
  assert.equal(fs.readFileSync(path.join(final, "new.md"), "utf8"), "new");
  assert.equal(fs.readFileSync(path.join(root, ".revisions", "author-1", "1", "old.md"), "utf8"), "old");
  assert.equal(fs.readFileSync(path.join(root, ".transactions", "zhihu-task-1.json"), "utf8").includes('"phase": "committed"'), true);
  assert.equal(resolvePublishedTaskItemPath(root, taskId, ".incoming/task-1/作者/new.md"), path.join(final, "new.md"));
  fs.rmSync(root, { recursive: true, force: true });
});

test("staged partial results stay isolated from the current archive", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zhihu-publish-partial-"));
  const taskId = "task-2";
  const incoming = resetTaskIncoming(root, taskId);
  const author = path.join(incoming, "作者");
  fs.mkdirSync(author, { recursive: true });
  fs.writeFileSync(path.join(author, "partial.md"), "partial");
  const final = path.join(root, "作者");
  fs.mkdirSync(final, { recursive: true });
  fs.writeFileSync(path.join(final, "stable.md"), "stable");

  assert.equal(fs.existsSync(path.join(final, "partial.md")), false);
  assert.equal(fs.readFileSync(path.join(final, "stable.md"), "utf8"), "stable");
  assert.equal(fs.existsSync(taskIncomingRoot(root, taskId)), true);
  fs.rmSync(root, { recursive: true, force: true });
});
