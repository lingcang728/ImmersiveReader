import { get, writable } from 'svelte/store';
import { themes, applyTheme, type Theme } from '$lib/theme/themes';

export const currentFilePath = writable<string | null>(null);
export const markdownSource = writable<string>('');
export const renderedHtml = writable<string>('');
export const isLoading = writable<boolean>(false);

// Theme
const THEME_STORAGE_KEY = 'mmbook-theme';

let savedTheme: string | null = null;
try {
	savedTheme =
		typeof localStorage !== 'undefined' ? localStorage.getItem(THEME_STORAGE_KEY) : null;
} catch {
	// localStorage may be disabled or quota exceeded
}

/** Same-family (identical label) variant of `theme` in the requested scheme. */
export function themeForScheme(theme: Theme, scheme: 'light' | 'dark'): Theme {
	return themes.find((t) => t.label === theme.label && t.scheme === scheme) ?? theme;
}

function systemPrefersDark(): boolean {
	try {
		return (
			typeof window !== 'undefined' &&
			typeof window.matchMedia === 'function' &&
			window.matchMedia('(prefers-color-scheme: dark)').matches
		);
	} catch {
		return false;
	}
}

/**
 * Boot theme: a valid stored choice wins; otherwise follow the OS
 * prefers-color-scheme within the default 素纸 family.
 */
export function resolveInitialTheme(savedName: string | null, prefersDark: boolean): Theme {
	const saved = savedName ? themes.find((t) => t.name === savedName) : undefined;
	return saved ?? themeForScheme(themes[0], prefersDark ? 'dark' : 'light');
}

// Only a stored value that names a real theme counts as an explicit user choice.
const hasSavedTheme = savedTheme !== null && themes.some((t) => t.name === savedTheme);
const defaultTheme = resolveInitialTheme(savedTheme, systemPrefersDark());

export const currentTheme = writable<Theme>(defaultTheme);

// While set, the next store emission must NOT be written to localStorage.
// It covers the system-derived initial value and every OS-driven switch via
// applySystemTheme — persisting those would masquerade as an explicit user
// choice and permanently stop the theme from following prefers-color-scheme.
let themePersistSuspended = !hasSavedTheme;

currentTheme.subscribe((theme) => {
	const suspended = themePersistSuspended;
	themePersistSuspended = false;
	if (typeof document === 'undefined') return;
	applyTheme(theme);
	if (suspended) return;
	try {
		localStorage.setItem(THEME_STORAGE_KEY, theme.name);
	} catch {
		// localStorage may be disabled or quota exceeded — silently skip
	}
});

/**
 * Apply a system-driven scheme switch. Goes through the store so all
 * subscribers see the new theme, but the write is flagged non-persisting:
 * 'mmbook-theme' stays absent and the app keeps following the OS scheme.
 */
export function applySystemTheme(theme: Theme): void {
	themePersistSuspended = true;
	currentTheme.set(theme);
}

// Follow prefers-color-scheme changes only while the user has no explicit
// stored choice; once 'mmbook-theme' holds a valid theme name the listener
// leaves the store alone. SSR/test builds without matchMedia keep the
// static default.
try {
	if (typeof window !== 'undefined' && typeof window.matchMedia === 'function') {
		const schemeMedia = window.matchMedia('(prefers-color-scheme: dark)');
		const onSchemeChange = (event: MediaQueryListEvent) => {
			let stored: string | null = null;
			try {
				stored = localStorage.getItem(THEME_STORAGE_KEY);
			} catch {
				return; // cannot tell whether the user chose — do not fight them
			}
			if (stored !== null && themes.some((t) => t.name === stored)) return;
			const current = get(currentTheme);
			const next = themeForScheme(current, event.matches ? 'dark' : 'light');
			if (next !== current) applySystemTheme(next);
		};
		if (typeof schemeMedia.addEventListener === 'function') {
			schemeMedia.addEventListener('change', onSchemeChange);
		} else {
			// Legacy Safari (<14): MediaQueryList.addListener passes the list
			// itself, which also exposes .matches.
			(
				schemeMedia as unknown as {
					addListener?: (listener: (event: MediaQueryListEvent) => void) => void;
				}
			).addListener?.(onSchemeChange);
		}
	}
} catch {
	// matchMedia unavailable — keep the static default
}

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
let savedFocusMode = false;
try {
	const raw = typeof localStorage !== 'undefined' ? localStorage.getItem('mmbook-focus-mode') : null;
	if (raw !== null) savedFocusMode = raw === 'true';
} catch {
	// localStorage may be disabled — fall back to default
}

export const focusMode = writable<boolean>(savedFocusMode);

focusMode.subscribe((value) => {
	try {
		localStorage.setItem('mmbook-focus-mode', String(value));
	} catch {
		// localStorage may be disabled — silently skip
	}
});

// Auto-focus: automatically enter focus mode when opening an article.
let savedAutoFocusMode = false;
try {
	const raw =
		typeof localStorage !== 'undefined' ? localStorage.getItem('mmbook-auto-focus') : null;
	if (raw !== null) savedAutoFocusMode = raw === 'true';
} catch {
	// localStorage may be disabled — fall back to default
}

export const autoFocusMode = writable<boolean>(savedAutoFocusMode);

autoFocusMode.subscribe((value) => {
	try {
		localStorage.setItem('mmbook-auto-focus', String(value));
	} catch {
		// localStorage may be disabled — silently skip
	}
});

// Search
export const searchOpen = writable<boolean>(false);
export const searchQuery = writable<string>('');

// TOC
export const tocOpen = writable<boolean>(false);

// Settings panel
export const settingsOpen = writable<boolean>(false);
