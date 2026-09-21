import { DatabaseSync } from 'node:sqlite';
import * as fs from 'fs';
import * as path from 'path';
import { logger } from './utils.js';
import { resolveDatabasePath } from './runtime-paths.js';

const SCHEMA_VERSION = 4;
let db: DatabaseSync | null = null;
let transactionDepth = 0;
/** 最近一次 initDb 解析出的库路径：供懒加载重开与损坏自愈沿用同一路径。 */
let currentDbPath: string | null = null;

function cleanParams(params: any[]): any[] {
  return params.map(p => p === undefined ? null : p);
}

function normalizeStoredPath(outputPath: string | null | undefined, outputDir = 'output'): string | null {
  if (!outputPath) return null;
  const normalized = outputPath.replace(/\\/g, '/');
  if (!path.isAbsolute(outputPath)) return normalized;

  const outputBase = path.resolve(process.cwd(), outputDir);
  const relToOutput = path.relative(outputBase, outputPath);
  if (relToOutput && !relToOutput.startsWith('..') && !path.isAbsolute(relToOutput)) {
    return relToOutput.replace(/\\/g, '/');
  }

  return path.relative(process.cwd(), outputPath).replace(/\\/g, '/');
}

function backupDatabaseIfNeeded(database: DatabaseSync, dbPath: string, reason: string) {
  if (!fs.existsSync(dbPath)) return;
  const stamp = new Date().toISOString().replace(/[:.]/g, '-');
  const backupPath = `${dbPath}.backup-${stamp}`;
  // P2-27⑤：WAL 模式下只拷主文件会得到静默陈旧/撕裂的备份（宿主 P1-17 同类）。
  // 先 TRUNCATE checkpoint 把 -wal 折叠回主文件再拷贝；若 checkpoint 失败
  // （例如文件被占用），则把 -wal/-shm 一并拷走，保证备份成对可恢复。
  let checkpointed = false;
  try {
    database.exec('PRAGMA wal_checkpoint(TRUNCATE)');
    checkpointed = true;
  } catch (err) {
    logger.warn(`备份前 WAL checkpoint 失败，将连同 -wal/-shm 一起拷贝: ${err instanceof Error ? err.message : err}`);
  }
  fs.copyFileSync(dbPath, backupPath);
  if (!checkpointed) {
    for (const suffix of ['-wal', '-shm']) {
      const side = `${dbPath}${suffix}`;
      try {
        if (fs.existsSync(side)) fs.copyFileSync(side, `${backupPath}${suffix}`);
      } catch (err) {
        logger.warn(`拷贝 WAL 侧文件失败 ${side}: ${err instanceof Error ? err.message : err}`);
      }
    }
  }
  logger.info(`已在迁移前备份 SQLite 数据库 (${reason}): ${backupPath}`);
  pruneStampedSiblings(dbPath, '.backup-', MIGRATION_BACKUPS_KEPT);
}

function tableExists(database: DatabaseSync, table: string): boolean {
  const rows = database.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?").all(table);
  return rows.length > 0;
}

function createSchema(database: DatabaseSync) {
  database.exec(`
    CREATE TABLE IF NOT EXISTS tasks (
      id TEXT PRIMARY KEY NOT NULL,
      input_url TEXT NOT NULL DEFAULT '',
      author_id TEXT NOT NULL DEFAULT '',
      author_name TEXT NOT NULL DEFAULT '',
      item_types TEXT NOT NULL DEFAULT 'all' CHECK(item_types IN ('answers', 'articles', 'all')),
      output_dir TEXT NOT NULL DEFAULT 'output',
      sort_by TEXT NOT NULL DEFAULT 'time' CHECK(sort_by IN ('time', 'vote')),
      top_n INTEGER,
      status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending', 'running', 'paused', 'cancelled', 'success', 'failed', 'partial_success')),
      index_status TEXT NOT NULL DEFAULT 'pending' CHECK(index_status IN ('pending', 'running', 'complete')),
      index_completed_at INTEGER,
      total_count INTEGER NOT NULL DEFAULT 0,
      success_count INTEGER NOT NULL DEFAULT 0,
      failed_count INTEGER NOT NULL DEFAULT 0,
      index_checkpoint_json TEXT,
      source_reported_count INTEGER,
      discovered_count INTEGER NOT NULL DEFAULT 0,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS items (
      id TEXT PRIMARY KEY NOT NULL,
      item_type TEXT NOT NULL CHECK(item_type IN ('answer', 'article')),
      author_id TEXT NOT NULL DEFAULT '',
      author_name TEXT NOT NULL DEFAULT '',
      title TEXT NOT NULL DEFAULT '',
      answer_id TEXT,
      question_id TEXT,
      article_id TEXT,
      url TEXT NOT NULL DEFAULT '',
      question_url TEXT,
      created_time INTEGER NOT NULL DEFAULT 0,
      updated_time INTEGER NOT NULL DEFAULT 0,
      voteup_count INTEGER NOT NULL DEFAULT 0,
      comment_count INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS task_items (
      task_id TEXT NOT NULL,
      item_id TEXT NOT NULL,
      status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending', 'running', 'success', 'failed')),
      output_path TEXT,
      failure_code TEXT,
      error_message TEXT,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL,
      PRIMARY KEY (task_id, item_id),
      FOREIGN KEY (task_id) REFERENCES tasks(id) ON DELETE CASCADE,
      FOREIGN KEY (item_id) REFERENCES items(id) ON DELETE CASCADE
    );

    CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
    CREATE INDEX IF NOT EXISTS idx_task_items_task_status ON task_items(task_id, status);
    CREATE INDEX IF NOT EXISTS idx_items_author_created ON items(author_id, created_time DESC);

    CREATE TABLE IF NOT EXISTS archive_authors (
      author_id TEXT PRIMARY KEY NOT NULL,
      author_name TEXT NOT NULL DEFAULT '',
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS archive_items (
      item_id TEXT PRIMARY KEY NOT NULL,
      author_id TEXT NOT NULL,
      source_url TEXT NOT NULL DEFAULT '',
      current_revision INTEGER NOT NULL DEFAULT 0,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL,
      FOREIGN KEY (author_id) REFERENCES archive_authors(author_id)
    );

    CREATE TABLE IF NOT EXISTS archive_revisions (
      item_id TEXT NOT NULL,
      revision INTEGER NOT NULL,
      task_id TEXT NOT NULL,
      output_path TEXT NOT NULL,
      -- 仅 migrate-legacy-archive.ts 在迁移旧归档时填充；正常发布路径不写
      -- （哈希校验走 publish journal 的 metadata 比对，不靠这两列）。
      manifest_sha256 TEXT,
      provenance_sha256 TEXT,
      created_at INTEGER NOT NULL,
      PRIMARY KEY (item_id, revision),
      UNIQUE (item_id, output_path),
      FOREIGN KEY (item_id) REFERENCES archive_items(item_id) ON DELETE CASCADE
    );

    CREATE INDEX IF NOT EXISTS idx_archive_items_author ON archive_items(author_id, updated_at DESC);
  `);
}

