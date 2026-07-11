export const BOOK_SOURCES = ["zhihu", "manual", "podcast"] as const;

export type BookSource = (typeof BOOK_SOURCES)[number];

export type Chapter = {
  readonly id: string;
  readonly path: string;
  readonly title: string;
  readonly date?: string;
  readonly voteCount: number;
  readonly wordCount: number;
  readonly metadataStatus?: "complete" | "inferred";
};

export type BookManifest = {
  readonly schemaVersion: 1;
  readonly bookId: string;
  readonly title: string;
  readonly source: BookSource;
  readonly sourceId?: string;
  readonly generatedAt: string;
  readonly updatedAt: string;
  readonly chapters: readonly Chapter[];
};

export type ReadingState = {
  readonly schemaVersion: 1;
  readonly current: string;
  readonly position: number;
  readonly read: readonly string[];
  readonly updated: string;
};

export type TemporaryRoot = {
  readonly source: "podcast";
  readonly path: string;
};

export type AppSettings = {
  readonly schemaVersion: 1;
  readonly libraryRoot: string;
  readonly companionRoot: string;
  readonly temporaryRoots: readonly TemporaryRoot[];
};

export class ContractParseError extends Error {
  readonly name = "ContractParseError";
  readonly field: string;

  constructor(field: string, message: string) {
    super(`${field}: ${message}`);
    this.field = field;
  }
}

function isRecord(value: unknown): value is Readonly<Record<string, unknown>> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function requireRecord(value: unknown, field: string): Readonly<Record<string, unknown>> {
  if (!isRecord(value)) {
    throw new ContractParseError(field, "must be an object");
  }
  return value;
}

function requireString(value: unknown, field: string): string {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new ContractParseError(field, "must be a non-empty string");
  }
  return value;
}

function requireNonNegativeNumber(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
    throw new ContractParseError(field, "must be a finite non-negative number");
  }
  return value;
}

function requireIsoDate(value: unknown, field: string): string {
  const text = requireString(value, field);
  if (Number.isNaN(Date.parse(text))) {
    throw new ContractParseError(field, "must be an ISO-8601 date or timestamp");
  }
  return text;
}

function requireSchemaV1(value: unknown, field: string): 1 {
  if (value !== 1) {
    throw new ContractParseError(field, "unsupported schema version");
  }
  return 1;
}

function requireRelativePath(value: unknown, field: string): string {
  const text = requireString(value, field);
  const segments = text.split("/");
  const isDrivePath = /^[A-Za-z]:/.test(text);
  if (
    text.startsWith("/") ||
    isDrivePath ||
    text.includes("\\") ||
    text.includes("\0") ||
    segments.some((segment) => segment === "." || segment === ".." || segment.length === 0)
  ) {
    throw new ContractParseError(field, "must be a safe forward-slash relative path");
  }
  return text;
}

function parseSource(value: unknown, field: string): BookSource {
  if (value === "zhihu" || value === "manual" || value === "podcast") {
    return value;
  }
  throw new ContractParseError(field, "must be zhihu, manual, or podcast");
}

function parseChapter(value: unknown, index: number): Chapter {
  const field = `chapters[${index}]`;
  const record = requireRecord(value, field);
  const base = {
    id: requireString(record.id, `${field}.id`),
    path: requireRelativePath(record.path, `${field}.path`),
    title: requireString(record.title, `${field}.title`),
    voteCount: requireNonNegativeNumber(record.voteCount, `${field}.voteCount`),
    wordCount: requireNonNegativeNumber(record.wordCount, `${field}.wordCount`),
  };
  const date = record.date === undefined ? undefined : requireIsoDate(record.date, `${field}.date`);
  const metadataStatus = record.metadataStatus;
  if (metadataStatus !== undefined && metadataStatus !== "complete" && metadataStatus !== "inferred") {
    throw new ContractParseError(`${field}.metadataStatus`, "must be complete or inferred");
  }
  return {
    ...base,
    ...(date === undefined ? {} : { date }),
    ...(metadataStatus === undefined ? {} : { metadataStatus }),
  };
}

export function parseManifest(value: unknown): BookManifest {
  const record = requireRecord(value, "manifest");
  if (!Array.isArray(record.chapters) || record.chapters.length === 0) {
    throw new ContractParseError("chapters", "must contain at least one chapter");
  }
  const chapters = record.chapters.map(parseChapter);
  const ids = new Set<string>();
  for (const chapter of chapters) {
    if (ids.has(chapter.id)) {
      throw new ContractParseError("chapters", `duplicate chapter id ${chapter.id}`);
    }
    ids.add(chapter.id);
  }
  const sourceId = record.sourceId === undefined ? undefined : requireString(record.sourceId, "sourceId");
  return {
    schemaVersion: requireSchemaV1(record.schemaVersion, "schemaVersion"),
    bookId: requireString(record.bookId, "bookId"),
    title: requireString(record.title, "title"),
    source: parseSource(record.source, "source"),
    ...(sourceId === undefined ? {} : { sourceId }),
    generatedAt: requireIsoDate(record.generatedAt, "generatedAt"),
    updatedAt: requireIsoDate(record.updatedAt, "updatedAt"),
    chapters,
  };
}

export function parseReadingState(value: unknown): ReadingState {
  const record = requireRecord(value, "readingState");
  const position = requireNonNegativeNumber(record.position, "position");
  if (position > 1) {
    throw new ContractParseError("position", "must be between 0 and 1");
  }
  if (!Array.isArray(record.read)) {
    throw new ContractParseError("read", "must be an array");
  }
  const read = [...new Set(record.read.map((item, index) => requireString(item, `read[${index}]`)))];
  return {
    schemaVersion: requireSchemaV1(record.schemaVersion, "schemaVersion"),
    current: requireString(record.current, "current"),
    position,
    read,
    updated: requireIsoDate(record.updated, "updated"),
  };
}

export function calculateOverallProgress(chapterCount: number, state: ReadingState): number {
  if (!Number.isInteger(chapterCount) || chapterCount <= 0) {
    return 0;
  }
  const completed = Math.min(state.read.length, chapterCount);
  const currentContribution = state.read.includes(state.current) ? 0 : state.position;
  return Math.min(1, (completed + currentContribution) / chapterCount);
}
