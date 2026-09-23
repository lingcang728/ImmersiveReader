import { afterEach, describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';
import { themes } from '../theme/themes';
import {
	applySystemTheme,
	currentTheme,
	resolveInitialTheme,
	themeForScheme,
} from './app';

describe('themeForScheme', () => {
	it('returns the same-family variant in the requested scheme', () => {
		const moshiLight = themes.find((t) => t.name === 'moshi-light')!;
		const moshiDark = themes.find((t) => t.name === 'moshi-dark')!;
		expect(themeForScheme(moshiLight, 'dark')).toBe(moshiDark);
		expect(themeForScheme(moshiDark, 'light')).toBe(moshiLight);
	});

	it('returns the theme itself when already in the requested scheme', () => {
		const suzhiLight = themes.find((t) => t.name === 'suzhi-light')!;
		expect(themeForScheme(suzhiLight, 'light')).toBe(suzhiLight);
	});
});

describe('resolveInitialTheme', () => {
	it('defaults to suzhi light when nothing is stored and OS is light', () => {
		expect(resolveInitialTheme(null, false).name).toBe('suzhi-light');
	});

	it('defaults to suzhi dark when nothing is stored and OS is dark', () => {
		expect(resolveInitialTheme(null, true).name).toBe('suzhi-dark');
	});

	it('honours a valid stored choice regardless of OS scheme', () => {
		expect(resolveInitialTheme('muguang-dark', false).name).toBe('muguang-dark');
		expect(resolveInitialTheme('moshi-light', true).name).toBe('moshi-light');
	});

	it('treats an unknown stored name like no choice and follows the OS', () => {
		expect(resolveInitialTheme('not-a-theme', true).name).toBe('suzhi-dark');
	});
});

describe('applySystemTheme', () => {
	it('drives the store without throwing in a DOM-less environment', () => {
		const muguangDark = themes.find((t) => t.name === 'muguang-dark')!;
		applySystemTheme(muguangDark);
		expect(get(currentTheme)).toBe(muguangDark);
	});
});

describe('system scheme following', () => {
	afterEach(() => {
		vi.unstubAllGlobals();
	});

	function stubDom(storage: Map<string, string>, prefersDark: boolean) {
		let listener: ((event: { matches: boolean }) => void) | undefined;
		vi.stubGlobal('window', {
			matchMedia: vi.fn(() => ({
				matches: prefersDark,
				addEventListener: (
					_type: string,
					cb: (event: { matches: boolean }) => void,
				) => {
					listener = cb;
				},
			})),
		});
		vi.stubGlobal('localStorage', {
			getItem: (key: string) => (storage.has(key) ? storage.get(key)! : null),
			setItem: (key: string, value: string) => void storage.set(key, String(value)),
		});
		vi.stubGlobal('document', {
			documentElement: {
				style: { setProperty: () => {}, colorScheme: '' },
				dataset: {},
			},
		});
		return {
			emit: (matches: boolean) => listener?.({ matches }),
		};
	}

	it('boots from the OS scheme and never persists system-driven picks', async () => {
		const storage = new Map<string, string>();
		const { emit } = stubDom(storage, true);
		vi.resetModules();
		const mod = await import('./app');

		// Dark OS + no stored choice → suzhi-dark, and crucially nothing is
		// written to localStorage so the app keeps following the system.
		expect(get(mod.currentTheme).name).toBe('suzhi-dark');
		expect(storage.has('mmbook-theme')).toBe(false);

		emit(false);
		expect(get(mod.currentTheme).name).toBe('suzhi-light');
		expect(storage.has('mmbook-theme')).toBe(false);
	});

	it('stops following the OS once the user picks a theme', async () => {
		const storage = new Map<string, string>();
		const { emit } = stubDom(storage, false);
		vi.resetModules();
		const mod = await import('./app');

		// User choice via the plain store write path (SettingsPanel assigns
		// $currentTheme the same way) persists and ends system following.
		const moshiLight = themes.find((t) => t.name === 'moshi-light')!;
		mod.currentTheme.set(moshiLight);
		expect(storage.get('mmbook-theme')).toBe('moshi-light');

		emit(true);
		expect(get(mod.currentTheme)).toBe(moshiLight);
	});

	it('keeps the current family when the OS scheme flips', async () => {
		const storage = new Map<string, string>();
		const { emit } = stubDom(storage, true);
		vi.resetModules();
		const mod = await import('./app');

		// System-driven family move: simulate the listener target by switching
		// through applySystemTheme, which must not persist either.
		const muguangDark = themes.find((t) => t.name === 'muguang-dark')!;
		mod.applySystemTheme(muguangDark);
		expect(storage.has('mmbook-theme')).toBe(false);

		emit(false);
		expect(get(mod.currentTheme).name).toBe('muguang-light');
		expect(storage.has('mmbook-theme')).toBe(false);
	});

	it('ignores OS changes when a stored theme exists at boot', async () => {
		const storage = new Map<string, string>([['mmbook-theme', 'moshi-dark']]);
		const { emit } = stubDom(storage, false);
		vi.resetModules();
		const mod = await import('./app');

		expect(get(mod.currentTheme).name).toBe('moshi-dark');
		emit(false);
		expect(get(mod.currentTheme).name).toBe('moshi-dark');
	});
});