function backfillArchiveCatalog(database: DatabaseSync) {
  database.exec(`
    INSERT OR IGNORE INTO archive_authors (author_id, author_name, created_at, updated_at)
    SELECT i.author_id, MAX(i.author_name), MIN(ti.created_at), MAX(ti.updated_at)
    FROM task_items ti
    JOIN items i ON i.id = ti.item_id
    WHERE ti.status = 'success' AND ti.output_path IS NOT NULL AND i.author_id != ''
    GROUP BY i.author_id;

    INSERT OR IGNORE INTO archive_items (item_id, author_id, source_url, current_revision, created_at, updated_at)
    SELECT i.id, i.author_id, i.url, 0, MIN(ti.created_at), MAX(ti.updated_at)
    FROM task_items ti
    JOIN items i ON i.id = ti.item_id
    WHERE ti.status = 'success' AND ti.output_path IS NOT NULL AND i.author_id != ''
    GROUP BY i.id;

    INSERT OR IGNORE INTO archive_revisions (item_id, revision, task_id, output_path, created_at)
    SELECT item_id, revision, task_id, output_path, updated_at
    FROM (
      SELECT ti.item_id, ti.task_id, ti.output_path, ti.updated_at,
             ROW_NUMBER() OVER (PARTITION BY ti.item_id ORDER BY ti.updated_at, ti.task_id) AS revision
      FROM task_items ti
      JOIN archive_items ai ON ai.item_id = ti.item_id
      WHERE ti.status = 'success' AND ti.output_path IS NOT NULL
    );

    UPDATE archive_items
    SET current_revision = COALESCE((
      SELECT MAX(ar.revision) FROM archive_revisions ar WHERE ar.item_id = archive_items.item_id
    ), 0);
  `);
}

function migrateToV1(database: DatabaseSync, absolutePath: string) {
  const versionRow = database.prepare('PRAGMA user_version').all()[0] as any;
  const version = Number(versionRow?.user_version || 0);
  const hasLegacyTables = tableExists(database, 'tasks') || tableExists(database, 'items') || tableExists(database, 'task_items');

  if (version >= 1 && hasLegacyTables) {
    createSchema(database);
    backfillArchiveCatalog(database);
    database.exec(`PRAGMA user_version = ${SCHEMA_VERSION}`);
    return;
  }

  if (hasLegacyTables) {
    backupDatabaseIfNeeded(database, absolutePath, `schema-v${version}-to-v${SCHEMA_VERSION}`);
  }

  database.exec('BEGIN IMMEDIATE');
  try {
    if (tableExists(database, 'task_items')) database.exec('ALTER TABLE task_items RENAME TO task_items_old');
    if (tableExists(database, 'items')) database.exec('ALTER TABLE items RENAME TO items_old');
    if (tableExists(database, 'tasks')) database.exec('ALTER TABLE tasks RENAME TO tasks_old');

    createSchema(database);

    if (tableExists(database, 'tasks_old')) {
      database.exec(`
        INSERT OR IGNORE INTO tasks (
          id, input_url, author_id, author_name, item_types, output_dir, sort_by, top_n,
          status, index_status, index_completed_at, total_count, success_count, failed_count,
          created_at, updated_at
        )
        SELECT
          COALESCE(NULLIF(id, ''), 'task_' || rowid),
          COALESCE(input_url, ''),
          COALESCE(author_id, ''),
          COALESCE(author_name, ''),
          CASE WHEN item_types IN ('answers', 'articles', 'all') THEN item_types ELSE 'all' END,
          COALESCE(NULLIF(output_dir, ''), 'output'),
          CASE WHEN sort_by IN ('time', 'vote') THEN sort_by ELSE 'time' END,
          top_n,
          CASE WHEN status IN ('pending', 'running', 'paused', 'cancelled', 'success', 'failed', 'partial_success') THEN status ELSE 'pending' END,
          CASE WHEN total_count > 0 THEN 'complete' ELSE 'pending' END,
          CASE WHEN total_count > 0 THEN COALESCE(updated_at, created_at, strftime('%s','now') * 1000) ELSE NULL END,
          COALESCE(total_count, 0),
          COALESCE(success_count, 0),
          COALESCE(failed_count, 0),
          COALESCE(created_at, strftime('%s','now') * 1000),
          COALESCE(updated_at, strftime('%s','now') * 1000)
        FROM tasks_old;
      `);
    }

    if (tableExists(database, 'items_old')) {
      database.exec(`
        INSERT OR IGNORE INTO items (
          id, item_type, author_id, author_name, title, answer_id, question_id, article_id,
          url, question_url, created_time, updated_time, voteup_count, comment_count
        )
        SELECT
          COALESCE(NULLIF(id, ''), item_type || ':' || rowid),
          CASE WHEN item_type IN ('answer', 'article') THEN item_type ELSE 'answer' END,
          COALESCE(author_id, ''),
          COALESCE(author_name, ''),
          COALESCE(title, ''),
          answer_id,
          question_id,
          article_id,
          COALESCE(url, ''),
          question_url,
          COALESCE(created_time, 0),
          COALESCE(updated_time, 0),
          COALESCE(voteup_count, 0),
          COALESCE(comment_count, 0)
        FROM items_old;
      `);
    }

    if (tableExists(database, 'task_items_old')) {
      database.exec(`
        INSERT OR IGNORE INTO task_items (
          task_id, item_id, status, output_path, failure_code, error_message, created_at, updated_at
        )
        SELECT
          ti.task_id,
          ti.item_id,
          CASE WHEN ti.status IN ('pending', 'running', 'success', 'failed') THEN ti.status ELSE 'pending' END,
          ti.output_path,
          ti.failure_code,
          ti.error_message,
          COALESCE(ti.created_at, strftime('%s','now') * 1000),
          COALESCE(ti.updated_at, strftime('%s','now') * 1000)
        FROM task_items_old ti
        JOIN tasks t ON t.id = ti.task_id
        JOIN items i ON i.id = ti.item_id;
      `);
    }

    if (tableExists(database, 'task_items_old')) database.exec('DROP TABLE task_items_old');
    if (tableExists(database, 'items_old')) database.exec('DROP TABLE items_old');
    if (tableExists(database, 'tasks_old')) database.exec('DROP TABLE tasks_old');

    backfillArchiveCatalog(database);
    database.exec(`PRAGMA user_version = ${SCHEMA_VERSION}`);
    database.exec('COMMIT');
  } catch (err) {
    database.exec('ROLLBACK');
    throw err;
  }

  migrateAbsoluteOutputPaths(database);
}

