import * as fs from "node:fs";
import * as path from "node:path";
import { createHash } from "node:crypto";
import { parseManifest } from "../../../packages/contracts/dist/index.js";
import { buildZhihuManifest, type ArchivedItem } from "./library-manifest.js";
import type { TaskItem } from "./db.js";
import { countWordChars, logger, sanitizeFilename } from "./utils.js";

export type ZhihuPublishPhase = "prepared" | "old_moved" | "new_moved" | "committed" | "rolled_back";

export function isPublishableTaskStatus(status: string, successCount: number): boolean {
  return (status === "success" || status === "partial_success") && successCount > 0;
}

export type ZhihuPublishTransaction = {
  readonly schemaVersion: 1;
  readonly transactionId: string;
  readonly taskId: string;
  readonly authorId: string;
  readonly bookId: string;
  readonly sourceId: string;
  readonly incomingRelativePath: string;
  readonly finalRelativePath: string;
  readonly rollbackRelativePath: string;
  readonly revision: number;
  readonly manifestSha256: string;
  readonly provenanceSha256: string;
  phase: ZhihuPublishPhase;
  createdAt: string;
  updatedAt: string;
};

function safeSegment(value: string): boolean {
  return value.length > 0
    && value !== "."
    && value !== ".."
    && !value.includes("/")
    && !value.includes("\\")
    && /^[A-Za-z0-9_-]+$/.test(value);
}

function assertSafeTaskId(taskId: string): void {
  if (!safeSegment(taskId)) {
    throw new Error("ZHIHU_PUBLISH_FAILED: unsafe task id");
  }
}

