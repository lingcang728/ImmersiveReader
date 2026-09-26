export const BOOK_SOURCES = ["zhihu", "manual", "podcast"] as const;

export type BookSource = (typeof BOOK_SOURCES)[number];

export type Chapter = {
  readonly id: string;
  readonly path: string;
  readonly title: string;
  readonly date?: string;
  readonly voteCount: number;
  /**
   * Non-whitespace Unicode scalar value (code point) count — the canonical
   * unit shared with the Rust backend (`str::chars()`). Never UTF-16 code
   * units: astral characters must count once.
   */
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
  // Optional since P2-29: the current Rust loader migrates v1 files by
  // `libraryRoot` alone, so the schema no longer requires these legacy keys.
  readonly companionRoot?: string;
  readonly temporaryRoots?: readonly TemporaryRoot[];
};

export type LegacyAppSettingsV2 = {
  readonly schemaVersion: 2;
  readonly libraryRoot: string;
};

/**
 * `provenance.json` written next to `manifest.json` by the publish paths
 * (Rust `podcast/publish.rs::write_book_metadata`, zhihu-packer
 * `publish.ts::writeMetadata`). Shape mirrors `BookProvenance` in
 * `apps/desktop/src-tauri/src/library.rs` and `schemas/provenance.schema.json`.
 */
export type BookProvenance = {
  readonly schemaVersion: 1;
  readonly bookId: string;
  readonly sourceId: string;
  readonly sourceKind: "zhihu" | "podcast";
  readonly createdByTaskId: string;
  readonly lastSuccessfulTaskId: string;
  readonly revision: number;
  readonly manifestSha256: string;
  readonly engineVersion: string;
  readonly updatedAt: string;
};

/**
 * Phases of the publish transaction journal as serialized by Rust
 * (`#[serde(rename_all = "snake_case")]` on `PublishPhase`).
 */
export type PublishPhase = "prepared" | "old_moved" | "new_moved" | "committed" | "rolled_back";

/**
 * `.transactions/<transactionId>.json` written by Rust
 * `publish/transaction.rs`. Mirrors `PublishTransaction` and
 * `schemas/publish-transaction.schema.json`. (The zhihu-packer journal is a
 * separate, similar contract — it carries `authorId`/`sourceId` in addition
 * to `transactionId`/`taskId`, so it does not validate against this schema's
 * `additionalProperties: false`.)
 */
