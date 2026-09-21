import * as fs from "node:fs";
import * as path from "node:path";

import { parseManifest, type BookManifest, type Chapter } from "../../../packages/contracts/dist/index.js";
import type { Item } from "./db.js";
import { buildZhihuManifest, inferOrphanChapter, type ArchivedItem } from "./library-manifest.js";
import { countWordChars, sanitizeFilename } from "./utils.js";

export type SuccessItem = Item & { readonly output_path: string };

export type ManifestReport = {
  readonly manifestPath: string;
  readonly databaseItems: number;
  readonly inferredItems: number;
  readonly missingItems: number;
  readonly excludedIndexes: number;
};

export type GenerateAuthorManifestInput = {
  readonly projectRoot: string;
  readonly outputRoot: string;
  readonly author: Readonly<{ author_id: string; author_name: string }>;
  readonly items: readonly SuccessItem[];
  readonly generatedAt?: string;
  readonly write: boolean;
};

export function authorDirectoryName(authorName: string, authorId: string): string {
  return sanitizeFilename(authorName, authorId);
}

function resolveArticlePath(
  projectRoot: string,
  outputRoot: string,
  authorPath: string,
  outputPath: string,
): { filePath: string; usedBasenameFallback: boolean } | null {
  const candidates: Array<{ filePath: string; usedBasenameFallback: boolean }> = [
    // output_path 的相对基准是归档输出根（`.incoming/<task>/…`、`作者/x.md`
    // 都相对它）；旧库也可能存了相对仓库根的 `output/…` 写法，作次候选。
    // 两个候选都以存在性裁决，顺序只是优先级。
    ...(path.isAbsolute(outputPath)
      ? [{ filePath: outputPath, usedBasenameFallback: false }]
      : [
          { filePath: path.resolve(outputRoot, outputPath), usedBasenameFallback: false },
          { filePath: path.resolve(projectRoot, outputPath), usedBasenameFallback: false },
        ]),
    { filePath: path.join(authorPath, path.basename(outputPath)), usedBasenameFallback: true },
  ];
  return (
    candidates.find(
      (candidate) => fs.existsSync(candidate.filePath) && fs.statSync(candidate.filePath).isFile(),
    ) ?? null
  );
}

function listMarkdownFiles(root: string, current = root): readonly string[] {
  if (!fs.existsSync(current)) {
    return [];
  }
  const files: string[] = [];
  for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
    if (entry.isSymbolicLink()) {
      continue;
    }
    const fullPath = path.join(current, entry.name);
    if (entry.isDirectory()) {
      files.push(...listMarkdownFiles(root, fullPath));
    } else if (entry.isFile() && /\.md$/i.test(entry.name)) {
      files.push(path.relative(root, fullPath).replaceAll("\\", "/"));
    }
  }
  return files.sort((left, right) => left.localeCompare(right, "zh-CN", { numeric: true }));
}

function writeManifestAtomic(filePath: string, manifest: BookManifest): void {
  const tempPath = `${filePath}.tmp-${process.pid}`;
  fs.writeFileSync(tempPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  fs.renameSync(tempPath, filePath);
}

export function loadManifest(filePath: string): BookManifest {
  const raw: unknown = JSON.parse(fs.readFileSync(filePath, "utf8"));
  return parseManifest(raw);
}

export function generateAuthorManifest(input: GenerateAuthorManifestInput): ManifestReport {
  const generatedAt = input.generatedAt ?? new Date().toISOString();
  const { author, items, outputRoot, projectRoot } = input;
  const authorPath = path.join(outputRoot, authorDirectoryName(author.author_name, author.author_id));
  fs.mkdirSync(authorPath, { recursive: true });
  let missingItems = 0;
  const archivedItems: ArchivedItem[] = [];
  const representedPaths = new Set<string>();
  for (const item of items) {
    const resolved = resolveArticlePath(projectRoot, outputRoot, authorPath, item.output_path);
    if (resolved === null) {
      missingItems += 1;
      continue;
    }
    const relativePath = path.relative(authorPath, resolved.filePath).replaceAll("\\", "/");
    const markdown = fs.readFileSync(resolved.filePath, "utf8");
    representedPaths.add(relativePath.toLowerCase());
    // Registering the bare basename unconditionally would wrongly suppress a
    // real orphan that merely shares a file name with this chapter in a
    // different subdirectory — only mark it when resolution actually went
    // through the basename fallback.
    if (resolved.usedBasenameFallback) {
      representedPaths.add(path.basename(relativePath).toLowerCase());
    }
    archivedItems.push({
      id: item.id,
      authorId: item.author_id,
      authorName: item.author_name,
      title: item.title,
      createdTime: item.created_time,
      voteCount: item.voteup_count,
      outputPath: relativePath,
      // P3-29：wordCount 规范口径 = Unicode 标量数（与 Rust 端一致），不按 UTF-16 码元。
      wordCount: countWordChars(markdown),
    });
  }

  let excludedIndexes = 0;
  const inferredChapters: Chapter[] = [];
  for (const relativePath of listMarkdownFiles(authorPath)) {
    const basename = path.posix.basename(relativePath).toLowerCase();
    if (basename === "index.md") {
      excludedIndexes += 1;
      continue;
    }
    if (representedPaths.has(relativePath.toLowerCase()) || representedPaths.has(basename)) {
      continue;
    }
    const markdown = fs.readFileSync(path.join(authorPath, ...relativePath.split("/")), "utf8");
    const inferred = inferOrphanChapter(relativePath, markdown);
    if (inferred !== null) {
      inferredChapters.push(inferred);
    }
  }

  const manifest = parseManifest(buildZhihuManifest({
    authorId: author.author_id,
    authorName: author.author_name,
    generatedAt,
    items: archivedItems,
    inferredChapters,
  }));
  const manifestPath = path.join(authorPath, "manifest.json");
  if (input.write) {
    writeManifestAtomic(manifestPath, manifest);
  }
  return {
    manifestPath,
    databaseItems: archivedItems.length,
    inferredItems: inferredChapters.length,
    missingItems,
    excludedIndexes,
  };
}
