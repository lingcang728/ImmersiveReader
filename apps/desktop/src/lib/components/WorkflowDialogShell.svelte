<script lang="ts">
	import { onDestroy, onMount, tick } from 'svelte';

	export let titleId: string;
	export let descriptionId: string;
	export let eyebrow = '';
	export let title: string;
	export let description = '';
	export let maxWidth = '720px';
	export let onClose: () => void;

	let dialogEl: HTMLDialogElement;
	let previousFocus: HTMLElement | null = null;

	function handleCancel(event: Event) {
		event.preventDefault();
		onClose();
	}

	function handleBackdropClick(event: MouseEvent) {
		if (event.target === dialogEl) {
			onClose();
		}
	}

	onMount(async () => {
		previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
		await tick();
		if (dialogEl && typeof dialogEl.showModal === 'function' && !dialogEl.open) {
			dialogEl.showModal();
		}
		// Focus first focusable control inside the dialog body/footer.
		const focusTarget =
			dialogEl?.querySelector<HTMLElement>(
				'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])'
			) ?? dialogEl;
		focusTarget?.focus();
	});

	onDestroy(() => {
		if (dialogEl?.open) {
			try {
				dialogEl.close();
			} catch {
				/* already closed */
			}
		}
		if (previousFocus && typeof previousFocus.focus === 'function') {
			previousFocus.focus();
		}
	});
</script>

<dialog
	bind:this={dialogEl}
	class="workflow-dialog"
	style="--workflow-max-width: {maxWidth}"
	aria-modal="true"
	aria-labelledby={titleId}
	aria-describedby={description ? descriptionId : undefined}
	on:cancel={handleCancel}
	on:click={handleBackdropClick}
