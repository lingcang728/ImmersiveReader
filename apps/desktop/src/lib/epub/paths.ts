// Pure path/href helpers for the EPUB reader. No DOM access — safe to unit
// test under vitest's node environment and to import anywhere.

import type { Publication } from "./types";

/** decodeURIComponent that never throws on malformed input. */
export function safeDecodeUri(value: string): string {
	try {
		return decodeURIComponent(value);
	} catch {
		return value;
	}
}

/**
 * Normalize a book-relative forward-slash path: collapse separators, drop
 * `.`, resolve `..` (clamped at the root), strip leading slashes, and
 * percent-decode segments. Returns "" for empty/nothing-left input.
 */
export function normalizeBookPath(path: string): string {
	const raw = safeDecodeUri(path.trim()).replace(/\\/g, "/");
	const segments: string[] = [];
	for (const part of raw.split("/")) {
		if (!part || part === ".") continue;
		if (part === "..") {
			// `..` past the root is clamped, not kept — containment checks
			// happen in the backend; here we only need a canonical key.
			segments.pop();
			continue;
		}
		segments.push(part);
	}
	return segments.join("/");
}

/** Join a book-relative directory and a relative ref, then normalize. */
export function joinBookPath(dir: string, rel: string): string {
	const base = normalizeBookPath(dir);
	return normalizeBookPath(base ? `${base}/${rel}` : rel);
}

const WINDOWS_ABS = /^[a-zA-Z]:[\\/]/;
const POSIX_ABS = /^[\\/]/;

export function isNativeAbsolutePath(path: string): boolean {
	return WINDOWS_ABS.test(path) || POSIX_ABS.test(path);
}

export function hasUriScheme(value: string): boolean {
	return /^[a-zA-Z][a-zA-Z\d+.-]*:/.test(value);
}

/** http/https URLs — the only schemes allowed to leave the reader. */
export function isExternalHttpUrl(value: string): boolean {
	return /^https?:\/\//i.test(value);
}

/** Protocol-relative `//host/path` resolves to remote http(s) in a webview. */
export function isProtocolRelative(value: string): boolean {
	return value.startsWith("//");
}

/** Local passthrough schemes that need no book-dir resolution. */
export function isInlineOrAssetUrl(value: string): boolean {
	return /^(data|blob|asset):/i.test(value) || /^https?:\/\/asset\.localhost\//i.test(value);
}

export function isWindowsAbsolutePath(path: string): boolean {
	return WINDOWS_ABS.test(path);
}

/**
 * Join a relative resource path onto a native base directory, using the
 * separator style of `baseDir` (mirrors render/images.ts behavior).
 */
export function joinNativePath(baseDir: string, rel: string): string {
	const sep = baseDir.includes("\\") ? "\\" : "/";
	const normalizedRel = rel.replace(/[\\/]+/g, sep);
	if (!baseDir) return normalizedRel;
	if (baseDir.endsWith("/") || baseDir.endsWith("\\")) return `${baseDir}${normalizedRel}`;
	return `${baseDir}${sep}${normalizedRel}`;
}

export interface SplitHref {
	/** Path part before `#`/`?`, percent-encoded as written. "" for pure fragments. */
	path: string;
	/** Raw query (`?a=b`) retained for cache-busting on asset URLs; "" when absent. */
	query: string;
	/** Percent-decoded fragment, "" when absent. */
	fragment: string;
}

/** Split `path/to.xhtml?x=1#frag` into its three parts. */
export function splitHref(href: string): SplitHref {
	const trimmed = href.trim();
	const hashIndex = trimmed.indexOf("#");
	const beforeHash = hashIndex === -1 ? trimmed : trimmed.slice(0, hashIndex);
	const rawFragment = hashIndex === -1 ? "" : trimmed.slice(hashIndex + 1);
	const queryIndex = beforeHash.indexOf("?");
	return {
		path: queryIndex === -1 ? beforeHash : beforeHash.slice(0, queryIndex),
		query: queryIndex === -1 ? "" : beforeHash.slice(queryIndex),
		fragment: safeDecodeUri(rawFragment),
	};
}

/**
 * Map of normalized book-relative content paths → spine chapterId.
 *
 * `publication.resources` is the package manifest's id→path map and spine
 * entries are manifest item ids, so `resources[chapterId]` gives the chapter
 * document's path relative to the book root. Entries are indexed under both
 * the normalized path and (defensively) under reversed key/value when the
 * value is a spine id — covers either serialization direction.
 */
export function buildChapterHrefMap(publication: Publication): Map<string, string> {
	const map = new Map<string, string>();
	const spine = publication.spine ?? [];
	const spineIds = new Set(spine);
	const resources = publication.resources ?? {};

	for (const [key, value] of Object.entries(resources)) {
		if (spineIds.has(key)) {
			const normalized = normalizeBookPath(value);
			if (normalized) map.set(normalized, key);
		}
		if (spineIds.has(value)) {
			const normalized = normalizeBookPath(key);
			if (normalized) map.set(normalized, value);
		}
	}
	// Direct id lookups: a hand-made href equal to a chapterId still resolves.
	for (const id of spineIds) {
		if (!map.has(id)) map.set(id, id);
	}
	return map;
}

export interface ChapterHrefTarget {
	chapterId: string;
	fragment: string;
}

/**
 * Resolve an in-chapter `<a href>` to a spine chapterId.
 * `resourceDir` is the chapter's book-relative directory from
 * `get_readable_chapter`. Returns null when the path lands outside the spine
 * (asset links, missing targets) — callers should leave those alone.
 */
export function resolveHrefToChapterId(
	chapterMap: ReadonlyMap<string, string>,
	resourceDir: string,
	href: string,
): ChapterHrefTarget | null {
	const { path, fragment } = splitHref(href);
	if (!path) return null;
	const decoded = safeDecodeUri(path);

	const candidates = [
		joinBookPath(resourceDir, decoded),
		normalizeBookPath(decoded),
		normalizeBookPath(decoded).split("/").pop() ?? "",
	];
	for (const candidate of candidates) {
		if (!candidate) continue;
		const chapterId = chapterMap.get(candidate);
		if (chapterId) return { chapterId, fragment };
	}
	return null;
}
