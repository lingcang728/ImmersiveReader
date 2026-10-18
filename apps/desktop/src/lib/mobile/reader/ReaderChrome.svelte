<!--
	Reader top chrome — 52px + safe area. Hidden state only animates
	transform/opacity and is non-interactive, so taps pass through to the
	content beneath.
-->
<script lang="ts">
	interface Props {
		visible: boolean;
		title: string;
		chapterTitle: string;
		hasToc: boolean;
		hasBookmarks: boolean;
		onBack: () => void;
		onToc: () => void;
		onSearch: () => void;
		onBookmarks: () => void;
		onMore: () => void;
	}

	let {
		visible,
		title,
		chapterTitle,
		hasToc,
		hasBookmarks,
		onBack,
		onToc,
		onSearch,
		onBookmarks,
		onMore
	}: Props = $props();
</script>

<header
	class="reader-toolbar"
	class:hidden={!visible}
	data-testid="reader-toolbar"
	aria-hidden={!visible}
>
	<button
		type="button"
		class="toolbar-btn"
		aria-label="返回书架"
		data-testid="reader-back"
		tabindex={visible ? 0 : -1}
		onclick={onBack}
	>
		<svg viewBox="0 0 24 24" aria-hidden="true">
			<path d="M15 18l-6-6 6-6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
		</svg>
	</button>
	<div class="toolbar-title">
		<span class="reader-title">{title}</span>
		{#if chapterTitle}
			<span class="reader-chapter" data-testid="reader-chapter-title">{chapterTitle}</span>
		{/if}
	</div>
	<div class="toolbar-actions">
		<button
			type="button"
			class="toolbar-btn"
			aria-label="目录"
			data-testid="reader-toc"
			tabindex={visible ? 0 : -1}
			disabled={!hasToc}
			onclick={onToc}
		>
			<svg viewBox="0 0 24 24" aria-hidden="true">
				<path d="M8 6h12M8 12h12M8 18h12M4 6h.01M4 12h.01M4 18h.01" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
			</svg>
		</button>
		<button
			type="button"
			class="toolbar-btn"
			aria-label="搜索"
			data-testid="reader-search"
			tabindex={visible ? 0 : -1}
			onclick={onSearch}
		>
			<svg viewBox="0 0 24 24" aria-hidden="true">
				<circle cx="11" cy="11" r="7" fill="none" stroke="currentColor" stroke-width="2" />
				<path d="M16.5 16.5L21 21" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
			</svg>
		</button>
		{#if hasBookmarks}
			<button
				type="button"
				class="toolbar-btn"
				aria-label="书签"
				data-testid="reader-bookmarks"
				tabindex={visible ? 0 : -1}
				onclick={onBookmarks}
			>
				<svg viewBox="0 0 24 24" aria-hidden="true">
					<path d="M7 4h10v16l-5-4-5 4z" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round" />
				</svg>
			</button>
		{/if}
		<button
			type="button"
			class="toolbar-btn"
			aria-label="更多"
			data-testid="reader-settings"
			tabindex={visible ? 0 : -1}
			onclick={onMore}
		>
			<svg viewBox="0 0 24 24" aria-hidden="true">
				<circle cx="5" cy="12" r="1.7" fill="currentColor" />
				<circle cx="12" cy="12" r="1.7" fill="currentColor" />
				<circle cx="19" cy="12" r="1.7" fill="currentColor" />
			</svg>
		</button>
	</div>
</header>

<style>
	.reader-toolbar {
		position: fixed;
		top: 0;
		left: 0;
		right: 0;
		z-index: 50;
		display: flex;
		align-items: center;
		gap: 4px;
		height: calc(52px + max(var(--ir-safe-top, 0px), env(safe-area-inset-top, 0px)));
		padding: max(var(--ir-safe-top, 0px), env(safe-area-inset-top, 0px)) 8px 0;
		background: var(--bg);
		border-bottom: 1px solid var(--hr);
		transform: translateY(0);
		opacity: 1;
		transition: transform 180ms cubic-bezier(0.2, 0.7, 0.2, 1), opacity 160ms ease;
	}

	.reader-toolbar.hidden {
		transform: translateY(-8px);
		opacity: 0;
		pointer-events: none;
	}

	.toolbar-btn {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 44px;
		height: 44px;
		flex: 0 0 auto;
		padding: 0;
		border: 0;
		border-radius: 12px;
		background: transparent;
		color: var(--text);
		-webkit-tap-highlight-color: transparent;
	}

	.toolbar-btn:active {
		background: var(--bg-secondary);
	}

	.toolbar-btn:disabled {
		opacity: 0.35;
	}

	.toolbar-btn svg {
		width: 22px;
		height: 22px;
	}

	.toolbar-title {
		flex: 1;
		min-width: 0;
		display: flex;
		flex-direction: column;
		line-height: 1.25;
		padding: 0 4px;
	}

	.reader-title {
		overflow: hidden;
		font-size: 15px;
		font-weight: 600;
		color: var(--text);
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.reader-chapter {
		overflow: hidden;
		font-size: 12px;
		color: var(--text-secondary);
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.toolbar-actions {
		display: flex;
		align-items: center;
		gap: 2px;
		flex: 0 0 auto;
	}

	@media (prefers-reduced-motion: reduce) {
		.reader-toolbar {
			transition: none;
		}
	}
</style>
