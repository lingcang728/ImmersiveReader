// Shared contracts for the Android-only mobile surface.
//
// The mobile app is a fresh, phone-first reading shell. It reuses the same
// Rust backend commands and the same MMbook theme tokens as the desktop app,
// but deliberately does not import the desktop page/components. Keep this file
// free of Svelte/DOM code so both the shell and both readers can import it.

import type {
	BookChapter,
	BookDetail,
	BookSummary,
	LibraryIssue,
	ReadingState as BookReadingState,
} from '$lib/library/books';
import type { ReaderLocator } from '$lib/epub/types';

export type { BookChapter, BookDetail, BookSummary, BookReadingState, LibraryIssue };
export type { ReaderLocator };

/** What the reader screen is asked to open. */
export type ReaderTarget =
	| {
			kind: 'book';
			bookId: string;
			title: string;
			/** Optional chapter to jump to (TOC/open-with start chapter). */
			chapterId?: string;
	  }
	| {
			kind: 'file';
			path: string;
			title: string;
	  };

/** Top-level mobile route. The reader replaces the whole viewport. */
export type MobileView = 'shelf' | 'reader' | 'settings';

/** A library scan snapshot (same payload as `scan_library`). */
export interface MobileLibrarySnapshot {
	books: BookSummary[];
	issues: LibraryIssue[];
	writable: boolean;
}

/** Recent single-file Markdown entry (mirrors the desktop recent list). */
export interface RecentFile {
	path: string;
	name: string;
	openedAt: number;
}

/** Persisted scroll state for a single (non-library) Markdown file. */
export interface FileReadingState {
	scroll_position: number;
	bookmarks: number[];
	progress?: number;
}

/** `get_storage_usage` payload. */
export interface StorageUsage {
	libraryBytes: number;
	dataBytes: number;
	cacheBytes: number;
	logsBytes: number;
	backupsBytes: number;
	runtimeStateBytes: number;
}

/** `clear_safe_cache` payload. */
export interface CacheSkip {
	category: string;
	taskId?: string | null;
	path: string;
	reason: string;
}

export interface CacheClearResult {
	deletedItems: number;
	releasedBytes: number;
	skipped: CacheSkip[];
	protectedRootsVerified?: boolean;
}

/** `list_trash` entry. */
export interface TrashItem {
	readonly schemaVersion: number;
	readonly trashId: string;
	readonly bookId: string;
	readonly title: string;
	readonly originalRelativePath: string;
	readonly trashRelativePath: string;
	readonly deletedAt: string;
	readonly revision: number;
}

/** `get_platform_capabilities` (backend snake_case, kept raw for the shell). */
export interface RawPlatformCapabilities {
	platform?: string;
	reader_session?: boolean;
	content_uri_import?: boolean;
	epub?: boolean;
	volume_key_capture?: boolean;
	reading_bundle?: boolean;
	is_mobile?: boolean;
	task_acquisition?: boolean;
	directory_import?: boolean;
	secret_store?: boolean;
	markdown_editing?: boolean;
	supported_formats?: string[];
	[key: string]: unknown;
}

/** Reader theme/typography change notification for the shell. */
export interface ReaderExitInfo {
	bookId?: string;
	/** True when the reader persisted fresh progress; the shelf should rescan. */
	libraryChanged?: boolean;
}