function migrateAbsoluteOutputPaths(database: DatabaseSync) {
  const rows = database.prepare(`
    SELECT ti.task_id, ti.item_id, ti.output_path, t.output_dir
    FROM task_items ti
    JOIN tasks t ON t.id = ti.task_id
    WHERE ti.output_path IS NOT NULL
  `).all() as any[];

  const stmt = database.prepare('UPDATE task_items SET output_path = ? WHERE task_id = ? AND item_id = ?');
  runInTransaction(() => {
    for (const row of rows) {
      const rel = normalizeStoredPath(row.output_path, row.output_dir || 'output');
      if (rel !== row.output_path) {
        stmt.run(rel, row.task_id, row.item_id);
      }
    }
  });
}

export interface Task {
  id: string;
  input_url: string;
  author_id: string;
  author_name: string;
  item_types: 'answers' | 'articles' | 'all';
  output_dir: string;
  sort_by: 'time' | 'vote';
  top_n: number | null;
  status: 'pending' | 'running' | 'paused' | 'cancelled' | 'success' | 'failed' | 'partial_success';
  index_status: 'pending' | 'running' | 'complete';
  index_completed_at: number | null;
  total_count: number;
  success_count: number;
  failed_count: number;
  /** JSON: { contentType, next, isEnd, pagesSeen, discovered, totals, lastHeartbeatAt } */
  index_checkpoint_json: string | null;
  source_reported_count: number | null;
  discovered_count: number;
  created_at: number;
  updated_at: number;
}

export interface IndexCheckpoint {
  contentType?: 'answers' | 'articles' | 'all';
  next?: string | null;
  isEnd?: boolean;
  pagesSeen?: number;
  discovered?: number;
  totals?: number | null;
  stage?: string;
  statusMessage?: string;
  lastHeartbeatAt?: number;
}

export interface Item {
  id: string;
  item_type: 'answer' | 'article';
  author_id: string;
  author_name: string;
  title: string;
  answer_id: string | null;
  question_id: string | null;
  article_id: string | null;
  url: string;
  question_url: string | null;
  created_time: number;
  updated_time: number;
  voteup_count: number;
  comment_count: number;
}

export interface TaskItem {
  task_id: string;
  item_id: string;
  status: 'pending' | 'running' | 'success' | 'failed';
  output_path: string | null;
  failure_code: string | null;
  error_message: string | null;
  created_at: number;
  updated_at: number;
}

export interface IndexItemInput {
  id: string;
  type: 'answer' | 'article';
  authorId: string;
  authorName: string;
  title: string;
  url: string;
  createdTime: number;
  updatedTime: number;
  voteupCount: number;
  commentCount: number;
  questionId?: string;
  questionUrl?: string;
}

function isProbablyCorruption(err: unknown): boolean {
  const message = String(err instanceof Error ? err.message : err).toLowerCase();
  return message.includes('not a database')
    || message.includes('malformed')
    || message.includes('corrupt')
    || message.includes('notadb');
}

/**
 * P2-27⑤：参照宿主 control.db 自愈模式（P1-16）——损坏的主库连同 WAL 侧文件
 * 一起隔离为 .corrupt-<ts>，让随后的 openAndMigrate 能在干净路径上重建空 schema。
 *
 * 顺序要点：句柄还活着时 SQLite 把 -wal/-shm 锁住（rename 会 EBUSY），但 copyFileSync
 * 仍能读 —— 先复制留证；随后 close 坏句柄（SQLite 会自动删掉侧文件并释放主库锁），
 * 最后把还活着的文件 rename 走。构造函数就抛错（无句柄）的场景同样走这条路。
 */
