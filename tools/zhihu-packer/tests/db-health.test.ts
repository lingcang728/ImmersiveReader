import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { DatabaseSync } from "node:sqlite";

import {
  closeDb,
  ensureDbHealthy,
  getTask,
  getTasks,
  initDb,
  probeDb,
  saveTask,
  transitionTaskStatus,
} from "../src/db.ts";

function freshDb(): { root: string; dbPath: string } {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zhihu-db-health-"));
  const dbPath = path.join(root, "zhihu.db");
  closeDb();
  initDb(dbPath);
  return { root, dbPath };
}

test("probeDb reports a freshly initialised database as healthy", () => {
  const { root } = freshDb();
  try {
    assert.equal(probeDb(), true);
    assert.equal(ensureDbHealthy(), true);
  } finally {
    closeDb();
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("P2-27⑤: corrupt database file is quarantined to .corrupt-* and rebuilt empty", () => {
  const { root, dbPath } = freshDb();
  try {
    saveTask({ id: "t-before-corruption", status: "pending" });
    assert.equal(getTask("t-before-corruption")?.status, "pending");
    closeDb();

    // 破坏主库文件，并放两个 WAL 侧文件验证它们会一起被隔离。
    fs.writeFileSync(dbPath, Buffer.from("definitely not a sqlite database".repeat(16)));
    fs.writeFileSync(`${dbPath}-wal`, Buffer.from("stale wal bytes"));
    fs.writeFileSync(`${dbPath}-shm`, Buffer.from("stale shm bytes"));

    assert.equal(ensureDbHealthy(), true, "self-heal must rebuild a usable database");
    assert.equal(probeDb(), true);

    const quarantined = fs.readdirSync(root).filter(f => f.startsWith("zhihu.db.corrupt-"));
    assert.ok(quarantined.some(f => !f.endsWith("-wal") && !f.endsWith("-shm")), "main file quarantined");
    assert.ok(quarantined.some(f => f.endsWith("-wal")), "WAL sibling quarantined too");
    assert.ok(quarantined.some(f => f.endsWith("-shm")), "SHM sibling quarantined too");

    assert.deepEqual(getTasks(), [], "rebuilt database starts with an empty task table");
    assert.equal(getTask("t-before-corruption"), null);
  } finally {
    closeDb();
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("P2-27⑤: pre-migration backup folds WAL into a restorable copy", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "zhihu-db-backup-"));
  const dbPath = path.join(root, "zhihu.db");
  try {
    // 造一个 user_version=0 的旧库：只有旧版 tasks 表 → initDb 触发迁移前备份。
    const legacy = new DatabaseSync(dbPath);
    legacy.exec("PRAGMA journal_mode = WAL");
    legacy.exec(`CREATE TABLE tasks (
      id TEXT, input_url TEXT, author_id TEXT, author_name TEXT,
      item_types TEXT, output_dir TEXT, sort_by TEXT, top_n INTEGER,
      status TEXT, total_count INTEGER, success_count INTEGER, failed_count INTEGER,
      created_at INTEGER, updated_at INTEGER
    )`);
    legacy.prepare("INSERT INTO tasks (id, status, created_at, updated_at) VALUES ('legacy-1', 'pending', 1, 1)").run();
    legacy.close();

    closeDb();
    initDb(dbPath);

    const backups = fs.readdirSync(root).filter(f => f.startsWith("zhihu.db.backup-") && !f.endsWith("-wal") && !f.endsWith("-shm"));
    assert.ok(backups.length >= 1, "migration must back up the database first");
    for (const backup of backups) {
      const restored = new DatabaseSync(path.join(root, backup));
      try {
        const rows = restored.prepare("SELECT id FROM tasks WHERE id = 'legacy-1'").all() as Array<{ id: string }>;
        assert.equal(rows.length, 1, `backup ${backup} must be a complete, untorn copy`);
      } finally {
        restored.close();
      }
    }
    assert.equal(getTask("legacy-1")?.status, "pending", "migrated database keeps the row");
  } finally {
    closeDb();
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("P2-27③: transitionTaskStatus is an atomic UPDATE that can never insert rows", () => {
  const { root } = freshDb();
  try {
    assert.equal(transitionTaskStatus("ghost", "paused", ["running", "pending"]), false);
    assert.equal(getTask("ghost"), null, "no stub row for missing tasks");

    saveTask({ id: "t-atomic", status: "pending" });
    assert.equal(transitionTaskStatus("t-atomic", "paused", ["running", "pending"]), true);
    assert.equal(getTask("t-atomic")?.status, "paused");
    // 状态门槛：paused → paused 不在 from 列表里，不命中。
    assert.equal(transitionTaskStatus("t-atomic", "paused", ["running", "pending"]), false);
    assert.equal(getTask("t-atomic")?.status, "paused");
    // 终态同样被拒。
    assert.equal(transitionTaskStatus("t-atomic", "cancelled", ["pending", "running"]), false);
    assert.equal(transitionTaskStatus("t-atomic", "cancelled", ["pending", "running", "paused"]), true);
    assert.equal(getTask("t-atomic")?.status, "cancelled");
  } finally {
    closeDb();
    fs.rmSync(root, { recursive: true, force: true });
  }
});
