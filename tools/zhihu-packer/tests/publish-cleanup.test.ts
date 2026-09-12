import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  publishTaskStage,
  resetTaskIncoming,
  taskIncomingRoot,
} from "../src/publish.ts";

function stageAndPublish(root: string, taskId: string, files: Record<string, string>, items: any[]) {
  const incoming = resetTaskIncoming(root, taskId);
  const author = path.join(incoming, "作者");
  fs.mkdirSync(author, { recursive: true });
  for (const [name, content] of Object.entries(files)) {
    fs.writeFileSync(path.join(author, name), content);
  }
  return publishTaskStage(root, taskId, "author-1", { authorName: "作者", items });
}

test("P2-27①: commit removes the incoming shell and keeps only the newest rollback revision", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zhihu-publish-clean-"));
  try {
    const taskId = "task-clean";
    const first = stageAndPublish(root, taskId, { "a.md": "A" }, [{
      item_id: "answer:1",
      output_path: `.incoming/${taskId}/作者/a.md`,
      author_id: "author-1",
      author_name: "作者",
      title: "A",
      created_time: 1,
      voteup_count: 1,
    }]);
    assert.equal(first.transaction.revision, 1);
    // 第一次发布没有旧树，不产生 revision 目录；暂存壳已删。
    assert.equal(fs.existsSync(taskIncomingRoot(root, taskId)), false, "incoming shell removed after commit");

    // 第二次发布（同任务重跑）：旧树挪进 .revisions/author-1/2。
    const second = stageAndPublish(root, taskId, { "b.md": "B" }, [
      {
        item_id: "answer:1",
        output_path: "作者/a.md",
        author_id: "author-1",
        author_name: "作者",
        title: "A",
        created_time: 1,
        voteup_count: 1,
      },
      {
        item_id: "answer:2",
        output_path: `.incoming/${taskId}/作者/b.md`,
        author_id: "author-1",
        author_name: "作者",
        title: "B",
        created_time: 2,
        voteup_count: 2,
      },
    ]);
    assert.equal(second.transaction.revision, 2);
    assert.equal(fs.readFileSync(path.join(second.finalRoot, "a.md"), "utf8"), "A", "previous chapters merged forward");
    assert.equal(fs.readFileSync(path.join(second.finalRoot, "b.md"), "utf8"), "B");
    assert.equal(fs.existsSync(taskIncomingRoot(root, taskId)), false, "incoming shell removed again");

    // 第三次发布产生 revision 3 后，revision 2（最近一次回滚副本）保留，更早的删除。
    stageAndPublish(root, taskId, { "c.md": "C" }, [
      {
        item_id: "answer:1",
        output_path: "作者/a.md",
        author_id: "author-1",
        author_name: "作者",
        title: "A",
        created_time: 1,
        voteup_count: 1,
      },
      {
        item_id: "answer:2",
        output_path: "作者/b.md",
        author_id: "author-1",
        author_name: "作者",
        title: "B",
        created_time: 2,
        voteup_count: 2,
      },
      {
        item_id: "answer:3",
        output_path: `.incoming/${taskId}/作者/c.md`,
        author_id: "author-1",
        author_name: "作者",
        title: "C",
        created_time: 3,
        voteup_count: 3,
      },
    ]);
    const revisionsRoot = path.join(root, ".revisions", "author-1");
    const kept = fs.existsSync(revisionsRoot)
      ? fs.readdirSync(revisionsRoot, { withFileTypes: true })
          .filter(e => e.isDirectory() && /^\d+$/.test(e.name))
          .map(e => e.name)
      : [];
    assert.deepEqual(kept, ["3"], "only the newest rollback revision is retained");

    // 当前任务的 committed journal 必须保留（幂等重发/并入上次已发布树都靠它）。
    const currentJournal = path.join(root, ".transactions", `zhihu-${taskId}.json`);
    assert.equal(fs.existsSync(currentJournal), true, "current committed journal is kept");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("P2-27①: commit prunes other tasks' rolled_back journals but keeps live ones", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zhihu-publish-journal-"));
  try {
    const transactionsDir = path.join(root, ".transactions");
    fs.mkdirSync(transactionsDir, { recursive: true });
    const staleJournal = path.join(transactionsDir, "zhihu-stale-task.json");
    fs.writeFileSync(staleJournal, JSON.stringify({ schemaVersion: 1, taskId: "stale-task", phase: "rolled_back" }));
    const liveJournal = path.join(transactionsDir, "zhihu-live-task.json");
    fs.writeFileSync(liveJournal, JSON.stringify({ schemaVersion: 1, taskId: "live-task", phase: "committed" }));
    const notJson = path.join(transactionsDir, "notes.txt");
    fs.writeFileSync(notJson, "keep me");

    const taskId = "task-journal";
    const result = stageAndPublish(root, taskId, { "a.md": "A" }, [{
      item_id: "answer:1",
      output_path: `.incoming/${taskId}/作者/a.md`,
      author_id: "author-1",
      author_name: "作者",
      title: "A",
      created_time: 1,
      voteup_count: 1,
    }]);
    assert.equal(result.transaction.phase, "committed");

    assert.equal(fs.existsSync(staleJournal), false, "rolled_back journal of another task is pruned");
    assert.equal(fs.existsSync(liveJournal), true, "committed journal of another task is kept");
    assert.equal(fs.existsSync(notJson), true, "non-journal files are untouched");
    assert.equal(fs.existsSync(path.join(transactionsDir, `zhihu-${taskId}.json`)), true);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
