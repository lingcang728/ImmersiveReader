import * as fs from "node:fs";
import * as path from "node:path";

export type ZhihuPublishPhase = "prepared" | "old_moved" | "new_moved" | "committed" | "rolled_back";

export type ZhihuPublishTransaction = {
  readonly schemaVersion: 1;
  readonly transactionId: string;
  readonly taskId: string;
  readonly authorId: string;
  readonly incomingRelativePath: string;
  readonly finalRelativePath: string;
  readonly rollbackRelativePath: string;
  readonly revision: number;
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

function nextRevision(root: string): number {
  if (!fs.existsSync(root)) return 1;
  const revisions = fs.readdirSync(root, { withFileTypes: true })
    .filter(entry => entry.isDirectory() && /^\d+$/.test(entry.name))
    .map(entry => Number(entry.name))
    .filter(Number.isSafeInteger);
  return (revisions.length > 0 ? Math.max(...revisions) : 0) + 1;
}

function transactionPath(outputRoot: string, taskId: string): string {
  return path.join(path.resolve(outputRoot), ".transactions", `zhihu-${taskId}.json`);
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
    throw new Error("ZHIHU_PUBLISH_FAILED: incoming directory is missing");
  }
  const entries = fs.readdirSync(incomingRoot, { withFileTypes: true });
  const unsafe = entries.some(entry => entry.isSymbolicLink());
  if (unsafe) throw new Error("ZHIHU_PUBLISH_FAILED: incoming directory contains a symlink");
  const directories = entries.filter(isSafeDirectoryEntry).map(entry => entry.name);
  if (directories.length !== 1) {
    throw new Error("ZHIHU_PUBLISH_FAILED: expected exactly one author directory");
  }
  return directories;
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
): ZhihuPublishResult {
  const root = path.resolve(outputRoot);
  const incomingRoot = taskIncomingRoot(root, taskId);
  const [authorDirectory] = listAuthorDirectories(incomingRoot);
  const incomingAuthor = path.join(incomingRoot, authorDirectory);
  const finalRoot = path.join(root, authorDirectory);
  const revision = nextRevision(revisionDirectory(root, authorId));
  const rollbackRoot = path.join(revisionDirectory(root, authorId), String(revision));
  if (fs.existsSync(rollbackRoot)) {
    throw new Error("ZHIHU_PUBLISH_FAILED: revision directory already exists");
  }

  const transaction: ZhihuPublishTransaction = {
    schemaVersion: 1,
    transactionId: `zhihu-${taskId}`,
    taskId,
    authorId,
    incomingRelativePath: path.relative(root, incomingRoot).replaceAll("\\", "/"),
    finalRelativePath: path.relative(root, finalRoot).replaceAll("\\", "/"),
    rollbackRelativePath: path.relative(root, rollbackRoot).replaceAll("\\", "/"),
    revision,
    phase: "prepared",
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
  };
  saveTransaction(root, transaction);

  try {
    if (fs.existsSync(finalRoot)) {
      ensureDirectory(path.dirname(rollbackRoot));
      fs.renameSync(finalRoot, rollbackRoot);
    }
    setPhase(root, transaction, "old_moved");
    ensureDirectory(path.dirname(finalRoot));
    fs.renameSync(incomingAuthor, finalRoot);
    setPhase(root, transaction, "new_moved");
    setPhase(root, transaction, "committed");
    return { transaction, finalRoot, authorDirectory };
  } catch (error) {
    try {
      if (fs.existsSync(finalRoot)) fs.rmSync(finalRoot, { recursive: true, force: true });
      if (fs.existsSync(rollbackRoot)) {
        ensureDirectory(path.dirname(finalRoot));
        fs.renameSync(rollbackRoot, finalRoot);
      }
      setPhase(root, transaction, "rolled_back");
    } catch {}
    throw error;
  }
}
