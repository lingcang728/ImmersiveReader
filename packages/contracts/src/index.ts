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
  readonly schemaVersion: 3;
  readonly libraryRoot: string;
};

export type LegacyAppSettingsV1 = {
  readonly schemaVersion: 1;
  readonly libraryRoot: string;
  readonly companionRoot: string;
  readonly temporaryRoots: readonly TemporaryRoot[];
};

export type LegacyAppSettingsV2 = {
  readonly schemaVersion: 2;
  readonly libraryRoot: string;
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

function rejectUnknownFields(
  record: Readonly<Record<string, unknown>>,
  allowed: readonly string[],
  field: string,
): void {
  const allowedFields = new Set(allowed);
  for (const key of Object.keys(record)) {
    if (!allowedFields.has(key)) {
      throw new ContractParseError(`${field}.${key}`, "unknown field");
    }
  }
}

function requireNonNegativeNumber(value: unknown, field: string): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
    throw new ContractParseError(field, "must be a finite non-negative number");
  }
  return value;
}

function requireNonNegativeInteger(value: unknown, field: string): number {
  const number = requireNonNegativeNumber(value, field);
  if (!Number.isInteger(number)) {
    throw new ContractParseError(field, "must be a non-negative integer");
  }
  return number;
}

function requireIsoDate(value: unknown, field: string): string {
  const text = requireString(value, field);
  if (!/^\d{4}-\d{2}-\d{2}$/.test(text) || Number.isNaN(Date.parse(`${text}T00:00:00Z`))) {
    throw new ContractParseError(field, "must be an ISO-8601 calendar date");
  }
  return text;
}

function requireIsoDateTime(value: unknown, field: string): string {
  const text = requireString(value, field);
  if (
    !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.test(text) ||
    Number.isNaN(Date.parse(text))
  ) {
    throw new ContractParseError(field, "must be an RFC-3339 date-time");
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
  rejectUnknownFields(
    record,
    ["id", "path", "title", "date", "voteCount", "wordCount", "metadataStatus"],
    field,
  );
  const base = {
    id: requireString(record.id, `${field}.id`),
    path: requireRelativePath(record.path, `${field}.path`),
    title: requireString(record.title, `${field}.title`),
    voteCount: requireNonNegativeInteger(record.voteCount, `${field}.voteCount`),
    wordCount: requireNonNegativeInteger(record.wordCount, `${field}.wordCount`),
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
  rejectUnknownFields(
    record,
    ["schemaVersion", "bookId", "title", "source", "sourceId", "generatedAt", "updatedAt", "chapters"],
    "manifest",
  );
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
    generatedAt: requireIsoDateTime(record.generatedAt, "generatedAt"),
    updatedAt: requireIsoDateTime(record.updatedAt, "updatedAt"),
    chapters,
  };
}

export function parseReadingState(value: unknown): ReadingState {
  const record = requireRecord(value, "readingState");
  rejectUnknownFields(record, ["schemaVersion", "current", "position", "read", "updated"], "readingState");
  const position = requireNonNegativeNumber(record.position, "position");
  if (position > 1) {
    throw new ContractParseError("position", "must be between 0 and 1");
  }
  if (!Array.isArray(record.read)) {
    throw new ContractParseError("read", "must be an array");
  }
  const read = record.read.map((item, index) => requireString(item, `read[${index}]`));
  if (new Set(read).size !== read.length) {
    throw new ContractParseError("read", "must not contain duplicate chapter ids");
  }
  return {
    schemaVersion: requireSchemaV1(record.schemaVersion, "schemaVersion"),
    current: requireString(record.current, "current"),
    position,
    read,
    updated: requireIsoDateTime(record.updated, "updated"),
  };
}

export function validateReadingState(state: ReadingState, manifest: BookManifest): void {
  const chapterIds = new Set(manifest.chapters.map((chapter) => chapter.id));
  if (!chapterIds.has(state.current)) {
    throw new ContractParseError("current", "must reference a chapter in the manifest");
  }
  for (const [index, id] of state.read.entries()) {
    if (!chapterIds.has(id)) {
      throw new ContractParseError(`read[${index}]`, "must reference a chapter in the manifest");
    }
  }
}

export function calculateOverallProgress(chapterCount: number, state: ReadingState): number {
  if (!Number.isInteger(chapterCount) || chapterCount <= 0) {
    return 0;
  }
  const completed = Math.min(state.read.length, chapterCount);
  const currentContribution = state.read.includes(state.current) ? 0 : state.position;
  return Math.min(1, (completed + currentContribution) / chapterCount);
}
