<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
	import { emit } from '@tauri-apps/api/event';
	import { detectDevice } from '$lib/platform/device';

	export let visible = true;
	/** When true, chrome overlays content (immersive reading). */
	export let overlay = false;
	/** Fired only when maximized state actually changes. */
	export let onMaximizedChange: ((maximized: boolean) => void) | undefined = undefined;

	let isMobile = typeof window !== 'undefined' ? detectDevice().isMobile : false;

	let maximized = false;
	let unlistenResize: (() => void) | undefined;
	let resizeRaf = 0;

	// Window system menu (right-click / Shift+F10): custom chrome has no
	// native titlebar menu, so we provide the same operations ourselves.
	let menuOpen = false;
	let menuX = 0;
	let menuY = 0;
	let menuEl: HTMLElement | null = null;
	let menuRestoreFocus: HTMLElement | null = null;

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

	function requestAppExit() {
		// Same event the tray menu emits — the page-level listener runs the
		// full preserve/cancel flow (flush edits, guard unsaved work).
		void emit('request-app-exit', { mode: 'preserve' }).catch(() => {});
	}

	function openSystemMenu(x: number, y: number) {
		menuRestoreFocus =
			document.activeElement instanceof HTMLElement ? document.activeElement : null;
		menuX = Math.max(4, Math.min(x, window.innerWidth - 200));
		menuY = Math.max(4, Math.min(y, window.innerHeight - 190));
		menuOpen = true;
		void tick().then(() => {
			menuEl?.querySelector<HTMLElement>('[role="menuitem"]')?.focus();
		});
	}

	function closeSystemMenu() {
		if (!menuOpen) return;
		menuOpen = false;
		menuRestoreFocus?.focus();
		menuRestoreFocus = null;
	}

	function onTitlebarContextMenu(event: MouseEvent) {
		const target = event.target as HTMLElement | null;
		if (target?.closest('button, a, input, [data-no-drag]')) return;
		event.preventDefault();
		openSystemMenu(event.clientX, event.clientY);
	}

	function onChromeKeydown(event: KeyboardEvent) {
		const isMenuKey =
			event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey);
		if (!isMenuKey) return;
		event.preventDefault();
		const anchor = (document.activeElement as HTMLElement | null)?.getBoundingClientRect();
		openSystemMenu(anchor ? anchor.left : 24, anchor ? anchor.bottom + 4 : 36);
	}

	function onMenuKeydown(event: KeyboardEvent) {
		const items = menuEl
			? Array.from(menuEl.querySelectorAll<HTMLElement>('[role="menuitem"]'))
			: [];
		if (event.key === 'Escape' || event.key === 'Tab') {
			event.preventDefault();
			closeSystemMenu();
			return;
		}
		if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
			event.preventDefault();
			if (!items.length) return;
			const current = items.indexOf(document.activeElement as HTMLElement);
			const step = event.key === 'ArrowDown' ? 1 : -1;
			const next = current < 0 ? 0 : (current + step + items.length) % items.length;
			items[next].focus();
		} else if (event.key === 'Home') {
			event.preventDefault();
			items[0]?.focus();
		} else if (event.key === 'End') {
			event.preventDefault();
			items[items.length - 1]?.focus();
		}
	}

	function onMenuItem(action: 'minimize' | 'maximize' | 'hide' | 'exit') {
		closeSystemMenu();
		if (action === 'minimize') void minimize();
		else if (action === 'maximize') void toggleMaximize();
		else if (action === 'hide') void closeWindow();
		else requestAppExit();
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
		// P3-11: onResized's unlisten resolves asynchronously — if the
		// component is destroyed first, cleanup runs before `unlistenResize`
		// is ever assigned and the listener leaks. The disposed flag makes
		// teardown deterministic either way.
		let disposed = false;
		try {
			void getCurrentWebviewWindow()
				.onResized(() => {
					scheduleRefreshMaximized();
				})
				.then((fn) => {
					if (disposed) {
						fn();
					} else {
						unlistenResize = fn;
					}
				})
				.catch(() => {
					/* web preview */
				});
		} catch {
			/* Web preview without Tauri internals. */
		}
		const updateMobile = () => {
			isMobile = detectDevice().isMobile;
		};
		window.addEventListener('resize', updateMobile);

		return () => {
			disposed = true;
			window.removeEventListener('resize', updateMobile);
			if (resizeRaf) cancelAnimationFrame(resizeRaf);
			unlistenResize?.();
			unlistenResize = undefined;
		};
	});
</script>

{#if !isMobile}
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
	on:contextmenu={onTitlebarContextMenu}
	on:keydown={onChromeKeydown}
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
			title={maximized ? '还原' : '最大化（Win+方向键可贴靠分屏）'}
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
			aria-label="隐藏到托盘"
			title="隐藏到托盘（在托盘图标中退出）"
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

{#if menuOpen && visible}
	<!-- svelte-ignore a11y-click-events-have-key-events -->
	<!-- svelte-ignore a11y-no-static-element-interactions -->
	<div class="chrome-menu-backdrop" on:mousedown|preventDefault={closeSystemMenu} on:contextmenu|preventDefault={closeSystemMenu}></div>
	<div
		class="chrome-menu"
		role="menu"
		tabindex="-1"
		aria-label="窗口菜单"
		style="left: {menuX}px; top: {menuY}px;"
		bind:this={menuEl}
		on:keydown={onMenuKeydown}
	>
		<button type="button" role="menuitem" class="chrome-menu-item" on:click={() => onMenuItem('minimize')}>
			最小化
		</button>
		<button type="button" role="menuitem" class="chrome-menu-item" on:click={() => onMenuItem('maximize')}>
			{maximized ? '还原' : '最大化'}
		</button>
		<button type="button" role="menuitem" class="chrome-menu-item" on:click={() => onMenuItem('hide')}>
			隐藏到托盘
		</button>
		<div class="chrome-menu-sep" role="separator"></div>
		<button type="button" role="menuitem" class="chrome-menu-item" on:click={() => onMenuItem('exit')}>
			退出
		</button>
	</div>
{/if}
{/if}

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

	@media (max-width: 768px) {
		.window-chrome {
			display: none !important;
			height: 0 !important;
			min-height: 0 !important;
		}
		.chrome-controls {
			display: none !important;
		}
	}

	:global(.is-mobile) .window-chrome {
		display: none !important;
		height: 0 !important;
		min-height: 0 !important;
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

	.chrome-menu-backdrop {
		position: fixed;
		inset: 0;
		z-index: 998;
	}

	.chrome-menu {
		position: fixed;
		z-index: 999;
		min-width: 180px;
		padding: 5px;
		border: 1px solid var(--hr);
		border-radius: 8px;
		background: var(--bg-secondary);
		box-shadow: 0 10px 32px rgba(0, 0, 0, 0.3);
		display: flex;
		flex-direction: column;
	}

	.chrome-menu-item {
		display: block;
		width: 100%;
		border: 0;
		border-radius: 5px;
		background: transparent;
		color: var(--text);
		font-size: 12.5px;
		padding: 7px 12px;
		text-align: left;
		cursor: pointer;
	}

	.chrome-menu-item:hover {
		background: color-mix(in srgb, var(--text) 9%, transparent);
	}

	.chrome-menu-item:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: -2px;
	}

	.chrome-menu-sep {
		height: 1px;
		margin: 4px 8px;
		background: var(--hr);
	}
</style>
