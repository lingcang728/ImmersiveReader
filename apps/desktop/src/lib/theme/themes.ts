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
			'--bg-secondary': '#f0ebe3',
			'--text': '#222222',
			'--text-secondary': '#5a5a5a',
			'--text-faded': '#7a7a7a',
			'--heading': '#141414',
			'--link': '#3f6499',
			'--link-hover': '#2a4d82',
			'--code-bg': '#ece7df',
			'--code-text': '#333333',
			'--blockquote-border': '#c8c2b6',
			'--blockquote-text': '#4e4e4e',
			'--hr': '#d0cbc1',
			'--selection': 'rgba(74, 111, 165, 0.18)',
			'--focus-fade': 'rgba(250, 248, 245, 0.7)',
			'--scrollbar': '#c4beb2',
			'--scrollbar-hover': '#a8a193',
			'--bookmark': 'rgba(74, 111, 165, 0.3)',
			'--search-highlight': 'rgba(255, 210, 80, 0.4)',
			'--table-border': '#d0cbc1',
			'--table-stripe': '#f3efe8',
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
			'--bg': '#1a1a18',
			'--bg-secondary': '#252522',
			'--text': '#e2ddd4',
			'--text-secondary': '#a7a39a',
			'--text-faded': '#7d7972',
			'--heading': '#f0ebe3',
			'--link': '#8bb4df',
			'--link-hover': '#a9c9eb',
			'--code-bg': '#222220',
			'--code-text': '#d4d0c8',
			'--blockquote-border': '#4a4742',
			'--blockquote-text': '#b0aba2',
			'--hr': '#43403b',
			'--selection': 'rgba(123, 164, 212, 0.24)',
			'--focus-fade': 'rgba(26, 26, 24, 0.75)',
			'--scrollbar': '#4a4742',
			'--scrollbar-hover': '#5c5852',
			'--bookmark': 'rgba(123, 164, 212, 0.32)',
			'--search-highlight': 'rgba(255, 210, 80, 0.28)',
			'--table-border': '#43403b',
			'--table-stripe': '#20201e',
			'--spotlight-color': 'rgba(200, 195, 180, 0.06)',
			'--spotlight-vignette': 'rgba(0, 0, 0, 0.25)',
			'--focus-text-glow': 'rgba(226, 221, 212, 0.12)'
		}
	},
	{
		name: 'moshi-light',
		label: '墨石',
		shikiTheme: 'github-light',
		vars: {
			'--bg': '#ffffff',
			'--bg-secondary': '#f0f0f0',
			'--text': '#151515',
			'--text-secondary': '#4a4a4a',
			'--text-faded': '#777777',
			'--heading': '#000000',
			'--link': '#005db8',
			'--link-hover': '#004a94',
			'--code-bg': '#ebebeb',
			'--code-text': '#2a2a2a',
			'--blockquote-border': '#bdbdbd',
			'--blockquote-text': '#4a4a4a',
			'--hr': '#cfcfcf',
			'--selection': 'rgba(0, 102, 204, 0.14)',
			'--focus-fade': 'rgba(255, 255, 255, 0.7)',
			'--scrollbar': '#bdbdbd',
			'--scrollbar-hover': '#999999',
			'--bookmark': 'rgba(0, 102, 204, 0.25)',
			'--search-highlight': 'rgba(255, 200, 50, 0.4)',
			'--table-border': '#cfcfcf',
			'--table-stripe': '#f5f5f5',
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
			'--bg': '#121212',
			'--bg-secondary': '#1c1c1c',
			'--text': '#d2d2d2',
			'--text-secondary': '#959595',
			'--text-faded': '#6e6e6e',
			'--heading': '#efefef',
			'--link': '#7ab8f5',
			'--link-hover': '#9cc9ff',
			'--code-bg': '#1a1a1a',
			'--code-text': '#c4c4c4',
			'--blockquote-border': '#444444',
			'--blockquote-text': '#a0a0a0',
			'--hr': '#3a3a3a',
			'--selection': 'rgba(108, 172, 236, 0.18)',
			'--focus-fade': 'rgba(18, 18, 18, 0.75)',
			'--scrollbar': '#444444',
			'--scrollbar-hover': '#575757',
			'--bookmark': 'rgba(108, 172, 236, 0.28)',
			'--search-highlight': 'rgba(255, 200, 50, 0.22)',
			'--table-border': '#3a3a3a',
			'--table-stripe': '#171717',
			'--spotlight-color': 'rgba(180, 180, 200, 0.07)',
			'--spotlight-vignette': 'rgba(0, 0, 0, 0.3)',
			'--focus-text-glow': 'rgba(210, 210, 210, 0.15)'
		}
	},
	{
		name: 'muguang-light',
		label: '暮光',
		shikiTheme: 'github-light',
		vars: {
			'--bg': '#fdf6ec',
			'--bg-secondary': '#f2e8d6',
			'--text': '#342a22',
			'--text-secondary': '#6a5e50',
			'--text-faded': '#8f8170',
			'--heading': '#221910',
			'--link': '#7d582c',
			'--link-hover': '#5f411f',
			'--code-bg': '#ebe1cf',
			'--code-text': '#3f3428',
			'--blockquote-border': '#c9bca6',
			'--blockquote-text': '#5c5144',
			'--hr': '#d4c7b3',
			'--selection': 'rgba(139, 101, 53, 0.16)',
			'--focus-fade': 'rgba(253, 246, 236, 0.7)',
			'--scrollbar': '#c9bca6',
			'--scrollbar-hover': '#b0a28c',
			'--bookmark': 'rgba(139, 101, 53, 0.3)',
			'--search-highlight': 'rgba(255, 190, 60, 0.35)',
			'--table-border': '#d4c7b3',
			'--table-stripe': '#f6ecdc',
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
			'--bg': '#18150f',
			'--bg-secondary': '#231f18',
			'--text': '#d6cbb8',
			'--text-secondary': '#a39684',
			'--text-faded': '#7a6f60',
			'--heading': '#eee3cf',
			'--link': '#d0a66a',
			'--link-hover': '#e0ba84',
			'--code-bg': '#1f1b14',
			'--code-text': '#c8bca8',
			'--blockquote-border': '#4a4234',
			'--blockquote-text': '#b0a492',
			'--hr': '#40382c',
			'--selection': 'rgba(196, 152, 92, 0.22)',
			'--focus-fade': 'rgba(24, 21, 15, 0.75)',
			'--scrollbar': '#4a4234',
			'--scrollbar-hover': '#5c5344',
			'--bookmark': 'rgba(196, 152, 92, 0.32)',
			'--search-highlight': 'rgba(255, 190, 60, 0.22)',
			'--table-border': '#40382c',
			'--table-stripe': '#1c1913',
			'--spotlight-color': 'rgba(196, 152, 92, 0.06)',
			'--spotlight-vignette': 'rgba(0, 0, 0, 0.28)',
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
}
