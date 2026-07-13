<script lang="ts">
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

	async function startResize(direction: ResizeDirection) {
		try {
			await getCurrentWebviewWindow().startResizeDragging(direction);
		} catch {
			/* web preview */
		}
	}
</script>

<div class="resize-layer" aria-hidden="true">
	{#each edges as edge (edge.dir)}
		<div
			class="resize-handle {edge.className}"
			role="presentation"
			on:mousedown|preventDefault={() => void startResize(edge.dir)}
		></div>
	{/each}
</div>

<style>
	.resize-layer {
		position: fixed;
		inset: 0;
		pointer-events: none;
		z-index: 100;
	}

	.resize-handle {
		position: absolute;
		pointer-events: auto;
	}

	.edge-n {
		top: 0;
		left: 6px;
		right: 6px;
		height: 4px;
		cursor: ns-resize;
	}
	.edge-s {
		bottom: 0;
		left: 6px;
		right: 6px;
		height: 4px;
		cursor: ns-resize;
	}
	.edge-e {
		top: 6px;
		right: 0;
		bottom: 6px;
		width: 4px;
		cursor: ew-resize;
	}
	.edge-w {
		top: 6px;
		left: 0;
		bottom: 6px;
		width: 4px;
		cursor: ew-resize;
	}
	.edge-ne {
		top: 0;
		right: 0;
		width: 8px;
		height: 8px;
		cursor: nesw-resize;
	}
	.edge-nw {
		top: 0;
		left: 0;
		width: 8px;
		height: 8px;
		cursor: nwse-resize;
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
</style>
