import type { TocItem } from '$lib/render/markdown';

export type BookSource = 'zhihu' | 'manual' | 'podcast';

export interface BookChapter {
	id: string;
	path: string;
	title: string;
	date?: string;
	voteCount: number;
	wordCount: number;
}

export interface BookManifest {
	schemaVersion: 1;
	bookId: string;
	title: string;
	source: BookSource;
	sourceId?: string;
	generatedAt: string;
	updatedAt: string;
	chapters: BookChapter[];
}

export interface ReadingState {
	schemaVersion: 1;
	current: string;
	position: number;
	read: string[];
	updated: string;
}

export interface BookSummary {
	bookId: string;
	title: string;
	source: BookSource;
	chapterCount: number;
	readCount: number;
	progress: number;
	currentChapterTitle?: string;
	updatedAt: string;
	lastReadAt?: string;
}

export interface LibraryIssue {
	path: string;
	message: string;
}

export interface LibraryScan {
	books: BookSummary[];
	issues: LibraryIssue[];
	writable: boolean;
}

export interface TemporaryItem {
	source: 'podcast';
	title: string;
	path: string;
	modifiedAt?: string;
}

export interface BookDetail {
	manifest: BookManifest;
	progress: ReadingState;
}

export interface AppSettings {
	schemaVersion: 2;
	libraryRoot: string;
}

export function chapterTocItems(chapters: readonly BookChapter[]): TocItem[] {
	return chapters.map((chapter) => ({
		id: chapter.id,
		text: chapter.title,
		level: 1,
	}));
}

export function resolveChapterIndex(
	chapters: readonly BookChapter[],
	current: string,
	read: readonly string[],
): number {
	const currentIndex = chapters.findIndex((chapter) => chapter.id === current);
	if (currentIndex >= 0) return currentIndex;

	const readIds = new Set(read);
	const unreadIndex = chapters.findIndex((chapter) => !readIds.has(chapter.id));
	if (unreadIndex >= 0) return unreadIndex;
	return chapters.length > 0 ? 0 : -1;
}

export function calculateBookProgress(
	chapters: readonly BookChapter[],
	state: ReadingState,
): number {
	if (chapters.length === 0) return 0;
	const chapterIds = new Set(chapters.map((chapter) => chapter.id));
	const readIds = new Set(state.read.filter((id) => chapterIds.has(id)));
	const currentIsUnread = chapterIds.has(state.current) && !readIds.has(state.current);
	const currentPosition = currentIsUnread ? Math.min(1, Math.max(0, state.position)) : 0;
	return Math.min(1, (readIds.size + currentPosition) / chapters.length);
}

export function nextReadingState(
	state: ReadingState,
	currentChapterId: string,
	position: number,
	completed: boolean,
): ReadingState {
	const readIds = new Set(state.read);
	if (completed) readIds.add(currentChapterId);
	return {
		schemaVersion: 1,
		current: currentChapterId,
		position: Math.min(1, Math.max(0, position)),
		read: [...readIds],
		updated: new Date().toISOString(),
	};
}
