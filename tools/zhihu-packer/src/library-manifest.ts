import { createHash } from "node:crypto";
import * as path from "node:path";

import type { BookManifest, Chapter } from "../../../packages/contracts/dist/index.js";

export type ArchivedItem = {
  readonly id: string;
  readonly authorId: string;
  readonly authorName: string;
  readonly title: string;
  readonly createdTime: number;
  readonly voteCount: number;
  readonly outputPath: string;
  readonly wordCount?: number;
};

export type ManifestBuildInput = {
  readonly authorId: string;
  readonly authorName: string;
  readonly generatedAt: string;
  readonly items: readonly ArchivedItem[];
  readonly inferredChapters: readonly Chapter[];
};

function normalizedRelativePath(outputPath: string): string {
  return path.basename(outputPath.replaceAll("\\", "/"));
}

function itemDate(createdTime: number): string | undefined {
  if (!Number.isFinite(createdTime) || createdTime <= 0) {
    return undefined;
  }
  return new Date(createdTime * 1000).toISOString().slice(0, 10);
}

function chapterFromItem(item: ArchivedItem): Chapter {
  const date = itemDate(item.createdTime);
  return {
    id: item.id,
    path: normalizedRelativePath(item.outputPath),
    title: item.title.trim() || normalizedRelativePath(item.outputPath).replace(/\.md$/i, ""),
    ...(date === undefined ? {} : { date }),
    voteCount: Math.max(0, item.voteCount),
    wordCount: Math.max(0, item.wordCount ?? 0),
    metadataStatus: "complete",
  };
}

export function inferOrphanChapter(relativePath: string, markdown: string): Chapter | null {
  const normalized = relativePath.replaceAll("\\", "/");
  if (path.posix.basename(normalized).toLowerCase() === "index.md") {
    return null;
  }
  const markdownHeading = markdown.match(/^#\s+(.+)$/m)?.[1]?.trim();
  const htmlHeading = markdown.match(/<h1[^>]*>([\s\S]*?)<\/h1>/i)?.[1]
    ?.replace(/<[^>]+>/g, "")
    .trim();
  const title = markdownHeading || htmlHeading || path.posix.basename(normalized).replace(/\.md$/i, "");
  const filenameDate = path.posix.basename(normalized).match(/^(\d{4}-\d{2}-\d{2})(?:[-_]|$)/)?.[1];
  const date = filenameDate !== undefined
    && new Date(`${filenameDate}T00:00:00.000Z`).toISOString().slice(0, 10) === filenameDate
    ? filenameDate
    : undefined;
  const id = createHash("sha256").update(normalized.toLowerCase()).digest("hex").slice(0, 16);
  return {
    id: `inferred:${id}`,
    path: normalized,
    title,
    ...(date === undefined ? {} : { date }),
    voteCount: 0,
    wordCount: markdown.replace(/\s+/g, "").length,
    metadataStatus: "inferred",
  };
}

export function buildZhihuManifest(input: ManifestBuildInput): BookManifest {
  const chapters = input.items.map(chapterFromItem);
  const representedPaths = new Set(chapters.map((chapter) => chapter.path.toLowerCase()));
  for (const chapter of input.inferredChapters) {
    if (!representedPaths.has(chapter.path.toLowerCase())) {
      chapters.push(chapter);
      representedPaths.add(chapter.path.toLowerCase());
    }
  }
  return {
    schemaVersion: 1,
    bookId: `zhihu:${input.authorId}`,
    title: `${input.authorName} · 知乎归档`,
    source: "zhihu",
    sourceId: input.authorId,
    generatedAt: input.generatedAt,
    updatedAt: input.generatedAt,
    chapters,
  };
}
