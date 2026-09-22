<script lang="ts">
	import { onMount } from 'svelte';
	import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';

	type ResizeDirection =
		| 'East'
		| 'North'
		| 'NorthEast'
		| 'NorthWest'
		| 'South'
		| 'SouthEast'
		| 'SouthWest'
		| 'West';

	const edges: { dir: ResizeDirection; className: string }[] = [
		{ dir: 'North', className: 'edge-n' },
		{ dir: 'South', className: 'edge-s' },
		{ dir: 'East', className: 'edge-e' },
		{ dir: 'West', className: 'edge-w' },
		{ dir: 'NorthEast', className: 'edge-ne' },
		{ dir: 'NorthWest', className: 'edge-nw' },
		{ dir: 'SouthEast', className: 'edge-se' },
		{ dir: 'SouthWest', className: 'edge-sw' }
	];

	// P2-40: the edge strips sit above app content — they must disappear when
	// the window is maximized or fullscreen, where they only swallow clicks.
	let handlesEnabled = true;

	async function refreshHandleState() {
		if (typeof window !== 'undefined' && (window.innerWidth <= 768 || (typeof navigator !== 'undefined' && navigator.maxTouchPoints > 0 && /Android|iPhone|iPad|Mobile/i.test(navigator.userAgent)))) {
			handlesEnabled = false;
			return;
		}
		try {
			const win = getCurrentWebviewWindow();
			const [maximized, fullscreen] = await Promise.all([
				win.isMaximized(),
				win.isFullscreen()
			]);
			handlesEnabled = !(maximized || fullscreen);
		} catch {
			/* web preview: keep handles enabled */
		}
	}

	onMount(() => {
		void refreshHandleState();
		const onWindowResize = () => void refreshHandleState();
		window.addEventListener('resize', onWindowResize);
		return () => window.removeEventListener('resize', onWindowResize);
	});

	async function startResize(direction: ResizeDirection) {
		try {
			await getCurrentWebviewWindow().startResizeDragging(direction);
		} catch {
			/* web preview */
		}
	}
</script>

{#if handlesEnabled}
	{#each edges as edge (edge.dir)}
		<div
			class="resize-handle {edge.className}"
			role="presentation"
			aria-hidden="true"
			on:mousedown|preventDefault={() => void startResize(edge.dir)}
		></div>
	{/each}
{/if}

<style>
	/* P2-40: each handle carries its own z-index (no shared fixed layer, which
	   would force one stacking level for all of them).
	   - Side/bottom strips sit at 60: above page content and modal backdrops
	     (nav guard 45, hover zone 50, chrome stack 55) yet below search ticks
	     (90) so tick marks and the overlay scrollbar stay clickable.
	   - Top strips sit at 85: above window chrome (70) and the reading
	     progress line (80) so top-edge resize still works whether or not
	     chrome is visible, while remaining below search ticks (90). */
	.resize-handle {
		position: fixed;
		z-index: 60;
	}

	.edge-n {
		top: 0;
		left: 6px;
		right: 6px;
		height: 4px;
		cursor: ns-resize;
		z-index: 85;
	}
	.edge-s {
		bottom: 0;
		left: 6px;
		right: 6px;
		height: 4px;
		cursor: ns-resize;
	}
	/* East/West strips overlap the overlay scrollbar; 3px leaves most of a
	   ~6px scrollbar clickable while still being grabbable. */
	.edge-e {
		top: 6px;
		right: 0;
		bottom: 6px;
		width: 3px;
		cursor: ew-resize;
	}
	.edge-w {
		top: 6px;
		left: 0;
		bottom: 6px;
		width: 3px;
		cursor: ew-resize;
	}
	.edge-ne {
		top: 0;
		right: 0;
		width: 8px;
		height: 8px;
		cursor: nesw-resize;
		z-index: 85;
	}
	.edge-nw {
		top: 0;
		left: 0;
		width: 8px;
		height: 8px;
		cursor: nwse-resize;
		z-index: 85;
	}
	.edge-se {
		bottom: 0;
		right: 0;
		width: 8px;
		height: 8px;
		cursor: nwse-resize;
	}
	.edge-sw {
		bottom: 0;
		left: 0;
		width: 8px;
		height: 8px;
		cursor: nesw-resize;
	}
	@media (max-width: 768px) {
		.resize-edge {
			display: none !important;
		}
	}
</style>
