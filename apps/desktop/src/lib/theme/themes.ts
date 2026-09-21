export interface Theme {
	name: string;
	label: string;
	scheme: 'light' | 'dark';
	vars: Record<string, string>;
	shikiTheme: string;
}

export const themes: Theme[] = [
	{
		name: 'suzhi-light',
		label: '素纸',
		scheme: 'light',
		shikiTheme: 'github-light',
		vars: {
			'--bg': '#faf8f5',
			'--bg-secondary': '#f0ebe3',
			'--text': '#222222',
			'--text-secondary': '#5a5a5a',
			'--text-faded': '#6c6964',
			'--heading': '#141414',
			'--link': '#3f6499',
			'--link-hover': '#2a4d82',
			/* Text painted on top of --link/--accent fills. */
			'--on-accent': '#ffffff',
			/* Muted warm red — readable text on the light paper surface. */
			'--danger': '#a03528',
			'--code-bg': '#ece7df',
			'--code-text': '#333333',
			'--blockquote-border': '#c8c2b6',
			'--blockquote-text': '#4e4e4e',
			'--hr': '#d0cbc1',
			'--selection': 'rgba(74, 111, 165, 0.18)',
			'--scrollbar': '#c4beb2',
			'--scrollbar-hover': '#a8a193',
			'--search-highlight': 'rgba(255, 210, 80, 0.4)',
			'--table-border': '#d0cbc1',
			'--table-stripe': '#f3efe8',
			'--spotlight-color': 'rgba(0, 0, 0, 0.02)',
			'--focus-text-glow': 'rgba(0, 0, 0, 0)'
		}
	},
	{
		name: 'suzhi-dark',
		label: '素纸',
		scheme: 'dark',
		shikiTheme: 'github-dark',
		vars: {
			/* Deep gray (not pure black) — original suzhi-dark surface */
			'--bg': '#1e1e1c',
			'--bg-secondary': '#262624',
			'--text': '#e2ddd4',
			'--text-secondary': '#a7a39a',
			'--text-faded': '#95908a',
			'--heading': '#f0ebe3',
			'--link': '#8bb4df',
			'--link-hover': '#a9c9eb',
			'--on-accent': '#17181c',
			'--danger': '#d4a099',
			'--code-bg': '#252523',
			'--code-text': '#d4d0c8',
			'--blockquote-border': '#4a4742',
			'--blockquote-text': '#b0aba2',
			'--hr': '#3a3835',
			'--selection': 'rgba(123, 164, 212, 0.24)',
			'--scrollbar': '#4a4742',
			'--scrollbar-hover': '#5c5852',
			'--search-highlight': 'rgba(255, 210, 80, 0.28)',
			'--table-border': '#3a3835',
			'--table-stripe': '#232321',
			'--spotlight-color': 'rgba(200, 195, 180, 0.06)',
			'--focus-text-glow': 'rgba(226, 221, 212, 0.12)'
		}
	},
	{
		name: 'moshi-light',
		label: '墨石',
		scheme: 'light',
		shikiTheme: 'github-light',
		vars: {
			'--bg': '#ffffff',
			'--bg-secondary': '#f0f0f0',
			'--text': '#151515',
			'--text-secondary': '#4a4a4a',
			'--text-faded': '#6a6a6a',
			'--heading': '#000000',
			'--link': '#005db8',
			'--link-hover': '#004a94',
			'--on-accent': '#ffffff',
			'--danger': '#b3261e',
			'--code-bg': '#ebebeb',
			'--code-text': '#2a2a2a',
			'--blockquote-border': '#bdbdbd',
			'--blockquote-text': '#4a4a4a',
			'--hr': '#cfcfcf',
			'--selection': 'rgba(0, 102, 204, 0.14)',
			'--scrollbar': '#bdbdbd',
			'--scrollbar-hover': '#999999',
			'--search-highlight': 'rgba(255, 200, 50, 0.4)',
			'--table-border': '#cfcfcf',
			'--table-stripe': '#f5f5f5',
			'--spotlight-color': 'rgba(0, 0, 0, 0.02)',
			'--focus-text-glow': 'rgba(0, 0, 0, 0)'
		}
	},
	{
		name: 'moshi-dark',
		label: '墨石',
		scheme: 'dark',
		shikiTheme: 'github-dark',
		vars: {
			/* Deep gray surface, not pure black */
			'--bg': '#141414',
			'--bg-secondary': '#1c1c1c',
			'--text': '#d2d2d2',
			'--text-secondary': '#959595',
			'--text-faded': '#888888',
			'--heading': '#efefef',
			'--link': '#7ab8f5',
			'--link-hover': '#9cc9ff',
			'--on-accent': '#10151c',
			'--danger': '#d4a099',
			'--code-bg': '#1a1a1a',
			'--code-text': '#c4c4c4',
			'--blockquote-border': '#444444',
			'--blockquote-text': '#a0a0a0',
			'--hr': '#2a2a2a',
			'--selection': 'rgba(108, 172, 236, 0.18)',
			'--scrollbar': '#444444',
			'--scrollbar-hover': '#575757',
			'--search-highlight': 'rgba(255, 200, 50, 0.22)',
			'--table-border': '#2a2a2a',
			'--table-stripe': '#181818',
			'--spotlight-color': 'rgba(180, 180, 200, 0.07)',
			'--focus-text-glow': 'rgba(210, 210, 210, 0.15)'
		}
	},
	{
		name: 'muguang-light',
		label: '暮光',
		scheme: 'light',
		shikiTheme: 'github-light',
		vars: {
			'--bg': '#fdf6ec',
			'--bg-secondary': '#f2e8d6',
			'--text': '#342a22',
			'--text-secondary': '#6a5e50',
			'--text-faded': '#6f6455',
			'--heading': '#221910',
			'--link': '#7d582c',
			'--link-hover': '#5f411f',
			'--on-accent': '#ffffff',
			'--danger': '#9d3a2f',
			'--code-bg': '#ebe1cf',
			'--code-text': '#3f3428',
			'--blockquote-border': '#c9bca6',
			'--blockquote-text': '#5c5144',
			'--hr': '#d4c7b3',
			'--selection': 'rgba(139, 101, 53, 0.16)',
			'--scrollbar': '#c9bca6',
			'--scrollbar-hover': '#b0a28c',
			'--search-highlight': 'rgba(255, 190, 60, 0.35)',
			'--table-border': '#d4c7b3',
			'--table-stripe': '#f6ecdc',
			'--spotlight-color': 'rgba(139, 101, 53, 0.03)',
			'--focus-text-glow': 'rgba(0, 0, 0, 0)'
		}
	},
	{
		name: 'muguang-dark',
		label: '暮光',
		scheme: 'dark',
		shikiTheme: 'github-dark',
		vars: {
			/* Warm deep gray, not pure black */
			'--bg': '#1c1914',
			'--bg-secondary': '#24201a',
			'--text': '#d6cbb8',
			'--text-secondary': '#a39684',
			'--text-faded': '#918775',
			'--heading': '#eee3cf',
			'--link': '#d0a66a',
			'--link-hover': '#e0ba84',
			'--on-accent': '#241a10',
			'--danger': '#d4a099',
			'--code-bg': '#211e18',
			'--code-text': '#c8bca8',
			'--blockquote-border': '#4a4234',
			'--blockquote-text': '#b0a492',
			'--hr': '#302a20',
			'--selection': 'rgba(196, 152, 92, 0.22)',
			'--scrollbar': '#4a4234',
			'--scrollbar-hover': '#5c5344',
			'--search-highlight': 'rgba(255, 190, 60, 0.22)',
			'--table-border': '#302a20',
			'--table-stripe': '#1f1c16',
			'--spotlight-color': 'rgba(196, 152, 92, 0.06)',
			'--focus-text-glow': 'rgba(214, 203, 184, 0.12)'
		}
	}
];

