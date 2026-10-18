<!--
	Bottom-sheet shell for every mobile reader panel.

	Owns: backdrop, drag handle, scroll lock inside the sheet body, safe-area
	padding, visualViewport keyboard inset (search input), Escape handling and
	enter/exit transitions. Panels only provide content (+ optional footer).
-->
<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		open: boolean;
		/** Required by the mobile QA harness. */
		testid: string;
		title?: string;
		/** 'auto' = content height capped at 80vh; '80' = 80vh; 'full' = near-fullscreen. */
		height?: 'auto' | '80' | 'full';
		onClose: () => void;
		children?: Snippet;
		footer?: Snippet;
	}

	let {
		open,
		testid,
		title = '',
		height = 'auto',
		onClose,
		children,
		footer
	}: Props = $props();

	let keyboardOffset = $state(0);
	const heightClass = $derived(height === '80' ? 'h-80' : height);

	function updateViewport() {
		if (typeof window === 'undefined' || !window.visualViewport) {
			keyboardOffset = 0;
			return;
		}
		const vv = window.visualViewport;
		const offset = Math.max(0, window.innerHeight - (vv.height + vv.offsetTop));
		keyboardOffset = offset > 80 ? offset : 0;
	}

	function handleKeydown(event: KeyboardEvent) {
		if (!open) return;
		if (event.key === 'Escape') {
			event.preventDefault();
			onClose();
		}
	}

	$effect(() => {
		if (!open) {
			keyboardOffset = 0;
			return;
		}
		updateViewport();
		window.addEventListener('keydown', handleKeydown);
		window.visualViewport?.addEventListener('resize', updateViewport);
		window.visualViewport?.addEventListener('scroll', updateViewport);
		return () => {
			window.removeEventListener('keydown', handleKeydown);
			window.visualViewport?.removeEventListener('resize', updateViewport);
			window.visualViewport?.removeEventListener('scroll', updateViewport);
		};
	});
</script>

{#if open}
	<div class="sheet-root" data-testid={testid}>
		<button type="button" class="sheet-backdrop" aria-label="关闭" onclick={onClose}></button>
		<div
			class="sheet-panel {heightClass}"
			role="dialog"
			aria-modal="true"
			aria-label={title || '面板'}
			style:bottom="{keyboardOffset}px"
		>
			<div class="sheet-grabber" aria-hidden="true"></div>
			{#if title}
				<header class="sheet-header">
					<h2 class="sheet-title">{title}</h2>
					<button type="button" class="sheet-close" aria-label="关闭" onclick={onClose}>
						<svg viewBox="0 0 24 24" aria-hidden="true">
							<path d="M6 6l12 12M18 6L6 18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
						</svg>
					</button>
				</header>
			{/if}
			<div class="sheet-body">
				{@render children?.()}
			</div>
			{#if footer}
				<div class="sheet-footer">{@render footer()}</div>
			{/if}
			<div class="sheet-safe" aria-hidden="true"></div>
		</div>
	</div>
{/if}

<style>
	.sheet-root {
		position: fixed;
		inset: 0;
		z-index: 60;
		display: flex;
		align-items: flex-end;
	}

	.sheet-backdrop {
		position: absolute;
		inset: 0;
		border: 0;
		padding: 0;
		background: rgba(0, 0, 0, 0.42);
		animation: sheetFade 160ms ease;
	}

	.sheet-panel {
		position: relative;
		z-index: 61;
		display: flex;
		flex-direction: column;
		width: 100%;
		max-height: 80dvh;
		min-height: 0;
		background: var(--bg);
		color: var(--text);
		border-radius: 20px 20px 0 0;
		border-top: 1px solid var(--hr);
		box-shadow: 0 -12px 40px rgba(0, 0, 0, 0.24);
		animation: sheetUp 200ms cubic-bezier(0.2, 0.7, 0.2, 1);
		will-change: transform;
		transition: bottom 120ms ease;
	}

	.sheet-panel.auto {
		max-height: 80dvh;
	}
	.sheet-panel.h-80 {
		height: 80dvh;
	}
	.sheet-panel.full {
		height: calc(100dvh - max(var(--ir-safe-top, 0px), env(safe-area-inset-top, 0px)) - 12px);
		max-height: none;
	}

	.sheet-grabber {
		flex: 0 0 auto;
		width: 36px;
		height: 4px;
		margin: 8px auto 2px;
		border-radius: 999px;
		background: var(--hr);
	}

	.sheet-header {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 4px 8px 4px 20px;
		flex: 0 0 auto;
	}

	.sheet-title {
		flex: 1;
		margin: 0;
		font-size: 16px;
		font-weight: 600;
		color: var(--heading);
	}

	.sheet-close {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 44px;
		height: 44px;
		border: 0;
		border-radius: 12px;
		background: transparent;
		color: var(--text-secondary);
	}

	.sheet-close:active {
		background: var(--bg-secondary);
	}

	.sheet-close svg {
		width: 20px;
		height: 20px;
	}

	.sheet-body {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		overscroll-behavior: contain;
		-webkit-overflow-scrolling: touch;
		padding: 4px 20px 12px;
	}

	.sheet-footer {
		flex: 0 0 auto;
		padding: 8px 20px;
		border-top: 1px solid var(--hr);
	}

	.sheet-safe {
		flex: 0 0 auto;
		height: calc(max(var(--ir-safe-bottom, 0px), env(safe-area-inset-bottom, 0px)) + 8px);
	}

	@keyframes sheetUp {
		from {
			transform: translateY(24px);
			opacity: 0.6;
		}
		to {
			transform: translateY(0);
			opacity: 1;
		}
	}

	@keyframes sheetFade {
		from {
			opacity: 0;
		}
		to {
			opacity: 1;
		}
	}

	@media (prefers-reduced-motion: reduce) {
		.sheet-panel,
		.sheet-backdrop {
			animation: none;
		}
	}
</style>
