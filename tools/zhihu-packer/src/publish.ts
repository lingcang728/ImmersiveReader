import * as fs from "node:fs";
import * as path from "node:path";
import { createHash } from "node:crypto";
import { parseManifest } from "../../../packages/contracts/dist/index.js";
import { buildZhihuManifest, type ArchivedItem } from "./library-manifest.js";
import type { TaskItem } from "./db.js";

export type ZhihuPublishPhase = "prepared" | "old_moved" | "new_moved" | "committed" | "rolled_back";

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
  taskId: string,
  authorId: string,
  revision: number,
  metadata: ZhihuPublishMetadata,
): { manifestSha256: string; provenanceSha256: string } {
  const generatedAt = new Date().toISOString();
  const items: ArchivedItem[] = metadata.items.map(item => {
    const normalized = item.output_path?.replaceAll("\\", "/") || "";
    const prefix = `.incoming/${taskId}/`;
    const relativeWithAuthor = normalized.startsWith(prefix)
      ? normalized.slice(prefix.length)
      : path.relative(incomingAuthor, item.output_path || "").replaceAll("\\", "/");
    const authorPrefix = `${path.basename(incomingAuthor).replaceAll("\\", "/")}/`;
    const relative = relativeWithAuthor.startsWith(authorPrefix)
      ? relativeWithAuthor.slice(authorPrefix.length)
      : relativeWithAuthor;
    const filePath = path.resolve(incomingAuthor, relative);
    const insideIncoming = filePath !== incomingAuthor && filePath.startsWith(`${incomingAuthor}${path.sep}`);
    if (!relative || relative.startsWith("../") || path.isAbsolute(relative) || !insideIncoming || !fs.existsSync(filePath)) {
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
      wordCount: fs.readFileSync(filePath, "utf8").replace(/\s+/g, "").length,
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
  metadata: ZhihuPublishMetadata,
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
  const metadataHashes = writeMetadata(incomingAuthor, taskId, authorId, revision, metadata);

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

  try {
    validateMetadata(root, transaction, transaction.incomingRelativePath + "/" + authorDirectory);
    if (fs.existsSync(finalRoot)) {
      ensureDirectory(path.dirname(rollbackRoot));
      fs.renameSync(finalRoot, rollbackRoot);
    }
    setPhase(root, transaction, "old_moved");
    ensureDirectory(path.dirname(finalRoot));
    fs.renameSync(incomingAuthor, finalRoot);
    setPhase(root, transaction, "new_moved");
    validateMetadata(root, transaction, transaction.finalRelativePath);
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