function quarantineCorruptDatabase(absolutePath: string, handle?: DatabaseSync | null): void {
  const stamp = new Date().toISOString().replace(/[:.]/g, '-');
  const quarantined = `${absolutePath}.corrupt-${stamp}`;
  // 1) close 前复制 WAL 侧文件留证（close 时 SQLite 会删除它们，之后就没机会拷了）。
  for (const suffix of ['-wal', '-shm', '-journal']) {
    const from = `${absolutePath}${suffix}`;
    if (!fs.existsSync(from)) continue;
    try {
      fs.copyFileSync(from, `${quarantined}${suffix}`);
    } catch (err) {
      logger.warn(`复制损坏数据库侧文件失败 ${from}: ${err instanceof Error ? err.message : err}`);
    }
  }
  // 2) 关闭坏句柄释放主库文件锁。
  if (handle) {
    try { handle.close(); } catch {}
  }
  // 3) 把仍在原位的文件移走：主库一定能 rename；侧文件若被 SQLite 删掉则跳过，
  //    若目标已存在（步骤1已复制）则直接删除源文件，确保不污染随后重建的新库。
  for (const suffix of ['', '-wal', '-shm', '-journal']) {
    const from = `${absolutePath}${suffix}`;
    if (!fs.existsSync(from)) continue;
    try {
      fs.renameSync(from, `${quarantined}${suffix}`);
    } catch {
      try {
        fs.rmSync(from, { force: true });
      } catch (err) {
        logger.warn(`隔离损坏数据库文件失败 ${from}: ${err instanceof Error ? err.message : err}`);
      }
    }
  }
  logger.error(`检测到 SQLite 数据库损坏，已隔离为 ${quarantined} 并重建空库（历史任务记录不再可见）。`);
  pruneStampedSiblings(absolutePath, '.corrupt-', CORRUPT_QUARANTINES_KEPT);
  pruneStampedSiblings(absolutePath, '.backup-', MIGRATION_BACKUPS_KEPT);
}

/** F8: `<db>.corrupt-<ts>` 证据与 `<db>.backup-<ts>` 迁移快照都要有保留上限，
 * 否则每次自愈/迁移都往数据目录里再堆一份整库。同戳文件（主文件 + -wal/-shm/
 * -journal 侧文件）视作一组整体裁剪。时间戳按 ISO 格式生成，字典序即时间序。 */
const CORRUPT_QUARANTINES_KEPT = 5;
const MIGRATION_BACKUPS_KEPT = 5;

function pruneStampedSiblings(dbPath: string, marker: '.corrupt-' | '.backup-', keep: number): void {
  const dir = path.dirname(dbPath);
  const base = path.basename(dbPath);
  let names: string[];
  try {
    names = fs.readdirSync(dir);
  } catch {
    return;
  }
  const groups = new Set<string>();
  for (const name of names) {
    if (!name.startsWith(`${base}${marker}`)) continue;
    groups.add(name.replace(/-(wal|shm|journal)$/, ''));
  }
  const stale = [...groups].sort().slice(0, Math.max(0, groups.size - keep));
  for (const group of stale) {
    for (const suffix of ['', '-wal', '-shm', '-journal']) {
      try {
        fs.rmSync(path.join(dir, `${group}${suffix}`), { force: true });
      } catch (err) {
        logger.warn(`清理旧${marker}文件失败 ${group}${suffix}: ${err instanceof Error ? err.message : err}`);
      }
    }
  }
}

function openAndMigrate(absolutePath: string): void {
  const handle = new DatabaseSync(absolutePath);
  // 必须先挂到模块级 db 再迁移：migrateToV1 → migrateAbsoluteOutputPaths →
  // runInTransaction → getDb() 需要拿到这个句柄；若迁移末尾才赋值，getDb 会
  // 递归 initDb 打开第二个连接并泄漏，导致 Windows 上 closeDb 后文件仍被锁。
  db = handle;
  try {
    handle.exec('PRAGMA busy_timeout = 5000');
    handle.exec('PRAGMA journal_mode = WAL');
    handle.exec('PRAGMA foreign_keys = ON');
    // F8: a database stamped by a NEWER build must not be migrated blindly —
    // this binary does not know that schema's invariants and writing into it
    // could corrupt rows. Refuse loudly instead of "helpfully" downgrading.
    const stampedRow = handle.prepare('PRAGMA user_version').all()[0] as any;
    const stamped = Number(stampedRow?.user_version || 0);
    if (stamped > SCHEMA_VERSION) {
      throw new Error(
        `DB_SCHEMA_TOO_NEW: database is schema v${stamped}, this build understands v${SCHEMA_VERSION} — update zhihu-packer instead of downgrading`,
      );
    }
    migrateToV1(handle, absolutePath);
    migrateToV3(handle, absolutePath);
    migrateToV4(handle, absolutePath);
    handle.exec('PRAGMA foreign_keys = ON');
  } catch (err) {
    // 失败的句柄不能留在模块级 db（调用方拿到坏句柄会连环报错），
    // 也不能留在半迁移状态占着文件。
    db = null;
    if (isProbablyCorruption(err)) {
      // 句柄还开着：quarantine 先复制 -wal/-shm 留证再关句柄做改名。
      quarantineCorruptDatabase(absolutePath, handle);
    } else {
      try { handle.close(); } catch {}
    }
    throw err;
  }
}

export function initDb(dbPath?: string) {
  if (db) return;

  const absolutePath = dbPath
    ? path.resolve(process.cwd(), dbPath)
    : resolveDatabasePath({ cwd: process.cwd(), environment: process.env });
  currentDbPath = absolutePath;
  logger.info(`正在初始化 SQLite 数据库: ${absolutePath}`);

  try {
    openAndMigrate(absolutePath);
  } catch (err) {
    // P2-27⑤：DB 损坏（file is not a database / malformed）不再是永久失败——
    // 隔离坏文件后重建空 schema，sidecar 保持可用；非损坏类错误原样上抛。
    if (!isProbablyCorruption(err)) throw err;
    // openAndMigrate 的 exec/迁移期失败已在内部完成隔离+关句柄（主文件已改名，
    // existsSync 为 false 时这里是幂等空操作）；new DatabaseSync 构造函数就
    // 抛错的场景没有句柄可走，需要在这里兜底隔离。
    if (fs.existsSync(absolutePath)) quarantineCorruptDatabase(absolutePath);
    openAndMigrate(absolutePath);
  }
}

