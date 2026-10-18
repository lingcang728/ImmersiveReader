<!--
	Bottom sheet primitive for the mobile shell.

	Implements the shared surface the design doc specifies: dimmed backdrop,
	20px top radius, drag handle, safe-area bottom padding, 44px close target
	and a body that scrolls internally. Opening only animates
	transform/opacity; `prefers-reduced-motion` is honoured globally in
	`mobile.css`.
-->
<script lang="ts">
	import type { Snippet } from 'svelte';
	import Icon from './Icon.svelte';

	let {
		open,
		title = '',
		closeLabel = '关闭',
		testid = 'sheet',
		onClose,
		children,
		footer
	}: {
		open: boolean;
		title?: string;
		closeLabel?: string;
		testid?: string;
		onClose: () => void;
		children?: Snippet;
		footer?: Snippet;
	} = $props();

	function handleKeydown(event: KeyboardEvent) {
		if (!open || event.key !== 'Escape') return;
		event.preventDefault();
		onClose();
	}
</script>

<svelte:window onkeydown={handleKeydown} />

{#if open}
	<div class="sheet-root" data-testid={testid}>
		<button
			type="button"
			class="sheet-backdrop"
			aria-label={closeLabel}
			onclick={onClose}
		></button>
		<div
			class="sheet-panel"
			role="dialog"
			aria-modal="true"
			aria-label={title || '面板'}
		>
			<div class="sheet-grabber" aria-hidden="true"></div>
			<header class="sheet-header">
				<h2 class="sheet-title">{title}</h2>
				<button
					type="button"
					class="sheet-close"
					aria-label={closeLabel}
					onclick={onClose}
				>
					<Icon name="close" size={20} />
				</button>
			</header>
			<div class="sheet-body ir-scroll">
				{@render children?.()}
			</div>
			{#if footer}
				<footer class="sheet-footer">
					{@render footer()}
				</footer>
			{/if}
		</div>
	</div>
{/if}

<style>
	.sheet-root {
		position: fixed;
		inset: 0;
		z-index: 60;
		display: flex;
		flex-direction: column;
		justify-content: flex-end;
	}

	.sheet-backdrop {
		position: absolute;
		inset: 0;
		width: 100%;
		height: 100%;
		padding: 0;
		border: 0;
		border-radius: 0;
		background: rgb(0 0 0 / 0.34);
		cursor: pointer;
		animation: backdrop-in var(--ir-duration) var(--ir-ease);
	}

	.sheet-panel {
		position: relative;
		display: flex;
		flex-direction: column;
		width: 100%;
		max-height: min(88dvh, 760px);
		border-top: 1px solid var(--hr);
		border-radius: var(--ir-radius-xl) var(--ir-radius-xl) 0 0;
		background: var(--bg);
		color: var(--text);
		box-shadow: 0 -8px 32px rgb(0 0 0 / 0.18);
		padding-bottom: max(var(--ir-safe-bottom, 0px), env(safe-area-inset-bottom, 0px));
		animation: sheet-in var(--ir-duration) var(--ir-ease);
	}

	.sheet-grabber {
		flex: none;
		align-self: center;
		width: 36px;
		height: 4px;
		margin: 8px 0 0;
		border-radius: 999px;
		background: var(--hr);
	}

	.sheet-header {
		display: flex;
		flex: none;
		gap: 8px;
		align-items: center;
		min-height: 52px;
		padding: 4px 8px 4px 20px;
	}

	.sheet-title {
		flex: 1;
		min-width: 0;
		margin: 0;
		overflow: hidden;
		color: var(--heading);
		font-size: 17px;
		font-weight: 600;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.sheet-close {
		display: inline-flex;
		flex: none;
		align-items: center;
		justify-content: center;
		width: 44px;
		height: 44px;
		padding: 0;
		border: 0;
		border-radius: var(--ir-radius-md);
		background: transparent;
		color: var(--text-secondary);
	}

	.sheet-close:active {
		background: var(--bg-secondary);
	}

	.sheet-body {
		flex: 1;
		min-height: 0;
		padding: 4px 20px 16px;
	}

	.sheet-footer {
		flex: none;
		padding: 12px 20px 16px;
		border-top: 1px solid var(--hr);
	}

	@keyframes backdrop-in {
		from {
			opacity: 0;
		}

		to {
			opacity: 1;
		}
	}

	@keyframes sheet-in {
		from {
			opacity: 0.6;
			transform: translateY(24px);
		}

		to {
			opacity: 1;
			transform: translateY(0);
		}
	}
</style>