>
	<div class="workflow-panel" role="document">
		<header class="workflow-header">
			<div class="workflow-heading">
				{#if eyebrow}
					<span class="workflow-eyebrow">{eyebrow}</span>
				{/if}
				<h1 id={titleId} class="workflow-title">{title}</h1>
				{#if description}
					<p id={descriptionId} class="workflow-description">{description}</p>
				{/if}
			</div>
			<button type="button" class="workflow-close" aria-label="关闭" on:click={onClose}>
				<svg width="14" height="14" viewBox="0 0 14 14" aria-hidden="true">
					<path
						d="M3 3l8 8M11 3L3 11"
						stroke="currentColor"
						stroke-width="1.4"
						stroke-linecap="round"
					/>
				</svg>
			</button>
		</header>

		<div class="workflow-body">
			<slot />
		</div>

		{#if $$slots.footer}
			<footer class="workflow-footer">
				<slot name="footer" />
			</footer>
		{/if}
	</div>
</dialog>

<style>
	/* Workflow tokens — all derived from the active theme's variables, so
	   dialogs follow MMbook monochrome + link blue instead of a hardcoded
	   navy palette. */
	.workflow-dialog {
		--wf-panel: var(--bg-secondary);
		--wf-panel-raised: var(--bg);
		--wf-title: var(--heading, var(--text));
		--wf-body: var(--text);
		--wf-muted: var(--text-secondary);
		--wf-label: color-mix(in srgb, var(--text-secondary) 80%, var(--text));
		--wf-border: color-mix(in srgb, var(--link) 40%, var(--hr));
		--wf-accent: var(--link);
		--wf-accent-hover: var(--link-hover);
		--wf-input-bg: var(--bg);
		--wf-focus: var(--link);
		--wf-shadow: 0 28px 90px rgba(0, 0, 0, 0.45);
		--wf-scrim: rgba(0, 0, 0, 0.55);

		width: min(var(--workflow-max-width, 720px), calc(100vw - 32px));
		max-height: min(760px, 92vh);
		margin: auto;
		padding: 0;
		border: 0;
		background: transparent;
		color: var(--wf-body);
	}

	/* Light themes only soften the scrim/shadow; every color token comes
	   from the theme variables above. */
	:global(.app.is-light-theme) .workflow-dialog {
		--wf-shadow: 0 24px 70px rgba(0, 0, 0, 0.16);
		--wf-scrim: rgba(0, 0, 0, 0.28);
	}

	.workflow-dialog::backdrop {
		background: var(--wf-scrim);
		backdrop-filter: blur(8px);
	}

	.workflow-panel {
		display: flex;
		flex-direction: column;
		max-height: min(760px, 92vh);
		overflow: auto;
		border: 1px solid var(--wf-border);
		border-radius: 18px;
		background: var(--wf-panel);
		box-shadow: var(--wf-shadow);
	}

	.workflow-header {
		display: flex;
		justify-content: space-between;
		align-items: flex-start;
		gap: 20px;
		padding: 26px 28px 18px;
		border-bottom: 1px solid color-mix(in srgb, var(--wf-border) 70%, transparent);
		flex: none;
	}

	.workflow-heading {
		min-width: 0;
	}

	.workflow-eyebrow {
		display: block;
		color: var(--wf-accent);
		font-size: 10px;
		font-weight: 600;
		/* CJK 字形：只需轻微拉开字距，uppercase 对中文无效。 */
		letter-spacing: 0.08em;
	}

	.workflow-title {
		margin: 8px 0 6px;
		font-size: 24px;
		font-weight: 600;
		line-height: 1.2;
		color: var(--wf-title);
		letter-spacing: 0;
	}

	.workflow-description {
		margin: 0;
		color: var(--wf-muted);
		font-size: 13px;
		line-height: 1.55;
		max-width: 52ch;
		/* Action-confirm messages carry \n\n paragraphs. */
		white-space: pre-line;
	}

	.workflow-close {
		flex: none;
		display: grid;
		place-items: center;
		width: 34px;
		height: 34px;
		border: 1px solid var(--wf-border);
		border-radius: 50%;
		background: transparent;
		color: var(--wf-muted);
		cursor: pointer;
		transition:
			background 140ms ease,
			color 140ms ease,
			border-color 140ms ease;
	}

	.workflow-close:hover {
		border-color: var(--wf-accent);
		color: var(--wf-title);
		background: color-mix(in srgb, var(--wf-accent) 12%, transparent);
	}

	.workflow-close:focus-visible {
		outline: 2px solid var(--wf-focus);
		outline-offset: 2px;
	}

	.workflow-body {
		display: grid;
		gap: 16px;
		padding: 22px 28px;
	}

	.workflow-footer {
		display: flex;
		justify-content: flex-end;
		gap: 12px;
		padding: 16px 28px 22px;
		border-top: 1px solid color-mix(in srgb, var(--wf-border) 70%, transparent);
		flex: none;
	}

	/* Shared form/control tokens for children via inheritance */
	.workflow-panel :global(.wf-label) {
		display: block;
		color: var(--wf-label);
		font-size: 11px;
		font-weight: 500;
		letter-spacing: 0.02em;
	}

	.workflow-panel :global(.wf-field) {
		display: grid;
		gap: 6px;
		color: var(--wf-body);
		font-size: 13px;
	}

	.workflow-panel :global(.wf-field input),
	.workflow-panel :global(.wf-field select),
	.workflow-panel :global(input[type='text']),
	.workflow-panel :global(input[type='number']),
	.workflow-panel :global(select) {
		min-width: 0;
		border: 1px solid var(--wf-border);
		border-radius: 8px;
		padding: 9px 10px;
		background: var(--wf-input-bg);
		color: var(--wf-title);
		font: inherit;
	}

	.workflow-panel :global(input:focus-visible),
	.workflow-panel :global(select:focus-visible),
	.workflow-panel :global(textarea:focus-visible),
	.workflow-panel :global(button:focus-visible) {
		outline: 2px solid var(--wf-focus);
		outline-offset: 2px;
	}

	.workflow-panel :global(.wf-card) {
		border: 1px solid var(--wf-border);
		border-radius: 12px;
		padding: 13px 14px;
		background: var(--wf-panel-raised);
		color: var(--wf-body);
	}

	.workflow-panel :global(.wf-quiet),
	.workflow-panel :global(.wf-secondary),
	.workflow-panel :global(.wf-primary) {
		border: 1px solid var(--wf-border);
		border-radius: 9px;
		padding: 9px 13px;
		cursor: pointer;
		font: inherit;
		font-size: 12px;
		transition:
			background 140ms ease,
			border-color 140ms ease,
			color 140ms ease,
			transform 100ms ease;
	}

	.workflow-panel :global(.wf-quiet) {
		background: transparent;
		color: var(--wf-muted);
	}

	.workflow-panel :global(.wf-secondary) {
		background: var(--wf-input-bg);
		color: var(--wf-title);
	}

	.workflow-panel :global(.wf-primary) {
		border-color: var(--wf-accent);
		background: var(--wf-accent);
		color: var(--on-accent);
	}

	.workflow-panel :global(.wf-quiet:hover),
	.workflow-panel :global(.wf-secondary:hover) {
		border-color: var(--wf-accent);
		color: var(--wf-title);
		transform: translateY(-1px);
	}

	.workflow-panel :global(.wf-primary:hover) {
		background: var(--wf-accent-hover);
		border-color: var(--wf-accent-hover);
		transform: translateY(-1px);
	}

	.workflow-panel :global(.wf-primary:active),
	.workflow-panel :global(.wf-secondary:active),
	.workflow-panel :global(.wf-quiet:active) {
		transform: translateY(0);
	}

	.workflow-panel :global(.wf-primary:disabled),
	.workflow-panel :global(.wf-secondary:disabled) {
		cursor: not-allowed;
		opacity: 0.45;
		transform: none;
	}

	.workflow-panel :global(.wf-msg-error) {
		margin: 0;
		color: var(--danger);
		font-size: 12px;
		line-height: 1.5;
	}

	.workflow-panel :global(.wf-msg-success) {
		margin: 0;
		color: #3fad78;
		font-size: 12px;
		line-height: 1.5;
	}

	.workflow-panel :global(.wf-status-warn) {
		/* P3-12: was hardcoded gold #c9922e — palette rule: warnings reuse --link. */
		color: var(--link);
	}

	.workflow-panel :global(.wf-status-ok) {
		color: #2f9e68;
	}

	@media (max-width: 620px) {
		.workflow-header,
		.workflow-body,
		.workflow-footer {
			padding-left: 16px;
			padding-right: 16px;
		}

		.workflow-footer {
			flex-direction: column;
			align-items: stretch;
		}
	}

	@media (prefers-reduced-motion: reduce) {
		.workflow-panel :global(.wf-quiet),
		.workflow-panel :global(.wf-secondary),
		.workflow-panel :global(.wf-primary),
		.workflow-close {
			transition: none;
		}
	}
</style>
