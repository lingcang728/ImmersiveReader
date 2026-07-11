import { writable } from 'svelte/store';
import { themes, applyTheme, type Theme } from '$lib/theme/themes';

export const currentFilePath = writable<string | null>(null);
export const markdownSource = writable<string>('');
export const renderedHtml = writable<string>('');
export const isLoading = writable<boolean>(false);

// Theme
let savedTheme: string | null = null;
try {
	savedTheme = typeof localStorage !== 'undefined' ? localStorage.getItem('mmbook-theme') : null;
} catch {
	// localStorage may be disabled or quota exceeded
}
const defaultTheme = savedTheme
	? themes.find((t) => t.name === savedTheme) || themes[0]
	: themes[0];

export const currentTheme = writable<Theme>(defaultTheme);

currentTheme.subscribe((theme) => {
	if (typeof document !== 'undefined') {
		applyTheme(theme);
		try {
			localStorage.setItem('mmbook-theme', theme.name);
		} catch {
			// localStorage may be disabled or quota exceeded — silently skip
		}
	}
});

// Font scale (reader zoom). Clamped, persisted, applied as a CSS variable so
// focus mode can derive its own capped enlargement from the same value.
export const FONT_SCALE_MIN = 0.8;
export const FONT_SCALE_MAX = 1.5;
export const FONT_SCALE_STEP = 0.05;

export function clampFontScale(value: number): number {
	if (!Number.isFinite(value)) return 1;
	return Math.round(Math.min(FONT_SCALE_MAX, Math.max(FONT_SCALE_MIN, value)) * 100) / 100;
}

let savedFontScale = 1;
try {
	const raw = typeof localStorage !== 'undefined' ? localStorage.getItem('mmbook-font-scale') : null;
	if (raw !== null) savedFontScale = clampFontScale(Number.parseFloat(raw));
} catch {
	// localStorage may be disabled — fall back to default scale
}

export const fontScale = writable<number>(savedFontScale);

fontScale.subscribe((value) => {
	if (typeof document !== 'undefined') {
		document.documentElement.style.setProperty('--font-scale', String(value));
		try {
			localStorage.setItem('mmbook-font-scale', String(value));
		} catch {
			// localStorage may be disabled — silently skip
		}
	}
});

// Typography (reader controls): line height, column width, font family.
// Each persists to localStorage and applies as a CSS variable.
export const READING_LINE_HEIGHTS = [1.6, 1.8, 2.0] as const;
export const READING_WIDTHS = [680, 760, 840] as const;
export type ReadingFontFamily = 'sans' | 'serif';

const SERIF_FONT_STACK =
	'Georgia, "Source Han Serif SC", "Noto Serif SC", "STSong", "SimSun", serif';

function loadChoice<T>(key: string, valid: readonly T[], fallback: T, parse: (raw: string) => T): T {
	try {
		const raw = typeof localStorage !== 'undefined' ? localStorage.getItem(key) : null;
		if (raw !== null) {
			const value = parse(raw);
			if ((valid as readonly unknown[]).includes(value)) return value;
		}
	} catch {
		// localStorage may be disabled — fall back
	}
	return fallback;
}

function applyTypographyVar(key: string, cssVar: string, cssValue: string, persisted: string) {
	if (typeof document === 'undefined') return;
	document.documentElement.style.setProperty(cssVar, cssValue);
	try {
		localStorage.setItem(key, persisted);
	} catch {
		// localStorage may be disabled — silently skip
	}
}

export const readingLineHeight = writable<number>(
	loadChoice('mmbook-line-height', READING_LINE_HEIGHTS, 1.8, Number),
);
readingLineHeight.subscribe((v) =>
	applyTypographyVar('mmbook-line-height', '--article-line-height', String(v), String(v)),
);

export const readingWidth = writable<number>(
	loadChoice('mmbook-width', READING_WIDTHS, 760, Number),
);
readingWidth.subscribe((v) =>
	applyTypographyVar('mmbook-width', '--article-max-width', `${v}px`, String(v)),
);

export const readingFontFamily = writable<ReadingFontFamily>(
	loadChoice<ReadingFontFamily>(
		'mmbook-font-family',
		['sans', 'serif'],
		'sans',
		(raw) => raw as ReadingFontFamily,
	),
);
readingFontFamily.subscribe((v) =>
	applyTypographyVar(
		'mmbook-font-family',
		'--article-font-family',
		v === 'serif' ? SERIF_FONT_STACK : 'inherit',
		v,
	),
);

// Focus mode
export const focusMode = writable<boolean>(false);

// Search
export const searchOpen = writable<boolean>(false);
export const searchQuery = writable<string>('');

// TOC
export const tocOpen = writable<boolean>(false);

// Settings panel
export const settingsOpen = writable<boolean>(false);