/** Incremental columns for index checkpoints (v2 → v3). Idempotent by column presence. */
function migrateToV3(database: DatabaseSync, absolutePath: string) {
  if (!tableExists(database, 'tasks')) {
    database.exec(`PRAGMA user_version = ${SCHEMA_VERSION}`);
    return;
  }
  const columns = database.prepare('PRAGMA table_info(tasks)').all() as Array<{ name: string }>;
  const names = new Set(columns.map((c) => c.name));
  const needs =
    !names.has('index_checkpoint_json') ||
    !names.has('source_reported_count') ||
    !names.has('discovered_count');
  if (!needs) {
    database.exec(`PRAGMA user_version = ${SCHEMA_VERSION}`);
    return;
  }
  const versionRow = database.prepare('PRAGMA user_version').all()[0] as any;
  const version = Number(versionRow?.user_version || 0);
  backupDatabaseIfNeeded(database, absolutePath, `schema-v${version}-to-v3-index-checkpoint`);
  if (!names.has('index_checkpoint_json')) {
    database.exec('ALTER TABLE tasks ADD COLUMN index_checkpoint_json TEXT');
  }
  if (!names.has('source_reported_count')) {
    database.exec('ALTER TABLE tasks ADD COLUMN source_reported_count INTEGER');
  }
  if (!names.has('discovered_count')) {
    database.exec('ALTER TABLE tasks ADD COLUMN discovered_count INTEGER NOT NULL DEFAULT 0');
  }
  database.exec(`PRAGMA user_version = ${SCHEMA_VERSION}`);
  logger.info('数据库已升级到 schema v3（index_checkpoint / discovered_count）。');
}

/**
 * v3 → v4: tasks.status CHECK 增加 'cancelled' 终态。
 * SQLite 不支持修改 CHECK 约束，需要重建 tasks 表。
 * 通过 sqlite_master 的建表 SQL 判定是否需要重建，与 user_version 无关，
 * 因此对任意旧版本库都是幂等且可靠的。
 */
function migrateToV4(database: DatabaseSync, absolutePath: string) {
  if (!tableExists(database, 'tasks')) {
    database.exec(`PRAGMA user_version = ${SCHEMA_VERSION}`);
    return;
  }
  const def = database
    .prepare("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'tasks'")
    .all()[0] as { sql?: string } | undefined;
  if (typeof def?.sql === 'string' && def.sql.includes("'cancelled'")) {
    database.exec(`PRAGMA user_version = ${SCHEMA_VERSION}`);
    return;
  }

  const versionRow = database.prepare('PRAGMA user_version').all()[0] as any;
  backupDatabaseIfNeeded(database, absolutePath, `schema-v${Number(versionRow?.user_version || 0)}-to-v4-cancelled-status`);

  // legacy_alter_table=ON 防止 RENAME 重写 task_items 的外键指向旧表；
  // foreign_keys 必须在事务外关闭（事务内 PRAGMA 是 no-op）。
  database.exec('PRAGMA foreign_keys = OFF');
  database.exec('PRAGMA legacy_alter_table = ON');
  database.exec('BEGIN IMMEDIATE');
  try {
    database.exec('ALTER TABLE tasks RENAME TO tasks_v4_old');
    createSchema(database);
    database.exec(`
      INSERT INTO tasks (
        id, input_url, author_id, author_name, item_types, output_dir, sort_by, top_n,
        status, index_status, index_completed_at, total_count, success_count, failed_count,
        index_checkpoint_json, source_reported_count, discovered_count, created_at, updated_at
      )
      SELECT
        id,
        COALESCE(input_url, ''),
        COALESCE(author_id, ''),
        COALESCE(author_name, ''),
        CASE WHEN item_types IN ('answers', 'articles', 'all') THEN item_types ELSE 'all' END,
        COALESCE(NULLIF(output_dir, ''), 'output'),
        CASE WHEN sort_by IN ('time', 'vote') THEN sort_by ELSE 'time' END,
        top_n,
        CASE WHEN status IN ('pending', 'running', 'paused', 'cancelled', 'success', 'failed', 'partial_success') THEN status ELSE 'pending' END,
        CASE WHEN index_status IN ('pending', 'running', 'complete') THEN index_status ELSE 'pending' END,
        index_completed_at,
        COALESCE(total_count, 0),
        COALESCE(success_count, 0),
        COALESCE(failed_count, 0),
        index_checkpoint_json,
        source_reported_count,
        COALESCE(discovered_count, 0),
        COALESCE(created_at, strftime('%s','now') * 1000),
        COALESCE(updated_at, strftime('%s','now') * 1000)
      FROM tasks_v4_old;
    `);
    database.exec('DROP TABLE tasks_v4_old');
    database.exec(`PRAGMA user_version = ${SCHEMA_VERSION}`);
    database.exec('COMMIT');
  } catch (err) {
    database.exec('ROLLBACK');
    throw err;
  } finally {
    database.exec('PRAGMA legacy_alter_table = OFF');
    database.exec('PRAGMA foreign_keys = ON');
  }
  logger.info('数据库已升级到 schema v4（tasks.status 支持 cancelled 终态）。');
}

export function closeDb() {
  db?.close();
  db = null;
  transactionDepth = 0;
}

function getDb(): DatabaseSync {
  if (!db) {
    // 沿用最近一次的库路径（含测试注入路径）；从未初始化时才回退到环境解析。
    initDb(currentDbPath ?? undefined);
  }
  return db!;
}

/**
 * P2-27⑤：/health 的廉价 DB 探测 —— 真实触碰 schema（COUNT FROM tasks），
 * 「进程活着但库已损坏/缺表」不再被误报为 ok。
 */
export function probeDb(): boolean {
  try {
    getDb().prepare('SELECT COUNT(*) AS c FROM tasks').all();
    return true;
  } catch {
    return false;
  }
}

/**
 * P2-27⑤：DB 健康自愈入口。探测失败时先 close+重开：正常迁移会补齐缺失的
 * schema（不丢数据）；若 initDb 判定为文件损坏，内部已按 .corrupt-<ts> 隔离重建。
 * 返回 true 表示当前可以正常服务；false 交给 /health 报 503。
 */
export function ensureDbHealthy(): boolean {
  if (probeDb()) return true;
  try { closeDb(); } catch {}
  try { initDb(currentDbPath ?? undefined); } catch {}
  return probeDb();
}

