<script lang="ts">
	import { onMount } from 'svelte';
	import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';

	export let visible = true;
	/** When true, chrome overlays content (immersive reading). */
	export let overlay = false;
	/** Fired only when maximized state actually changes. */
	export let onMaximizedChange: ((maximized: boolean) => void) | undefined = undefined;

	let maximized = false;
	let unlistenResize: (() => void) | undefined;
	let resizeRaf = 0;

	async function refreshMaximized() {
		try {
			const next = await getCurrentWebviewWindow().isMaximized();
			if (next !== maximized) {
				maximized = next;
				onMaximizedChange?.(maximized);
			}
		} catch {
			// Web preview without Tauri.
		}
	}

	function scheduleRefreshMaximized() {
		if (resizeRaf) return;
		resizeRaf = requestAnimationFrame(() => {
			resizeRaf = 0;
			void refreshMaximized();
		});
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
		try {
			void getCurrentWebviewWindow()
				.onResized(() => {
					scheduleRefreshMaximized();
				})
				.then((fn) => {
					unlistenResize = fn;
				})
				.catch(() => {
					/* web preview */
				});
		} catch {
			/* Web preview without Tauri internals. */
		}
		return () => {
			if (resizeRaf) cancelAnimationFrame(resizeRaf);
			unlistenResize?.();
		};
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
	<!-- Drag surface only — brand lives in the app toolbar below. -->
	<div class="chrome-drag" aria-hidden="true"></div>
	<div class="chrome-controls" data-no-drag>
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
		--chrome-h: 32px;
		display: flex;
		align-items: center;
		justify-content: flex-end;
		height: var(--chrome-h);
		min-height: var(--chrome-h);
		padding: 0 4px 0 0;
		border-bottom: 0;
		background: color-mix(in srgb, var(--bg) 92%, var(--bg-secondary) 8%);
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

	.chrome-drag {
		flex: 1;
		align-self: stretch;
		min-width: 0;
	}

	.chrome-controls {
		display: flex;
		align-items: center;
		gap: 0;
		flex: none;
	}

	.chrome-btn {
		display: grid;
		place-items: center;
		width: 42px;
		height: 28px;
		border: 0;
		border-radius: 0;
		background: transparent;
		color: var(--text-secondary);
		cursor: pointer;
		transition:
			background 140ms ease,
			color 140ms ease;
	}

	.chrome-btn:hover {
		background: color-mix(in srgb, var(--text) 10%, transparent);
		color: var(--text);
	}

	.chrome-btn:active {
		transform: none;
	}

	.chrome-btn:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: -2px;
	}

	.chrome-btn-close:hover {
		background: #c42b1c;
		color: #fff;
	}
</style>
