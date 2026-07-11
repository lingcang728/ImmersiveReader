export interface Theme {
	name: string;
	label: string;
	vars: Record<string, string>;
	shikiTheme: string;
}

export const themes: Theme[] = [
	{
		name: 'suzhi-light',
		label: '素纸',
		shikiTheme: 'github-light',
		vars: {
			'--bg': '#faf8f5',
			'--bg-secondary': '#f2efe9',
			'--text': '#2c2c2c',
			'--text-secondary': '#6b6b6b',
			'--text-faded': '#a0a0a0',
			'--heading': '#1a1a1a',
			'--link': '#4a6fa5',
			'--link-hover': '#2d5090',
			'--code-bg': '#f0ede7',
			'--code-text': '#3c3c3c',
			'--blockquote-border': '#d4cfc5',
			'--blockquote-text': '#5a5a5a',
			'--hr': '#e0dcd4',
			'--selection': 'rgba(74, 111, 165, 0.15)',
			'--focus-fade': 'rgba(250, 248, 245, 0.7)',
			'--scrollbar': '#d4cfc5',
			'--scrollbar-hover': '#b8b2a6',
			'--bookmark': 'rgba(74, 111, 165, 0.3)',
			'--search-highlight': 'rgba(255, 210, 80, 0.4)',
			'--table-border': '#e0dcd4',
			'--table-stripe': '#f5f2ec',
			'--spotlight-color': 'rgba(0, 0, 0, 0.02)',
			'--spotlight-vignette': 'rgba(0, 0, 0, 0.06)',
			'--focus-text-glow': 'rgba(0, 0, 0, 0)'
		}
	},
	{
		name: 'suzhi-dark',
		label: '素纸',
		shikiTheme: 'github-dark',
		vars: {
			'--bg': '#1e1e1c',
			'--bg-secondary': '#262624',
			'--text': '#d4d0c8',
			'--text-secondary': '#8a8780',
			'--text-faded': '#5a5856',
			'--heading': '#e8e4dc',
			'--link': '#7ba4d4',
			'--link-hover': '#9dbde6',
			'--code-bg': '#252523',
			'--code-text': '#c8c4bc',
			'--blockquote-border': '#3a3835',
			'--blockquote-text': '#9a9690',
			'--hr': '#333130',
			'--selection': 'rgba(123, 164, 212, 0.2)',
			'--focus-fade': 'rgba(30, 30, 28, 0.75)',
			'--scrollbar': '#3a3835',
			'--scrollbar-hover': '#4a4845',
			'--bookmark': 'rgba(123, 164, 212, 0.3)',
			'--search-highlight': 'rgba(255, 210, 80, 0.25)',
			'--table-border': '#333130',
			'--table-stripe': '#232321',
			'--spotlight-color': 'rgba(200, 195, 180, 0.06)',
			'--spotlight-vignette': 'rgba(0, 0, 0, 0.25)',
			'--focus-text-glow': 'rgba(212, 208, 200, 0.12)'
		}
	},
	{
		name: 'moshi-light',
		label: '墨石',
		shikiTheme: 'github-light',
		vars: {
			'--bg': '#ffffff',
			'--bg-secondary': '#f5f5f5',
			'--text': '#1a1a1a',
			'--text-secondary': '#555555',
			'--text-faded': '#999999',
			'--heading': '#000000',
			'--link': '#0066cc',
			'--link-hover': '#0044aa',
			'--code-bg': '#f0f0f0',
			'--code-text': '#333333',
			'--blockquote-border': '#cccccc',
			'--blockquote-text': '#555555',
			'--hr': '#e0e0e0',
			'--selection': 'rgba(0, 102, 204, 0.12)',
			'--focus-fade': 'rgba(255, 255, 255, 0.7)',
			'--scrollbar': '#cccccc',
			'--scrollbar-hover': '#aaaaaa',
			'--bookmark': 'rgba(0, 102, 204, 0.25)',
			'--search-highlight': 'rgba(255, 200, 50, 0.4)',
			'--table-border': '#e0e0e0',
			'--table-stripe': '#f8f8f8',
			'--spotlight-color': 'rgba(0, 0, 0, 0.02)',
			'--spotlight-vignette': 'rgba(0, 0, 0, 0.05)',
			'--focus-text-glow': 'rgba(0, 0, 0, 0)'
		}
	},
	{
		name: 'moshi-dark',
		label: '墨石',
		shikiTheme: 'github-dark',
		vars: {
			'--bg': '#141414',
			'--bg-secondary': '#1c1c1c',
			'--text': '#c0c0c0',
			'--text-secondary': '#707070',
			'--text-faded': '#484848',
			'--heading': '#e0e0e0',
			'--link': '#6cacec',
			'--link-hover': '#8dc4ff',
			'--code-bg': '#1a1a1a',
			'--code-text': '#b0b0b0',
			'--blockquote-border': '#333333',
			'--blockquote-text': '#888888',
			'--hr': '#2a2a2a',
			'--selection': 'rgba(108, 172, 236, 0.15)',
			'--focus-fade': 'rgba(20, 20, 20, 0.75)',
			'--scrollbar': '#333333',
			'--scrollbar-hover': '#444444',
			'--bookmark': 'rgba(108, 172, 236, 0.25)',
			'--search-highlight': 'rgba(255, 200, 50, 0.2)',
			'--table-border': '#2a2a2a',
			'--table-stripe': '#181818',
			'--spotlight-color': 'rgba(180, 180, 200, 0.07)',
			'--spotlight-vignette': 'rgba(0, 0, 0, 0.3)',
			'--focus-text-glow': 'rgba(192, 192, 192, 0.15)'
		}
	},
	{
		name: 'muguang-light',
		label: '暮光',
		shikiTheme: 'github-light',
		vars: {
			'--bg': '#fdf6ec',
			'--bg-secondary': '#f5eddf',
			'--text': '#3d3229',
			'--text-secondary': '#7a6e60',
			'--text-faded': '#a89a88',
			'--heading': '#2a2018',
			'--link': '#8b6535',
			'--link-hover': '#6d4c25',
			'--code-bg': '#f3ebdd',
			'--code-text': '#4a3d30',
			'--blockquote-border': '#d8ccb8',
			'--blockquote-text': '#6a5e50',
			'--hr': '#e4d8c8',
			'--selection': 'rgba(139, 101, 53, 0.15)',
			'--focus-fade': 'rgba(253, 246, 236, 0.7)',
			'--scrollbar': '#d8ccb8',
			'--scrollbar-hover': '#c0b4a0',
			'--bookmark': 'rgba(139, 101, 53, 0.3)',
			'--search-highlight': 'rgba(255, 190, 60, 0.35)',
			'--table-border': '#e4d8c8',
			'--table-stripe': '#f8f0e2',
			'--spotlight-color': 'rgba(139, 101, 53, 0.03)',
			'--spotlight-vignette': 'rgba(0, 0, 0, 0.06)',
			'--focus-text-glow': 'rgba(0, 0, 0, 0)'
		}
	},
	{
		name: 'muguang-dark',
		label: '暮光',
		shikiTheme: 'github-dark',
		vars: {
			'--bg': '#1c1914',
			'--bg-secondary': '#24201a',
			'--text': '#c8bca8',
			'--text-secondary': '#8a7e6e',
			'--text-faded': '#5a5044',
			'--heading': '#e0d4c0',
			'--link': '#c4985c',
			'--link-hover': '#d8ae72',
			'--code-bg': '#211e18',
			'--code-text': '#b8ac98',
			'--blockquote-border': '#3a3428',
			'--blockquote-text': '#968a78',
			'--hr': '#302a20',
			'--selection': 'rgba(196, 152, 92, 0.2)',
			'--focus-fade': 'rgba(28, 25, 20, 0.75)',
			'--scrollbar': '#3a3428',
			'--scrollbar-hover': '#4a4438',
			'--bookmark': 'rgba(196, 152, 92, 0.3)',
			'--search-highlight': 'rgba(255, 190, 60, 0.2)',
			'--table-border': '#302a20',
			'--table-stripe': '#1f1c16',
			'--spotlight-color': 'rgba(196, 152, 92, 0.06)',
			'--spotlight-vignette': 'rgba(0, 0, 0, 0.28)',
			'--focus-text-glow': 'rgba(200, 188, 168, 0.12)'
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
}