export function runInTransaction<T>(fn: () => T): T {
  const database = getDb();
  if (transactionDepth > 0) return fn();

  transactionDepth++;
  database.exec('BEGIN IMMEDIATE');
  try {
    const result = fn();
    database.exec('COMMIT');
    return result;
  } catch (err) {
    database.exec('ROLLBACK');
    throw err;
  } finally {
    transactionDepth--;
  }
}

export function saveTask(task: Partial<Task> & { id: string }) {
  const database = getDb();
  const existing = getTask(task.id);

  const now = Date.now();
  if (!existing) {
    const stmt = database.prepare(`
      INSERT INTO tasks (
        id, input_url, author_id, author_name, item_types, output_dir,
        sort_by, top_n, status, index_status, index_completed_at,
        total_count, success_count, failed_count,
        index_checkpoint_json, source_reported_count, discovered_count,
        created_at, updated_at
      ) VALUES (
        ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?
      )
    `);
    stmt.run(...cleanParams([
      task.id,
      task.input_url || '',
      task.author_id || '',
      task.author_name || '',
      task.item_types || 'all',
      task.output_dir || 'output',
      task.sort_by || 'time',
      task.top_n !== undefined ? task.top_n : null,
      task.status || 'pending',
      task.index_status || 'pending',
      task.index_completed_at !== undefined ? task.index_completed_at : null,
      task.total_count || 0,
      task.success_count || 0,
      task.failed_count || 0,
      task.index_checkpoint_json !== undefined ? task.index_checkpoint_json : null,
      task.source_reported_count !== undefined ? task.source_reported_count : null,
      task.discovered_count || 0,
      task.created_at || now,
      task.updated_at || now
    ]));
  } else {
    const keys = Object.keys(task).filter(k => k !== 'id');
    if (keys.length === 0) return;

    const setClause = keys.map(k => `${k} = ?`).join(', ');
    const values = keys.map(k => (task as any)[k]);
    values.push(now);
    values.push(task.id);

    const stmt = database.prepare(`
      UPDATE tasks SET ${setClause}, updated_at = ? WHERE id = ?
    `);
    stmt.run(...cleanParams(values));
  }
}

/**
 * 仅允许 pending → running 的启动声明。
 * 暂停（paused）/取消（cancelled）/终态任务必须显式回到 pending 才能被调度，
 * 防止「已暂停/已取消的排队任务」被 runQueue 静默翻回 running 全量执行（P1-9）。
 */
export function tryStartTask(id: string): boolean {
  const database = getDb();
  const result = database.prepare(`
    UPDATE tasks
    SET status = 'running', updated_at = ?
    WHERE id = ? AND status = 'pending'
  `).run(Date.now(), id) as any;
  return Number(result?.changes || 0) > 0;
}

/**
 * P2-27③：原子状态迁移 —— UPDATE 自带「当前状态 ∈ from」门槛，绝不 INSERT。
 * pause/cancel 这类控制写走这里而不是 saveTask：saveTask 的「行不存在则插入」
 * 分支会为不存在的任务造出幻影行（旧 /pause 就踩过这个坑）。
 */
export function transitionTaskStatus(
  id: string,
  to: Task['status'],
  from: readonly Task['status'][],
): boolean {
  if (from.length === 0) return false;
  const database = getDb();
  const placeholders = from.map(() => '?').join(', ');
  const result = database.prepare(`
    UPDATE tasks
    SET status = ?, updated_at = ?
    WHERE id = ? AND status IN (${placeholders})
  `).run(to, Date.now(), id, ...from) as any;
  return Number(result?.changes || 0) > 0;
}

export function getTask(id: string): Task | null {
  const database = getDb();
  const stmt = database.prepare('SELECT * FROM tasks WHERE id = ?');
  const res = stmt.all(id) as unknown as Task[];
  return res.length > 0 ? res[0] : null;
}

export function getTasks(): Task[] {
  const database = getDb();
  const stmt = database.prepare('SELECT * FROM tasks ORDER BY created_at DESC');
  return stmt.all() as unknown as Task[];
}

export function saveItem(item: Item) {
  const database = getDb();
  const stmt = database.prepare(`
    INSERT INTO items (
      id, item_type, author_id, author_name, title, answer_id,
      question_id, article_id, url, question_url, created_time,
      updated_time, voteup_count, comment_count
    ) VALUES (
      ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?
    ) ON CONFLICT(id) DO UPDATE SET
      item_type = excluded.item_type,
      author_id = COALESCE(NULLIF(excluded.author_id, ''), items.author_id),
      author_name = COALESCE(NULLIF(excluded.author_name, ''), items.author_name),
      title = COALESCE(NULLIF(excluded.title, ''), items.title),
      answer_id = COALESCE(excluded.answer_id, items.answer_id),
      question_id = COALESCE(excluded.question_id, items.question_id),
      article_id = COALESCE(excluded.article_id, items.article_id),
      url = COALESCE(NULLIF(excluded.url, ''), items.url),
      question_url = COALESCE(excluded.question_url, items.question_url),
      created_time = CASE WHEN excluded.created_time > 0 THEN excluded.created_time ELSE items.created_time END,
      -- updated_time 是「内容最后修改时间」：一次索引读到旧快照不能把已记录的
      -- 更新值回退（P3-4）——取两者较大者，0（未知）永远输给任何真实时间戳。
      updated_time = MAX(items.updated_time, excluded.updated_time),
      voteup_count = excluded.voteup_count,
      comment_count = excluded.comment_count
  `);
  stmt.run(...cleanParams([
    item.id,
    item.item_type,
    item.author_id,
    item.author_name,
    item.title,
    item.answer_id,
    item.question_id,
    item.article_id,
    item.url,
    item.question_url,
    item.created_time,
    item.updated_time,
    item.voteup_count,
    item.comment_count
  ]));
}

export function getItem(id: string): Item | null {
  const database = getDb();
  const stmt = database.prepare('SELECT * FROM items WHERE id = ?');
  const res = stmt.all(id) as unknown as Item[];
  return res.length > 0 ? res[0] : null;
}