export function getThemePairs(): { label: string; light: Theme; dark: Theme }[] {
	const pairs: { label: string; light: Theme; dark: Theme }[] = [];
	for (let i = 0; i < themes.length; i += 2) {
		pairs.push({
			label: themes[i].label,
			light: themes[i],
			dark: themes[i + 1]
		});
	}
	return pairs;
}

export function applyTheme(theme: Theme) {
	const root = document.documentElement;
	for (const [key, value] of Object.entries(theme.vars)) {
		root.style.setProperty(key, value);
	}
	// Lets CSS select the matching Shiki variant (light/dark vars are both
	// inlined on each token by the dual-theme highlighter).
	root.dataset.shiki = theme.shikiTheme;
	// Keep UA-rendered widgets (scrollbars, form controls) on the right scheme.
	root.style.colorScheme = theme.scheme;
}

/**
 * Token map for the 连读 iframe (tools/zhihu-packer reader-template.html).
 * The template keeps its own --*-color names; we translate the active theme
 * into its vocabulary so the continuous reader follows the app theme.
 */
export function flowThemeVars(theme: Theme): Record<string, string> {
	const v = theme.vars;
	return {
		'--bg-color': v['--bg'],
		'--bg-raised': v['--bg-secondary'],
		'--bg-elevated': `color-mix(in srgb, ${v['--bg-secondary']} 80%, ${v['--text']} 6%)`,
		'--card-bg': `color-mix(in srgb, ${v['--bg-secondary']} 72%, transparent)`,
		'--card-bg-active': `color-mix(in srgb, ${v['--bg-secondary']} 94%, transparent)`,
		'--border-color': `color-mix(in srgb, ${v['--hr']} 70%, transparent)`,
		'--border-color-active': `color-mix(in srgb, ${v['--link']} 40%, transparent)`,
		'--line': `color-mix(in srgb, ${v['--hr']} 70%, transparent)`,
		'--text-color': v['--text'],
		'--text-secondary': v['--text-secondary'],
		'--text-muted': v['--text-faded'],
		'--text-light': v['--heading'],
		'--accent-color': v['--link'],
		'--accent-glow': `color-mix(in srgb, ${v['--link']} 20%, transparent)`,
		'--danger-color': v['--danger'],
		'--on-accent': v['--on-accent']
	};
}