function writeJsonAtomic(filePath: string, value: unknown): void {
  const tempPath = `${filePath}.tmp-${process.pid}`;
  fs.writeFileSync(tempPath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
  fs.renameSync(tempPath, filePath);
}

function ensureDirectory(pathname: string): void {
  fs.mkdirSync(pathname, { recursive: true });
}

function isSafeDirectoryEntry(entry: fs.Dirent): boolean {
  return !entry.isSymbolicLink()
    && entry.isDirectory()
    && entry.name !== "."
    && entry.name !== ".."
    && !entry.name.includes("/")
    && !entry.name.includes("\\");
}

export function taskIncomingRoot(outputRoot: string, taskId: string): string {
  assertSafeTaskId(taskId);
  return path.join(path.resolve(outputRoot), ".incoming", taskId);
}

/**
 * P2-27①：发布临时产物的保留策略 —— 曾经三者都永不清，重归档一次整树翻一倍。
 * - `.incoming/<taskId>` 壳：提交即被 rename 消耗，残留的只是空壳/杂散文件 → 删除；
 *   （失败路径保留，供重试与排查。）
 * - `.transactions/zhihu-<taskId>.json`：当前任务的 committed journal 必须保留
 *   —— committedResult 幂等重发、重跑时「并入上次已发布树」都靠它；其他任务
 *   rolled_back 的 journal 回滚已执行完，只剩死记录 → 顺手清掉。
 * - `.revisions/<authorId>/<N>`：每次发布把旧版整树挪进新的 N 目录。只保留最新
 *   一份（当前已发布树唯一需要的回滚副本），更早的删除。
 */
const RETAINED_ROLLBACK_REVISIONS = 1;

function removeIncomingShell(root: string, taskId: string): void {
  try {
    fs.rmSync(taskIncomingRoot(root, taskId), { recursive: true, force: true });
  } catch (err) {
    logger.warn(`清理 .incoming 暂存壳失败（不影响已提交归档）: ${err instanceof Error ? err.message : err}`);
  }
}

export function cleanupPublishArtifacts(root: string, taskId: string, authorId: string): void {
  removeIncomingShell(root, taskId);
  try {
    const transactionsDir = path.join(root, ".transactions");
    if (fs.existsSync(transactionsDir)) {
      for (const entry of fs.readdirSync(transactionsDir, { withFileTypes: true })) {
        if (!entry.isFile() || !entry.name.endsWith(".json")) continue;
        const journalPath = path.join(transactionsDir, entry.name);
        try {
          const journal = JSON.parse(fs.readFileSync(journalPath, "utf8")) as Partial<ZhihuPublishTransaction>;
          if (journal?.schemaVersion === 1 && journal.phase === "rolled_back" && journal.taskId !== taskId) {
            fs.rmSync(journalPath, { force: true });
          }
        } catch {
          // 不可读的 journal 留给人工排查，不替它做删除决定。
        }
      }
    }
    const revisionsRoot = revisionDirectory(root, authorId);
    if (fs.existsSync(revisionsRoot)) {
      const numericDirs = fs.readdirSync(revisionsRoot, { withFileTypes: true })
        .filter(entry => entry.isDirectory() && /^\d+$/.test(entry.name))
        .map(entry => Number(entry.name))
        .sort((a, b) => b - a);
      for (const stale of numericDirs.slice(RETAINED_ROLLBACK_REVISIONS)) {
        fs.rmSync(path.join(revisionsRoot, String(stale)), { recursive: true, force: true });
      }
    }
  } catch (err) {
    logger.warn(`发布后清理 revisions/journal 失败（不影响已提交归档）: ${err instanceof Error ? err.message : err}`);
  }
}

export type ZhihuPublishMetadata = {
  readonly authorName: string;
  readonly items: readonly (TaskItem & {
    readonly author_id: string;
    readonly author_name: string;
    readonly title: string;
    readonly created_time: number;
    readonly voteup_count: number;
  })[];
};

function sha256File(filePath: string): string {
  return createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

function writeMetadata(
  incomingAuthor: string,
  publishedAuthor: string,
  taskId: string,
  authorId: string,
  revision: number,
  metadata: ZhihuPublishMetadata,
): { manifestSha256: string; provenanceSha256: string } {
  const generatedAt = new Date().toISOString();
  const items: ArchivedItem[] = metadata.items.map(item => {
    const normalized = item.output_path?.replaceAll("\\", "/") || "";
    const prefix = `.incoming/${taskId}/`;
    const publishedAuthorPrefix = `${path.basename(publishedAuthor).replaceAll("\\", "/")}/`;
    const absoluteOutput = item.output_path && path.isAbsolute(item.output_path)
      ? path.resolve(item.output_path)
      : null;
    const relativeWithAuthor = normalized.startsWith(prefix)
      ? normalized.slice(prefix.length)
      : normalized.startsWith(publishedAuthorPrefix)
        ? normalized
        : absoluteOutput
          && absoluteOutput !== publishedAuthor
          && absoluteOutput.startsWith(`${publishedAuthor}${path.sep}`)
          ? `${publishedAuthorPrefix}${path.relative(publishedAuthor, absoluteOutput).replaceAll("\\", "/")}`
          : path.relative(incomingAuthor, item.output_path || "").replaceAll("\\", "/");
    const authorPrefix = `${path.basename(incomingAuthor).replaceAll("\\", "/")}/`;
    const relative = relativeWithAuthor.startsWith(authorPrefix)
      ? relativeWithAuthor.slice(authorPrefix.length)
      : relativeWithAuthor;
    if (!relative || relative.startsWith("../") || path.isAbsolute(relative)) {
      throw new Error(`ZHIHU_PUBLISH_FAILED: staged file missing for ${item.item_id}`);
    }
    let filePath = path.resolve(incomingAuthor, relative);
    const insideIncoming = () =>
      filePath !== incomingAuthor && filePath.startsWith(`${incomingAuthor}${path.sep}`);
    if (!insideIncoming() || !fs.existsSync(filePath)) {
      // P1-10：多作者目录归并后，条目 output_path 里仍可能记着旧目录名；
      // 章节文件一律平铺在作者目录根部，按文件名回退定位。
      const flat = path.join(incomingAuthor, path.basename(relative));
      if (fs.existsSync(flat)) {
        filePath = flat;
      }
    }
    if (!insideIncoming() || !fs.existsSync(filePath)) {
      throw new Error(`ZHIHU_PUBLISH_FAILED: staged file missing for ${item.item_id}`);
    }
    const chapterPath = path.relative(incomingAuthor, filePath).replaceAll("\\", "/");
    return {
      id: item.item_id,
      authorId: item.author_id,
      authorName: item.author_name,
      title: item.title,
      createdTime: item.created_time,
      voteCount: item.voteup_count,
      outputPath: chapterPath,
      // P3-29：wordCount 规范口径 = Unicode 标量数（与 Rust 端一致），不按 UTF-16 码元。
      wordCount: countWordChars(fs.readFileSync(filePath, "utf8")),
    };
  });
  const manifest = parseManifest(buildZhihuManifest({
    authorId,
    authorName: metadata.authorName,
    generatedAt,
    items,
    inferredChapters: [],
  }));
  const manifestPath = path.join(incomingAuthor, "manifest.json");
  writeJsonAtomic(manifestPath, manifest);
  const manifestSha256 = sha256File(manifestPath);
  const provenancePath = path.join(incomingAuthor, "provenance.json");
  writeJsonAtomic(provenancePath, {
    schemaVersion: 1,
    bookId: `zhihu:${authorId}`,
    sourceId: authorId,
    sourceKind: "zhihu",
    createdByTaskId: taskId,
    lastSuccessfulTaskId: taskId,
    revision,
    manifestSha256,
    engineVersion: "zhihu-packer@1.0.0",
    updatedAt: generatedAt,
  });
  return { manifestSha256, provenanceSha256: sha256File(provenancePath) };
}

function validateMetadata(root: string, transaction: ZhihuPublishTransaction, relative: string): void {
  const bookRoot = path.resolve(root, relative);
  const manifestPath = path.join(bookRoot, "manifest.json");
  const provenancePath = path.join(bookRoot, "provenance.json");
  if (sha256File(manifestPath) !== transaction.manifestSha256 || sha256File(provenancePath) !== transaction.provenanceSha256) {
    throw new Error("ZHIHU_PUBLISH_FAILED: metadata hash mismatch");
  }
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8")) as Record<string, unknown>;
  const provenance = JSON.parse(fs.readFileSync(provenancePath, "utf8")) as Record<string, unknown>;
  if (manifest.bookId !== transaction.bookId || provenance.bookId !== transaction.bookId
    || manifest.sourceId !== transaction.sourceId || provenance.sourceId !== transaction.sourceId
    || provenance.revision !== transaction.revision || provenance.manifestSha256 !== transaction.manifestSha256) {
    throw new Error("ZHIHU_PUBLISH_FAILED: metadata identity mismatch");
  }
  parseManifest(manifest);
}

export function resetTaskIncoming(outputRoot: string, taskId: string): string {
  const root = taskIncomingRoot(outputRoot, taskId);
  fs.rmSync(root, { recursive: true, force: true });
  ensureDirectory(root);
  return root;
}

export function resolvePublishedTaskItemPath(
  outputRoot: string,
  taskId: string,
  outputPath: string | null | undefined,
): string | null {
  if (!outputPath) return null;
  const root = path.resolve(outputRoot);
  const normalized = outputPath.replaceAll("\\", "/");
  const prefix = `.incoming/${taskId}/`;
  let relative = normalized.startsWith(prefix)
    ? normalized.slice(prefix.length)
    : path.isAbsolute(outputPath)
      ? path.relative(taskIncomingRoot(root, taskId), outputPath).replaceAll("\\", "/")
      : normalized;
  relative = relative.replace(/^\.\//, "");
  if (!relative || relative.startsWith("../") || path.isAbsolute(relative)) {
    return null;
  }
  const resolved = path.resolve(root, relative);
  if (resolved !== root && !resolved.startsWith(`${root}${path.sep}`)) {
    return null;
  }
  return resolved;
}

function revisionDirectory(outputRoot: string, authorId: string): string {
  if (!safeSegment(authorId)) {
    throw new Error("ZHIHU_PUBLISH_FAILED: unsafe author id");
  }
  return path.join(path.resolve(outputRoot), ".revisions", authorId);
}

function currentPublishedRevision(finalRoot: string): number {
  const provenancePath = path.join(finalRoot, "provenance.json");
  if (!fs.existsSync(provenancePath)) return 0;
  try {
    const provenance = JSON.parse(fs.readFileSync(provenancePath, "utf8")) as Record<string, unknown>;
    const revision = Number(provenance.revision);
    return Number.isSafeInteger(revision) && revision > 0 ? revision : 0;
  } catch {
    return 0;
  }
}

function nextRevision(root: string, finalRoot: string): number {
  const revisions = fs.existsSync(root) ? fs.readdirSync(root, { withFileTypes: true })
    .filter(entry => entry.isDirectory() && /^\d+$/.test(entry.name))
    .map(entry => Number(entry.name))
    .filter(Number.isSafeInteger) : [];
  return Math.max(currentPublishedRevision(finalRoot), ...revisions, 0) + 1;
}

function transactionPath(outputRoot: string, taskId: string): string {
  return path.join(path.resolve(outputRoot), ".transactions", `zhihu-${taskId}.json`);
}

function readTransaction(outputRoot: string, taskId: string): ZhihuPublishTransaction | null {
  const journal = transactionPath(outputRoot, taskId);
  if (!fs.existsSync(journal)) return null;
  try {
    const transaction = JSON.parse(fs.readFileSync(journal, "utf8")) as ZhihuPublishTransaction;
    return transaction.schemaVersion === 1 && transaction.taskId === taskId ? transaction : null;
  } catch {
    return null;
  }
}

function saveTransaction(outputRoot: string, transaction: ZhihuPublishTransaction): void {
  const journal = transactionPath(outputRoot, transaction.taskId);
  ensureDirectory(path.dirname(journal));
  writeJsonAtomic(journal, transaction);
}

function setPhase(outputRoot: string, transaction: ZhihuPublishTransaction, phase: ZhihuPublishPhase): void {
  transaction.phase = phase;
  transaction.updatedAt = new Date().toISOString();
  saveTransaction(outputRoot, transaction);
}

function listAuthorDirectories(incomingRoot: string): string[] {
  if (!fs.existsSync(incomingRoot)) {
    return [];
  }
  const entries = fs.readdirSync(incomingRoot, { withFileTypes: true });
  const unsafe = entries.some(entry => entry.isSymbolicLink());
  if (unsafe) throw new Error("ZHIHU_PUBLISH_FAILED: incoming directory contains a symlink");
  return entries.filter(isSafeDirectoryEntry).map(entry => entry.name);
}

/**
 * P3-28：临时残片（图片下载的 `.<hash>.tmp-<pid>-<ts>`、JSON 原子写的
 * `*.tmp-<pid>` 等）在进程崩溃后会以孤儿形式留在 assets/ 等目录里，
 * 绝不能被拷进发布树。判定口径：文件名含 `.tmp-` 或以 `.tmp` 开头。
 */
function isTransientArtifactName(name: string): boolean {
  return name.startsWith(".tmp") || name.includes(".tmp-");
}

/** 发布前清掉 incoming 树里的临时残片孤儿（递归，含 assets/ 子目录）。 */
function pruneTransientArtifacts(dir: string): void {
  let entries: fs.Dirent[];
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return;
  }
  for (const entry of entries) {
    if (entry.isSymbolicLink()) continue;
    const entryPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      pruneTransientArtifacts(entryPath);
    } else if (entry.isFile() && isTransientArtifactName(entry.name)) {
      try {
        fs.rmSync(entryPath, { force: true });
        logger.warn(`已清理发布树中的临时残片: ${entryPath}`);
      } catch {
        // 清理失败不阻断发布——该文件最多以孤儿形式存在。
      }
    }
  }
}

/** 递归把 source 目录里目标处尚不存在的文件并入 destination（不覆盖已有文件）。 */
function mergeDirectoryInto(source: string, destination: string): void {
  ensureDirectory(destination);
  for (const entry of fs.readdirSync(source, { withFileTypes: true })) {
    if (entry.isSymbolicLink()) {
      throw new Error("ZHIHU_PUBLISH_FAILED: incoming directory contains a symlink");
    }
    if (entry.isFile() && isTransientArtifactName(entry.name)) {
      continue; // P3-28：临时残片不并入规范目录
    }
    const sourcePath = path.join(source, entry.name);
    const destinationPath = path.join(destination, entry.name);
    if (entry.isDirectory()) {
      mergeDirectoryInto(sourcePath, destinationPath);
    } else if (entry.isFile() && !fs.existsSync(destinationPath)) {
      fs.renameSync(sourcePath, destinationPath);
    }
  }
}

/**
 * P1-10：归档目录名归一到任务 peopleId 前的历史遗留，可能让 .incoming/<task>/
 * 下同时存在多个作者目录（'unknown'/'anonymous'/真实 ID 混用），导致发布必败。
 * 这里把多余目录的内容归并进任务级规范目录 sanitizeFilename(authorName, authorId)，
 * 文件名冲突时保留先入目录的同名文件（文件名本身含条目 ID，冲突即同一条目）。
 */
function consolidateAuthorDirectories(
  incomingRoot: string,
  authorId: string,
  authorName: string,
  directories: readonly string[],
): string {
  const preferred = sanitizeFilename(authorName || '未知作者', authorId);
  const target = path.join(incomingRoot, preferred);
  const merged: string[] = [];
  for (const dir of directories) {
    if (dir === preferred) continue;
    mergeDirectoryInto(path.join(incomingRoot, dir), target);
    fs.rmSync(path.join(incomingRoot, dir), { recursive: true, force: true });
    merged.push(dir);
  }
  if (merged.length > 0) {
    logger.warn(`归档目录名已归一并合并到任务级规范目录 ${preferred}（来源: ${merged.join(', ')}）`);
  }
  return preferred;
}

function safePublishedRoot(root: string, relative: string): string {
  if (!relative || path.isAbsolute(relative)) {
    throw new Error("ZHIHU_PUBLISH_FAILED: unsafe published path");
  }
  const resolved = path.resolve(root, relative);
  if (resolved === root || !resolved.startsWith(`${root}${path.sep}`)) {
    throw new Error("ZHIHU_PUBLISH_FAILED: published path escapes output root");
  }
  return resolved;
}

function copyPublishedTree(source: string, destination: string, root = true): void {
  ensureDirectory(destination);
  for (const entry of fs.readdirSync(source, { withFileTypes: true })) {
    if (entry.isSymbolicLink()) {
      throw new Error("ZHIHU_PUBLISH_FAILED: published archive contains a symlink");
    }
    if (root && (entry.name === "manifest.json" || entry.name === "provenance.json")) {
      continue;
    }
    if (entry.isFile() && isTransientArtifactName(entry.name)) {
      continue; // P3-28：旧发布树里的临时残片不拷回新一轮暂存
    }
    const sourcePath = path.join(source, entry.name);
    const destinationPath = path.join(destination, entry.name);
    if (entry.isDirectory()) {
      copyPublishedTree(sourcePath, destinationPath, false);
    } else if (entry.isFile() && !fs.existsSync(destinationPath)) {
      fs.copyFileSync(sourcePath, destinationPath);
    }
  }
}

function metadataItemIds(metadata: ZhihuPublishMetadata): string[] {
  return [...new Set(metadata.items.map(item => item.item_id))].sort();
}

function publishedItemIds(finalRoot: string): string[] {
  const manifest = parseManifest(JSON.parse(fs.readFileSync(path.join(finalRoot, "manifest.json"), "utf8")));
  return manifest.chapters.map(chapter => chapter.id).sort();
}

function sameIds(left: readonly string[], right: readonly string[]): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

function committedResult(
  root: string,
  taskId: string,
  authorId: string,
  metadata: ZhihuPublishMetadata,
): ZhihuPublishResult | null {
  const transaction = readTransaction(root, taskId);
  if (!transaction || transaction.phase !== "committed" || transaction.authorId !== authorId) {
    return null;
  }
  const finalRoot = safePublishedRoot(root, transaction.finalRelativePath);
  validateMetadata(root, transaction, transaction.finalRelativePath);
  if (!sameIds(publishedItemIds(finalRoot), metadataItemIds(metadata))) {
    return null;
  }
  return {
    transaction,
    finalRoot,
    authorDirectory: path.basename(finalRoot),
  };
}

export type ZhihuPublishResult = {
  readonly transaction: ZhihuPublishTransaction;
  readonly finalRoot: string;
  readonly authorDirectory: string;
};

export function publishTaskStage(
  outputRoot: string,
  taskId: string,
  authorId: string,
  metadata: ZhihuPublishMetadata,
): ZhihuPublishResult {
  const root = path.resolve(outputRoot);
  const incomingRoot = taskIncomingRoot(root, taskId);
  // Crash recovery: a journal left in prepared/old_moved/new_moved means the
  // process died mid-transaction. new_moved + final present → the new tree
  // already landed, just commit it; otherwise restore the rollback copy of
  // the previous archive so the retry sees the same world the first attempt
  // did (without this, a re-run threw "expected exactly one author directory"
  // or silently dropped the previously published chapters).
  const crashed = readTransaction(root, taskId);
  if (
    crashed &&
    crashed.authorId === authorId &&
    (crashed.phase === "prepared" || crashed.phase === "old_moved" || crashed.phase === "new_moved")
  ) {
    const crashedFinal = safePublishedRoot(root, crashed.finalRelativePath);
    const crashedRollback = safePublishedRoot(root, crashed.rollbackRelativePath);
    if (crashed.phase === "new_moved" && fs.existsSync(crashedFinal)) {
      validateMetadata(root, crashed, crashed.finalRelativePath);
      setPhase(root, crashed, "committed");
      cleanupPublishArtifacts(root, taskId, authorId);
      return {
        transaction: crashed,
        finalRoot: crashedFinal,
        authorDirectory: path.basename(crashedFinal),
      };
    }
    if (!fs.existsSync(crashedFinal) && fs.existsSync(crashedRollback)) {
      ensureDirectory(path.dirname(crashedFinal));
      fs.renameSync(crashedRollback, crashedFinal);
    }
  }
  const existingResult = committedResult(root, taskId, authorId, metadata);
  let authorDirectories = listAuthorDirectories(incomingRoot);
  if (authorDirectories.length === 0 && existingResult) {
    // 幂等重发命中：暂存壳同样已无用，顺手清掉（P2-27①）。
    removeIncomingShell(root, taskId);
    return existingResult;
  }
  if (authorDirectories.length > 1) {
    // 旧版本混合 authorId 产生的多作者目录归并到任务级规范目录，避免整任务发布必败。
    authorDirectories = [
      consolidateAuthorDirectories(incomingRoot, authorId, metadata.authorName, authorDirectories),
    ];
  }
  if (authorDirectories.length !== 1) {
    throw new Error("ZHIHU_PUBLISH_FAILED: expected exactly one author directory");
  }
  const [authorDirectory] = authorDirectories;
  const incomingAuthor = path.join(incomingRoot, authorDirectory);
  // P3-28：发布即把 incomingAuthor 整树 rename 进最终目录——先清掉 assets/ 等处
  // 残留的 `.tmp-*`/`.tmp*` 孤儿残片，否则它们会被一并搬进发布树。
  pruneTransientArtifacts(incomingAuthor);
  const finalRoot = path.join(root, authorDirectory);
  const previousTransaction = readTransaction(root, taskId);
  if (previousTransaction?.phase === "committed" && previousTransaction.authorId === authorId) {
    const previousFinal = safePublishedRoot(root, previousTransaction.finalRelativePath);
    validateMetadata(root, previousTransaction, previousTransaction.finalRelativePath);
    if (path.basename(previousFinal) !== authorDirectory) {
      throw new Error("ZHIHU_PUBLISH_FAILED: retry author directory changed");
    }
    copyPublishedTree(previousFinal, incomingAuthor);
  } else if (fs.existsSync(finalRoot) && fs.existsSync(incomingAuthor)) {
    // Crash-recovered or cross-task republish: the existing archive may hold
    // chapters this run did not re-fetch — merge them into the staging tree
    // instead of dropping them into the rollback copy.
    copyPublishedTree(finalRoot, incomingAuthor);
  }
  const revision = nextRevision(revisionDirectory(root, authorId), finalRoot);
  const rollbackRoot = path.join(revisionDirectory(root, authorId), String(revision));
  if (fs.existsSync(rollbackRoot)) {
    throw new Error("ZHIHU_PUBLISH_FAILED: revision directory already exists");
  }
  const metadataHashes = writeMetadata(incomingAuthor, finalRoot, taskId, authorId, revision, metadata);

  const transaction: ZhihuPublishTransaction = {
    schemaVersion: 1,
    transactionId: `zhihu-${taskId}`,
    taskId,
    authorId,
    bookId: `zhihu:${authorId}`,
    sourceId: authorId,
    incomingRelativePath: path.relative(root, incomingRoot).replaceAll("\\", "/"),
    finalRelativePath: path.relative(root, finalRoot).replaceAll("\\", "/"),
    rollbackRelativePath: path.relative(root, rollbackRoot).replaceAll("\\", "/"),
    revision,
    ...metadataHashes,
    phase: "prepared",
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
  };
  saveTransaction(root, transaction);

  let oldMoved = false;
  let newMoved = false;
  try {
    validateMetadata(root, transaction, transaction.incomingRelativePath + "/" + authorDirectory);
    if (fs.existsSync(finalRoot)) {
      ensureDirectory(path.dirname(rollbackRoot));
      fs.renameSync(finalRoot, rollbackRoot);
      oldMoved = true;
    }
    setPhase(root, transaction, "old_moved");
    ensureDirectory(path.dirname(finalRoot));
    fs.renameSync(incomingAuthor, finalRoot);
    newMoved = true;
    setPhase(root, transaction, "new_moved");
    validateMetadata(root, transaction, transaction.finalRelativePath);
    setPhase(root, transaction, "committed");
    // P2-27①：归档已 durable，再清理本次发布的暂存壳、过期回滚副本与死 journal。
    // 清理函数内部逐段兜底，任何失败只记日志、绝不影响已提交的归档。
    cleanupPublishArtifacts(root, taskId, authorId);
    return { transaction, finalRoot, authorDirectory };
  } catch (error) {
    try {
      if (newMoved && fs.existsSync(finalRoot)) fs.rmSync(finalRoot, { recursive: true, force: true });
      if (oldMoved && fs.existsSync(rollbackRoot)) {
        ensureDirectory(path.dirname(finalRoot));
        fs.renameSync(rollbackRoot, finalRoot);
      }
      setPhase(root, transaction, "rolled_back");
    } catch {}
    throw error;
  }
}