export function saveTaskItem(
  taskItem: TaskItem,
  options: { readonly recordArchive?: boolean } = {},
) {
  runInTransaction(() => {
    const database = getDb();
    const task = getTask(taskItem.task_id);
    const outputPath = normalizeStoredPath(taskItem.output_path, task?.output_dir || 'output');
    const stmt = database.prepare(`
      INSERT INTO task_items (
        task_id, item_id, status, output_path, failure_code, error_message, created_at, updated_at
      ) VALUES (
        ?, ?, ?, ?, ?, ?, ?, ?
      ) ON CONFLICT(task_id, item_id) DO UPDATE SET
        status = excluded.status,
        output_path = excluded.output_path,
        failure_code = excluded.failure_code,
        error_message = excluded.error_message,
        updated_at = excluded.updated_at
    `);
    stmt.run(...cleanParams([
      taskItem.task_id,
      taskItem.item_id,
      taskItem.status,
      outputPath,
      taskItem.failure_code,
      taskItem.error_message,
      taskItem.created_at,
      taskItem.updated_at
    ]));
    if (taskItem.status === 'success' && outputPath && options.recordArchive !== false) {
      archiveSuccessfulTaskItem(database, taskItem, outputPath);
    }
  });
}

export function recordPublishedTaskItems(taskItems: readonly TaskItem[]) {
  runInTransaction(() => {
    for (const taskItem of taskItems) {
      saveTaskItem(taskItem, { recordArchive: true });
    }
  });
}

function archiveSuccessfulTaskItem(database: DatabaseSync, taskItem: TaskItem, outputPath: string) {
  const item = getItem(taskItem.item_id);
  if (!item?.author_id) return;
  database.prepare(`
    INSERT INTO archive_authors (author_id, author_name, created_at, updated_at)
    VALUES (?, ?, ?, ?)
    ON CONFLICT(author_id) DO UPDATE SET author_name = excluded.author_name, updated_at = excluded.updated_at
  `).run(item.author_id, item.author_name, taskItem.created_at, taskItem.updated_at);
  database.prepare(`
    INSERT INTO archive_items (item_id, author_id, source_url, current_revision, created_at, updated_at)
    VALUES (?, ?, ?, 0, ?, ?)
    ON CONFLICT(item_id) DO UPDATE SET author_id = excluded.author_id, source_url = excluded.source_url, updated_at = excluded.updated_at
  `).run(item.id, item.author_id, item.url, taskItem.created_at, taskItem.updated_at);
  const existing = database.prepare(
    'SELECT revision FROM archive_revisions WHERE item_id = ? AND output_path = ?'
  ).all(item.id, outputPath)[0] as { revision: number } | undefined;
  const revision = existing?.revision ?? Number((database.prepare(
    'SELECT COALESCE(MAX(revision), 0) + 1 AS revision FROM archive_revisions WHERE item_id = ?'
  ).all(item.id)[0] as any).revision);
  if (!existing) {
    database.prepare(`
      INSERT INTO archive_revisions (item_id, revision, task_id, output_path, created_at)
      VALUES (?, ?, ?, ?, ?)
    `).run(item.id, revision, taskItem.task_id, outputPath, taskItem.updated_at);
  }
  database.prepare(
    'UPDATE archive_items SET current_revision = ?, updated_at = ? WHERE item_id = ?'
  ).run(revision, taskItem.updated_at, item.id);
}

function writeIndexItem(taskId: string, item: IndexItemInput, now: number) {
  saveItem({
    id: item.id,
    item_type: item.type,
    author_id: item.authorId,
    author_name: item.authorName,
    title: item.title,
    answer_id: item.type === 'answer' ? item.id.split(':')[1] : null,
    question_id: item.type === 'answer' ? item.questionId || null : null,
    article_id: item.type === 'article' ? item.id.split(':')[1] : null,
    url: item.url,
    question_url: item.type === 'answer' ? item.questionUrl || null : null,
    created_time: item.createdTime,
    updated_time: item.updatedTime,
    voteup_count: item.voteupCount,
    comment_count: item.commentCount
  });

  // Keep successful rows; only insert missing task_items as pending.
  const existing = getDb()
    .prepare('SELECT status FROM task_items WHERE task_id = ? AND item_id = ?')
    .all(taskId, item.id) as Array<{ status: string }>;
  if (existing.length > 0) return;
  saveTaskItem({
    task_id: taskId,
    item_id: item.id,
    status: 'pending',
    output_path: null,
    failure_code: null,
    error_message: null,
    created_at: now,
    updated_at: now
  });
}

/** Upsert discovered index items without wiping already-success content rows. */
export function upsertTaskIndexItems(
  taskId: string,
  authorName: string,
  items: IndexItemInput[],
  checkpoint?: IndexCheckpoint | null
) {
  const now = Date.now();
  runInTransaction(() => {
    saveTask({
      id: taskId,
      author_name: authorName,
      index_status: 'running',
      discovered_count: items.length,
      source_reported_count:
        checkpoint?.totals !== undefined && checkpoint?.totals !== null
          ? checkpoint.totals
          : undefined,
      index_checkpoint_json: checkpoint ? JSON.stringify(checkpoint) : undefined
    });
    for (const item of items) {
      writeIndexItem(taskId, item, now);
    }
    refreshTaskCounts(taskId);
  });
}

export function saveIndexCheckpoint(taskId: string, checkpoint: IndexCheckpoint) {
  saveTask({
    id: taskId,
    index_checkpoint_json: JSON.stringify(checkpoint),
    discovered_count: checkpoint.discovered ?? undefined,
    source_reported_count:
      checkpoint.totals !== undefined && checkpoint.totals !== null ? checkpoint.totals : undefined
  });
}

export function readIndexCheckpoint(taskId: string): IndexCheckpoint | null {
  const task = getTask(taskId);
  if (!task?.index_checkpoint_json) return null;
  try {
    return JSON.parse(task.index_checkpoint_json) as IndexCheckpoint;
  } catch {
    return null;
  }
}

