<!--
	Inline SVG icon set for the mobile shell (stroke = currentColor).

	No icon library is added: the handful of glyphs the shell needs are one
	path each, which keeps the Android shell bundle small and lets the icons
	inherit the active MMbook theme token.
-->
<script module lang="ts">
	export type IconName =
		| 'search'
		| 'settings'
		| 'back'
		| 'close'
		| 'plus'
		| 'book'
		| 'chevron-right'
		| 'check'
		| 'trash'
		| 'refresh'
		| 'alert'
		| 'file'
		| 'folder'
		| 'more';
</script>

<script lang="ts">
	let {
		name,
		size = 22,
		strokeWidth = 1.8
	}: { name: IconName; size?: number; strokeWidth?: number } = $props();

	const PATHS: Record<IconName, string> = {
		search: 'M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14ZM16.5 16.5 21 21',
		settings: 'M4 7h9M17 7h3M9 4v6M4 17h3M11 17h9M15 14v6',
		back: 'M15 18l-6-6 6-6',
		close: 'M6 6l12 12M18 6L6 18',
		plus: 'M12 5v14M5 12h14',
		book: 'M5 5.5A1.5 1.5 0 0 1 6.5 4H16v16H6.5A1.5 1.5 0 0 1 5 18.5ZM16 4h3v16h-3',
		'chevron-right': 'M9 6l6 6-6 6',
		check: 'M5 13l4 4L19 7',
		trash: 'M4 7h16M9 7V5h6v2M7 7l1 13h8l1-13M10 11v6M14 11v6',
		refresh: 'M20 11a8 8 0 1 0-2.35 6.35M20 5v6h-6',
		alert: 'M12 4l9 16H3zM12 10v4M12 17h.01',
		file: 'M7 3h7l4 4v14H7zM14 3v5h5',
		folder: 'M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z',
		more: 'M6 12h.01M12 12h.01M18 12h.01'
	};
</script>

<svg
	viewBox="0 0 24 24"
	width={size}
	height={size}
	fill="none"
	stroke="currentColor"
	stroke-width={strokeWidth}
	stroke-linecap="round"
	stroke-linejoin="round"
	aria-hidden="true"
	focusable="false"
>
	<path d={PATHS[name]} />
</svg>