export type PublishTransaction = {
  readonly schemaVersion: 1;
  readonly transactionId: string;
  readonly taskId: string;
  readonly bookId: string;
  readonly incomingRelativePath: string;
  readonly finalRelativePath: string;
  readonly rollbackRelativePath: string;
  readonly manifestSha256: string;
  readonly provenanceSha256: string;
  readonly revision: number;
  readonly phase: PublishPhase;
  readonly createdAt: string;
  readonly updatedAt: string;
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
  // Blank check uses the Unicode White_Space property so it agrees with Rust
  // `str::trim` exactly — JS `trim` would accept NEL-only strings and reject
  // FEFF-only ones that Rust treats the opposite way.
  if (typeof value !== "string" || !/\P{White_Space}/u.test(value)) {
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

const DAYS_IN_MONTH = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31] as const;

function isLeapYear(year: number): boolean {
  return year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
}

/**
 * Mirrors `is_iso_calendar_date` in contracts.rs: `YYYY-MM-DD` shape plus a
 * real proleptic-Gregorian calendar day. `Date.parse` must never be used
 * here — it normalizes out-of-range fields (`2026-02-30` parses as Mar 2).
 */
function isIsoCalendarDate(text: string): boolean {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(text);
  if (match === null) {
    return false;
  }
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  if (month < 1 || month > 12) {
    return false;
  }
  const daysInMonth = DAYS_IN_MONTH[month - 1] ?? 0;
  const maxDay = daysInMonth + (month === 2 && isLeapYear(year) ? 1 : 0);
  return daysInMonth > 0 && day >= 1 && day <= maxDay;
}

function requireIsoDate(value: unknown, field: string): string {
  const text = requireString(value, field);
  if (!isIsoCalendarDate(text)) {
    throw new ContractParseError(field, "must be an ISO-8601 calendar date");
  }
  return text;
}

// Same shape as the `manifest.schema.json` date-time pattern and the Rust
// `is_rfc3339_date_time`: strict RFC-3339 — uppercase `T`, `Z` or `±HH:MM`
// offset, optional non-empty fraction.
const RFC3339_DATE_TIME =
  /^(\d{4}-\d{2}-\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d+)?(Z|([+-])(\d{2}):(\d{2}))$/;

function requireIsoDateTime(value: unknown, field: string): string {
  const text = requireString(value, field);
  const match = RFC3339_DATE_TIME.exec(text);
  // Mirror `is_rfc3339_date_time`: the date part must be a real calendar day
  // (`Date.parse` would roll `2026-02-30` over to March), the clock fields
  // are range-checked (rejects `24:00` and the leap-second `:60`), and a
  // numeric offset must be `±00:00..=23:59`.
  const valid =
    match !== null &&
    isIsoCalendarDate(match[1] ?? "") &&
    Number(match[2]) < 24 &&
    Number(match[3]) < 60 &&
    Number(match[4]) < 60 &&
    (match[5] === "Z" || (Number(match[7]) <= 23 && Number(match[8]) <= 59));
  if (!valid) {
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

/**
 * Windows device basenames that must never name a path segment. Compared
 * case-insensitively against the portion of the segment before the first `.`
 * (so `CON`, `con.md` and `LPT1.tar.md` are all rejected).
 */
const RESERVED_SEGMENT_BASENAMES: ReadonlySet<string> = new Set([
  "con",
  "prn",
  "aux",
  "nul",
  "com1",
  "com2",
  "com3",
  "com4",
  "com5",
  "com6",
  "com7",
  "com8",
  "com9",
  "lpt1",
  "lpt2",
  "lpt3",
  "lpt4",
  "lpt5",
  "lpt6",
  "lpt7",
  "lpt8",
  "lpt9",
]);

// Win32-forbidden characters inside a segment: `\`, NUL, `:`, `<`, `>`,
// `|`, `?`, `*`. `/` is excluded implicitly — it is the segment separator.
const FORBIDDEN_SEGMENT_CHAR = /[\\:\x00<>|?*]/;

function isSafePathSegment(segment: string): boolean {
  if (segment.length === 0 || segment === "." || segment === "..") {
    return false;
  }
  // A trailing `.`/` ` is silently normalized away by Win32 — the on-disk
  // name would no longer match the manifest.
  if (segment.endsWith(".") || segment.endsWith(" ")) {
    return false;
  }
  if (FORBIDDEN_SEGMENT_CHAR.test(segment)) {
    return false;
  }
  const dot = segment.indexOf(".");
  const basename = dot === -1 ? segment : segment.slice(0, dot);
  return !RESERVED_SEGMENT_BASENAMES.has(basename.toLowerCase());
}

/**
 * Non-throwing predicate mirroring `is_safe_relative_path` in contracts.rs and
 * the `path` pattern in manifest.schema.json — keep all three in lockstep:
 * forward-slash relative paths only; non-blank; no leading `/`; no drive
 * prefix; and every `/`-separated segment must be non-empty, not `.`/`..`,
 * free of `\` NUL `:` `<` `>` `|` `?` `*`, not a reserved device basename
 * (CON/PRN/AUX/NUL/COM1-9/LPT1-9, case-insensitive, before the first `.`),
 * and not end in `.` or ` `.
 */
export function isSafeRelativePath(value: string): boolean {
  // Blank check uses the Unicode White_Space property so it agrees with
  // Rust `str::trim` exactly (JS `trim` additionally strips FEFF and misses
  // NEL).
  if (!/\P{White_Space}/u.test(value)) {
    return false;
  }
  if (value.startsWith("/") || /^[A-Za-z]:/.test(value)) {
    return false;
  }
  return value.split("/").every(isSafePathSegment);
}

function requireRelativePath(value: unknown, field: string): string {
  if (typeof value !== "string" || !isSafeRelativePath(value)) {
    throw new ContractParseError(field, "must be a safe forward-slash relative path");
  }
  return value;
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
  // Chapter paths are compared case-insensitively: the library lives on
  // case-insensitive NTFS, so `a.md` and `A.MD` would collide on disk.
  const paths = new Set<string>();
  for (const chapter of chapters) {
    if (ids.has(chapter.id)) {
      throw new ContractParseError("chapters", `duplicate chapter id ${chapter.id}`);
    }
    ids.add(chapter.id);
    const pathKey = chapter.path.toLowerCase();
    if (paths.has(pathKey)) {
      throw new ContractParseError("chapters", `duplicate chapter path ${chapter.path}`);
    }
    paths.add(pathKey);
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

/**
 * Mirrors `resolve_current` in `progress.rs` — the Rust load path repairs a
 * dangling `current` (a chapter removed after `.reading.json` was written)
 * instead of reporting it: re-point to the first unread chapter, fall back
 * to the first chapter, and reset `position`. `validateReadingState` still
 * rejects a dangling `current` — exactly like Rust `validate_reading` — so
 * consumers that want the tolerate-and-fix load behavior must compose
 * `parseReadingState` → `resolveCurrent` → `validateReadingState`.
 */
export function resolveCurrent(state: ReadingState, manifest: BookManifest): ReadingState {
  if (manifest.chapters.some((chapter) => chapter.id === state.current)) {
    return state;
  }
  const firstUnread =
    manifest.chapters.find((chapter) => !state.read.includes(chapter.id)) ?? manifest.chapters[0];
  return {
    ...state,
    current: firstUnread?.id ?? "",
    position: 0,
  };
}

/**
 * Mirrors `progress_value` in `library.rs` (and `calculateBookProgress` in
 * the frontend): `read` is treated as a union of chapter ids — duplicates
 * and ids not present in the manifest never count, so a dirty `read` array
 * cannot inflate progress. `current` contributes its `position` only when
 * it exists in the manifest and is not already read.
 */
export function calculateOverallProgress(manifest: BookManifest, state: ReadingState): number {
  if (manifest.chapters.length === 0) {
    return 0;
  }
  const chapterIds = new Set(manifest.chapters.map((chapter) => chapter.id));
  const readIds = new Set(state.read.filter((id) => chapterIds.has(id)));
  const currentContribution =
    chapterIds.has(state.current) && !readIds.has(state.current) ? state.position : 0;
  return Math.min(1, (readIds.size + currentContribution) / manifest.chapters.length);
}

// ---------------------------------------------------------------------------
// EPUB contracts — publication.json sidecar + reader.db locator record.
// Mirrors `Publication`/`NavItem` in apps/desktop/src-tauri/src/epub.rs and
// `ReaderLocator`/`ReaderAnchor` in contracts.rs. Keep in lockstep.
// ---------------------------------------------------------------------------

/** One navigation entry (EPUB 3 nav.xhtml or EPUB 2 NCX). */
export type EpubNavItem = {
  readonly title: string;
  /** Chapter id in `manifest.chapters` this entry opens. */
  readonly chapterId: string;
  readonly children?: readonly EpubNavItem[];
};

/**
 * `publication.json` — versioned sidecar describing a non-Markdown book;
 * books lacking the file keep being treated as Markdown.
 */
export type Publication = {
  readonly schemaVersion: 1;
  /** Always "epub" for this revision. */
  readonly format: "epub";
  /** "2" or "3" as declared by the OPF package version. */
  readonly epubVersion: "2" | "3";
  readonly title: string;
  readonly creator?: string;
  readonly language?: string;
  /** Safe-relative path to the cover image inside the book dir, if any. */
  readonly cover?: string;
  readonly nav: readonly EpubNavItem[];
  /** Spine order as chapter ids; mirrors `manifest.chapters` order. */
  readonly spine: readonly string[];
  /** Format-specific extras (resource map id→book-relative path). */
  readonly resources: Readonly<Record<string, string>>;
  /** Declared-but-unrendered features: fixed-layout / media-overlay / scripted. */
  readonly unsupported: readonly string[];
};

/**
 * Where inside a chapter the locator points. `element` is a DOM element path
 * (a document path, not a filesystem path — the relative-path rules do NOT
 * apply), `text` is a quote plus its code-point offset into the chapter's
 * plain text, and `ratio` is the bare scroll-position fallback.
 */
export type ReaderAnchor =
  | { readonly kind: "element"; readonly path: string }
  | { readonly kind: "text"; readonly quote: string; readonly offset: number }
  | { readonly kind: "ratio" };

/** Last-read position record stored per book in reader.db. */
export type ReaderLocator = {
  readonly schemaVersion: 1;
  readonly bookId: string;
  readonly chapterId: string;
  readonly anchor: ReaderAnchor;
  /** Scroll fraction 0..1 — fallback when the anchor misses. */
  readonly ratio: number;
  /** RFC-3339 date-time of the last locator write. */
  readonly updated: string;
};

function parseNavItem(value: unknown, field: string): EpubNavItem {
  const record = requireRecord(value, field);
  rejectUnknownFields(record, ["title", "chapterId", "children"], field);
  if (record.children !== undefined && !Array.isArray(record.children)) {
    throw new ContractParseError(`${field}.children`, "must be an array");
  }
  return {
    title: requireString(record.title, `${field}.title`),
    chapterId: requireString(record.chapterId, `${field}.chapterId`),
    ...(record.children === undefined
      ? {}
      : {
          children: (record.children as readonly unknown[]).map((child, index) =>
            parseNavItem(child, `${field}.children[${index}]`),
          ),
        }),
  };
}

/**
 * Mirrors `Publication` deserialization + `validate_publication` in
 * contracts.rs — shape plus the cross-field rule JSON Schema cannot express:
 * every nav `chapterId` must appear in `spine`.
 */
export function parsePublication(value: unknown): Publication {
  const record = requireRecord(value, "publication");
  rejectUnknownFields(
    record,
    [
      "schemaVersion",
      "format",
      "epubVersion",
      "title",
      "creator",
      "language",
      "cover",
      "nav",
      "spine",
      "resources",
      "unsupported",
    ],
    "publication",
  );
  if (record.format !== "epub") {
    throw new ContractParseError("format", "must be epub");
  }
  if (record.epubVersion !== "2" && record.epubVersion !== "3") {
    throw new ContractParseError("epubVersion", "must be 2 or 3");
  }
  if (!Array.isArray(record.nav)) {
    throw new ContractParseError("nav", "must be an array");
  }
  if (!Array.isArray(record.spine) || record.spine.length === 0) {
    throw new ContractParseError("spine", "must contain at least one chapter id");
  }
  const spine = record.spine.map((item, index) => requireString(item, `spine[${index}]`));
  if (new Set(spine).size !== spine.length) {
    throw new ContractParseError("spine", "must not contain duplicate chapter ids");
  }
  const spineIds = new Set(spine);
  const nav = record.nav.map((item, index) => parseNavItem(item, `nav[${index}]`));
  const checkNavRefs = (items: readonly EpubNavItem[]): void => {
    for (const item of items) {
      if (!spineIds.has(item.chapterId)) {
        throw new ContractParseError(
          "nav",
          `chapterId references a chapter outside the spine: ${item.chapterId}`,
        );
      }
      checkNavRefs(item.children ?? []);
    }
  };
  checkNavRefs(nav);
  if (record.resources === undefined || !isRecord(record.resources)) {
    throw new ContractParseError("resources", "must be an object");
  }
  const resources: Record<string, string> = {};
  for (const [key, path] of Object.entries(record.resources)) {
    if (!/\P{White_Space}/u.test(key)) {
      throw new ContractParseError("resources", "resource id must be a non-empty string");
    }
    resources[key] = requireRelativePath(path, `resources.${key}`);
  }
  if (!Array.isArray(record.unsupported)) {
    throw new ContractParseError("unsupported", "must be an array");
  }
  const unsupported = record.unsupported.map((item, index) =>
    requireString(item, `unsupported[${index}]`),
  );
  const creator =
    record.creator === undefined ? undefined : requireString(record.creator, "creator");
  const language =
    record.language === undefined ? undefined : requireString(record.language, "language");
  const cover =
    record.cover === undefined
      ? undefined
      : requireRelativePath(record.cover, "cover");
  return {
    schemaVersion: requireSchemaV1(record.schemaVersion, "schemaVersion"),
    format: record.format,
    epubVersion: record.epubVersion,
    title: requireString(record.title, "title"),
    ...(creator === undefined ? {} : { creator }),
    ...(language === undefined ? {} : { language }),
    ...(cover === undefined ? {} : { cover }),
    nav,
    spine,
    resources,
    unsupported,
  };
}

function parseReaderAnchor(value: unknown, field: string): ReaderAnchor {
  const record = requireRecord(value, field);
  switch (record.kind) {
    case "element":
      rejectUnknownFields(record, ["kind", "path"], field);
      return { kind: "element", path: requireString(record.path, `${field}.path`) };
    case "text":
      rejectUnknownFields(record, ["kind", "quote", "offset"], field);
      return {
        kind: "text",
        quote: requireString(record.quote, `${field}.quote`),
        offset: requireNonNegativeInteger(record.offset, `${field}.offset`),
      };
    case "ratio":
      rejectUnknownFields(record, ["kind"], field);
      return { kind: "ratio" };
    default:
      throw new ContractParseError(`${field}.kind`, "must be element, text, or ratio");
  }
}

/**
 * Mirrors `ReaderLocator` deserialization + `validate_reader_locator` in
 * contracts.rs. There is no manifest cross-check — `chapterId` is
 * shape-checked only (the record is stored per book).
 */
export function parseReaderLocator(value: unknown): ReaderLocator {
  const record = requireRecord(value, "readerLocator");
  rejectUnknownFields(
    record,
    ["schemaVersion", "bookId", "chapterId", "anchor", "ratio", "updated"],
    "readerLocator",
  );
  const ratio = requireNonNegativeNumber(record.ratio, "ratio");
  if (ratio > 1) {
    throw new ContractParseError("ratio", "must be between 0 and 1");
  }
  return {
    schemaVersion: requireSchemaV1(record.schemaVersion, "schemaVersion"),
    bookId: requireString(record.bookId, "bookId"),
    chapterId: requireString(record.chapterId, "chapterId"),
    anchor: parseReaderAnchor(record.anchor, "anchor"),
    ratio,
    updated: requireIsoDateTime(record.updated, "updated"),
  };
}