export function replaceTaskIndex(taskId: string, authorName: string, items: IndexItemInput[]) {
  const database = getDb();
  const now = Date.now();

  runInTransaction(() => {
    saveTask({
      id: taskId,
      author_name: authorName,
      index_status: 'running'
    });

    database.prepare('DELETE FROM task_items WHERE task_id = ?').run(taskId);

    for (const item of items) {
      writeIndexItem(taskId, item, now);
    }

    refreshTaskCounts(taskId);
    saveTask({
      id: taskId,
      index_status: 'complete',
      index_completed_at: now,
      discovered_count: items.length,
      index_checkpoint_json: JSON.stringify({
        isEnd: true,
        discovered: items.length,
        stage: 'content',
        statusMessage: '索引完成',
        lastHeartbeatAt: now
      } satisfies IndexCheckpoint)
    });
  });
}

export function getTaskItems(taskId: string): (TaskItem & Item)[] {
  const database = getDb();
  const stmt = database.prepare(`
    SELECT ti.*, i.item_type, i.author_id, i.author_name, i.title, i.answer_id,
           i.question_id, i.article_id, i.url, i.question_url, i.created_time,
           i.updated_time, i.voteup_count, i.comment_count
    FROM task_items ti
    LEFT JOIN items i ON ti.item_id = i.id
    WHERE ti.task_id = ?
  `);
  return stmt.all(taskId) as unknown as (TaskItem & Item)[];
}

export function resetRunningTasks() {
  runInTransaction(() => {
    const database = getDb();
    database.exec(`
      UPDATE tasks SET status = 'pending' WHERE status = 'running';
      UPDATE tasks SET index_status = 'pending' WHERE index_status = 'running';
      UPDATE task_items SET status = 'pending' WHERE status = 'running';
    `);
  });
  logger.info('已将遗留的 running 任务和状态重置为 pending。');
}

export function refreshTaskCounts(taskId: string) {
  const database = getDb();

  const totalStmt = database.prepare('SELECT COUNT(*) as count FROM task_items WHERE task_id = ?');
  const successStmt = database.prepare("SELECT COUNT(*) as count FROM task_items WHERE task_id = ? AND status = 'success'");
  const failedStmt = database.prepare("SELECT COUNT(*) as count FROM task_items WHERE task_id = ? AND status = 'failed'");

  const total = (totalStmt.all(taskId)[0] as any).count;
  const success = (successStmt.all(taskId)[0] as any).count;
  const failed = (failedStmt.all(taskId)[0] as any).count;

  saveTask({
    id: taskId,
    total_count: total,
    success_count: success,
    failed_count: failed
  });
}

export function getAuthorSuccessItems(authorId: string): (Item & { output_path: string })[] {
  const database = getDb();
  const stmt = database.prepare(`
    SELECT i.*, ar.output_path
    FROM archive_items ai
    JOIN items i ON i.id = ai.item_id
    JOIN archive_revisions ar ON ar.item_id = ai.item_id AND ar.revision = ai.current_revision
    WHERE ai.author_id = ?
    ORDER BY i.created_time DESC
  `);
  return stmt.all(authorId) as unknown as (Item & { output_path: string })[];
}

export function getAllSuccessAuthors(): { author_id: string; author_name: string }[] {
  const database = getDb();
  const stmt = database.prepare(`
    SELECT author_id, author_name
    FROM archive_authors
    ORDER BY author_name
  `);
  return stmt.all() as unknown as { author_id: string; author_name: string }[];
}

export function deleteTask(id: string) {
  runInTransaction(() => {
    const database = getDb();
    database.prepare('DELETE FROM tasks WHERE id = ?').run(id);
  });
  logger.info(`已在数据库中成功物理清理任务 ${id} 及其关联记录`);
}

export function clearCompletedTasks(): number {
  const database = getDb();
  const stmt = database.prepare("SELECT id FROM tasks WHERE status IN ('success', 'failed', 'paused', 'cancelled', 'partial_success')");
  const completed = stmt.all() as unknown as { id: string }[];

  runInTransaction(() => {
    const deleteTaskStmt = database.prepare('DELETE FROM tasks WHERE id = ?');
    for (const t of completed) {
      deleteTaskStmt.run(t.id);
    }
  });
  logger.info(`已清理数据库中所有已结束（含成功/失败/暂停/取消/部分成功）的任务记录，共计 ${completed.length} 个`);
  return completed.length;
}

/**
 * 06-F-07：终态任务行（含 peopleId、条目计数）曾永久驻留 zhihu-packer.db——
 * 宿主 control.db 对终态任务有 7 天清理，边车这里对齐同一口径，启动时自清
 * 过期行。'paused' 可恢复、不算终态，保留。task_items 经 ON DELETE CASCADE
 * 级联删除。
 */
const TERMINAL_TASK_RETENTION_MS = 7 * 24 * 60 * 60 * 1000;

export function pruneTerminalTasks(olderThanMs = TERMINAL_TASK_RETENTION_MS): number {
  const cutoff = Date.now() - olderThanMs;
  const database = getDb();
  const stale = database.prepare(
    "SELECT id FROM tasks WHERE status IN ('success', 'failed', 'cancelled', 'partial_success') AND updated_at < ?"
  ).all(cutoff) as unknown as { id: string }[];
  if (stale.length === 0) {
    return 0;
  }
  runInTransaction(() => {
    const deleteTaskStmt = database.prepare('DELETE FROM tasks WHERE id = ?');
    for (const t of stale) {
      deleteTaskStmt.run(t.id);
    }
  });
  logger.info(`已清理 ${stale.length} 条超过保留期的终态任务记录。`);
  return stale.length;
}

export function resetTaskForce(taskId: string) {
  runInTransaction(() => {
    const database = getDb();
    database.prepare(`
      UPDATE task_items
      SET status = 'pending', output_path = null, failure_code = null, error_message = null
      WHERE task_id = ?
    `).run(taskId);

    database.prepare(`
      UPDATE tasks
      SET status = 'pending',
          index_status = CASE WHEN total_count > 0 THEN 'complete' ELSE 'pending' END,
          success_count = 0,
          failed_count = 0
      WHERE id = ?
    `).run(taskId);
  });

  logger.info(`数据库中任务 ${taskId} 的状态已被强制重置为 PENDING`);
}
