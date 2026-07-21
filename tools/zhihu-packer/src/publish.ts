import * as fs from "node:fs";
import * as path from "node:path";
import { createHash } from "node:crypto";
import { parseManifest } from "../../../packages/contracts/dist/index.js";
import { buildZhihuManifest, type ArchivedItem } from "./library-manifest.js";
import type { TaskItem } from "./db.js";

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
  const existingResult = committedResult(root, taskId, authorId, metadata);
  const authorDirectories = listAuthorDirectories(incomingRoot);
  if (authorDirectories.length === 0 && existingResult) {
    return existingResult;
  }
  if (authorDirectories.length !== 1) {
    throw new Error("ZHIHU_PUBLISH_FAILED: expected exactly one author directory");
  }
  const [authorDirectory] = authorDirectories;
  const incomingAuthor = path.join(incomingRoot, authorDirectory);
  const finalRoot = path.join(root, authorDirectory);
  const previousTransaction = readTransaction(root, taskId);
  if (previousTransaction?.phase === "committed" && previousTransaction.authorId === authorId) {
    const previousFinal = safePublishedRoot(root, previousTransaction.finalRelativePath);
    validateMetadata(root, previousTransaction, previousTransaction.finalRelativePath);
    if (path.basename(previousFinal) !== authorDirectory) {
      throw new Error("ZHIHU_PUBLISH_FAILED: retry author directory changed");
    }
    copyPublishedTree(previousFinal, incomingAuthor);
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
