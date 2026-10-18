<!--
	Transient toast stack for the mobile shell.

	The shell owns the queue (see `$lib/mobile/ui.ts`); this component only
	renders it above the safe-area bottom edge so it never collides with the
	Android gesture bar or the import FAB.
-->
<script lang="ts">
	import Icon from './Icon.svelte';
	import type { ToastItem } from '../ui';

	let { toasts }: { toasts: ToastItem[] } = $props();
</script>

<div class="toast-layer" role="status" aria-live="polite">
	{#each toasts as toast (toast.id)}
		<div class="toast" class:toast-error={toast.kind === 'error'} data-testid="toast-{toast.id}">
			<span class="toast-icon" aria-hidden="true">
				{#if toast.kind === 'success'}
					<Icon name="check" size={16} />
				{:else if toast.kind === 'error'}
					<Icon name="alert" size={16} />
				{:else}
					<Icon name="file" size={16} />
				{/if}
			</span>
			<span class="toast-text">{toast.text}</span>
		</div>
	{/each}
</div>

<style>
	.toast-layer {
		position: fixed;
		right: 16px;
		bottom: calc(88px + max(var(--ir-safe-bottom, 0px), env(safe-area-inset-bottom, 0px)));
		left: 16px;
		z-index: 200;
		display: flex;
		flex-direction: column;
		gap: 8px;
		align-items: center;
		pointer-events: none;
	}

	.toast {
		display: flex;
		gap: 8px;
		align-items: center;
		max-width: min(100%, 420px);
		padding: 10px 14px;
		border: 1px solid var(--hr);
		border-radius: var(--ir-radius-md);
		background: var(--bg-secondary);
		box-shadow: 0 8px 24px rgb(0 0 0 / 0.16);
		color: var(--text);
		font-size: 14px;
		line-height: 1.4;
		animation: toast-in var(--ir-duration-fast) var(--ir-ease);
		pointer-events: auto;
	}

	.toast-icon {
		display: inline-flex;
		flex: none;
		color: var(--text-secondary);
	}

	.toast-error {
		border-color: color-mix(in srgb, var(--danger) 36%, var(--hr));
	}

	.toast-error .toast-icon {
		color: var(--danger);
	}

	.toast-text {
		min-width: 0;
		overflow-wrap: anywhere;
	}

	@keyframes toast-in {
		from {
			opacity: 0;
			transform: translateY(8px);
		}

		to {
			opacity: 1;
			transform: translateY(0);
		}
	}
</style>
