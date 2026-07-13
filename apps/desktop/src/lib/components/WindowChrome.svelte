<script lang="ts">
	import { onMount } from 'svelte';
	import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';

	export let title = '';
	export let visible = true;
	/** When true, chrome overlays content (immersive reading). */
	export let overlay = false;

	let maximized = false;
	let unlistenResize: (() => void) | undefined;

	async function refreshMaximized() {
		try {
			maximized = await getCurrentWebviewWindow().isMaximized();
		} catch {
			// Web preview without Tauri.
		}
	}

	async function minimize() {
		try {
			await getCurrentWebviewWindow().minimize();
		} catch {
			/* noop */
		}
	}

	async function toggleMaximize() {
		try {
			await getCurrentWebviewWindow().toggleMaximize();
			await refreshMaximized();
		} catch {
			/* noop */
		}
	}

	async function closeWindow() {
		try {
			// Emits closeRequested so the existing save/exit path runs.
			await getCurrentWebviewWindow().close();
		} catch {
			/* noop */
		}
	}

	async function startDrag(event: MouseEvent) {
		if (event.button !== 0) return;
		const target = event.target as HTMLElement | null;
		if (target?.closest('button, a, input, [data-no-drag]')) return;
		try {
			await getCurrentWebviewWindow().startDragging();
		} catch {
			/* noop */
		}
	}

	function onTitlebarDblClick(event: MouseEvent) {
		const target = event.target as HTMLElement | null;
		if (target?.closest('button, a, input, [data-no-drag]')) return;
		void toggleMaximize();
	}

	onMount(() => {
		void refreshMaximized();
		void getCurrentWebviewWindow()
			.onResized(() => {
				void refreshMaximized();
			})
			.then((fn) => {
				unlistenResize = fn;
			})
			.catch(() => {
				/* web preview */
			});
		return () => unlistenResize?.();
	});
</script>

<header
	class="window-chrome"
	class:hidden={!visible}
	class:overlay
	class:maximized
	aria-label="窗口栏"
	aria-hidden={!visible}
	inert={!visible || undefined}
	on:mousedown={startDrag}
	on:dblclick={onTitlebarDblClick}
>
	<div class="chrome-leading" data-no-drag>
		<img class="app-icon" src="/favicon.png" alt="" width="16" height="16" draggable="false" />
		<span class="chrome-title">{title || '沉浸阅读'}</span>
	</div>
	<div class="chrome-controls" data-no-drag>
		<slot name="actions" />
		<button
			type="button"
			class="chrome-btn"
			aria-label="最小化"
			title="最小化"
			on:click={() => void minimize()}
		>
			<svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
				<path d="M2 6h8" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
			</svg>
		</button>
		<button
			type="button"
			class="chrome-btn"
			aria-label={maximized ? '还原' : '最大化'}
			title={maximized ? '还原' : '最大化'}
			on:click={() => void toggleMaximize()}
		>
			{#if maximized}
				<svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
					<path
						d="M3.5 4.5h5v5h-5zM4.5 3.5h4.5v4.5"
						fill="none"
						stroke="currentColor"
						stroke-width="1.1"
					/>
				</svg>
			{:else}
				<svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
					<rect
						x="2.5"
						y="2.5"
						width="7"
						height="7"
						fill="none"
						stroke="currentColor"
						stroke-width="1.1"
					/>
				</svg>
			{/if}
		</button>
		<button
			type="button"
			class="chrome-btn chrome-btn-close"
			aria-label="关闭"
			title="关闭"
			on:click={() => void closeWindow()}
		>
			<svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
				<path
					d="M3 3l6 6M9 3L3 9"
					stroke="currentColor"
					stroke-width="1.2"
					stroke-linecap="round"
				/>
			</svg>
		</button>
	</div>
</header>

<style>
	.window-chrome {
		--chrome-h: 36px;
		display: flex;
		align-items: center;
		justify-content: space-between;
		height: var(--chrome-h);
		min-height: var(--chrome-h);
		padding: 0 6px 0 12px;
		border-bottom: 1px solid var(--hr);
		background: color-mix(in srgb, var(--bg) 92%, var(--link) 8%);
		color: var(--text);
		user-select: none;
		flex: none;
		z-index: 70;
		transition:
			transform 200ms cubic-bezier(0.2, 0.8, 0.2, 1),
			opacity 200ms cubic-bezier(0.2, 0.8, 0.2, 1);
		will-change: transform, opacity;
	}

	.window-chrome.overlay {
		position: absolute;
		top: 0;
		left: 0;
		right: 0;
	}

	.window-chrome.hidden {
		transform: translateY(-100%);
		opacity: 0;
		pointer-events: none;
		visibility: hidden;
	}

	@media (prefers-reduced-motion: reduce) {
		.window-chrome {
			transition: none;
		}
	}

	.chrome-leading {
		display: flex;
		align-items: center;
		gap: 8px;
		min-width: 0;
		pointer-events: none;
	}

	.app-icon {
		width: 16px;
		height: 16px;
		flex: none;
		border-radius: 3px;
	}

	.chrome-title {
		font-size: 12px;
		font-weight: 500;
		letter-spacing: 0.01em;
		color: var(--text-secondary);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		max-width: min(52vw, 480px);
	}

	.chrome-controls {
		display: flex;
		align-items: center;
		gap: 2px;
		flex: none;
	}

	.chrome-btn {
		display: grid;
		place-items: center;
		width: 40px;
		height: 28px;
		border: 0;
		border-radius: 6px;
		background: transparent;
		color: var(--text-secondary);
		cursor: pointer;
		transition:
			background 140ms ease,
			color 140ms ease;
	}

	.chrome-btn:hover {
		background: color-mix(in srgb, var(--link) 14%, transparent);
		color: var(--text);
	}

	.chrome-btn:active {
		transform: translateY(1px);
	}

	.chrome-btn:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: 1px;
	}

	.chrome-btn-close:hover {
		background: #c42b1c;
		color: #fff;
	}
</style>
