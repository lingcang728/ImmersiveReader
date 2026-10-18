<!--
	Mobile bookshelf.

	First screen of the Android app: brand header, optional search field,
	"continue reading" hero, batched book list, empty/error states and the
	import FAB. The shell owns scan/refresh/import IPC — this component only
	renders the snapshot it is given and emits intents.

	Performance notes:
	- the list renders in `SHELF_BATCH_SIZE` slices and grows on a
	  requestAnimationFrame-throttled scroll listener;
	- the hero is excluded from the list so a long library does not paint it
	  twice;
	- all animation is transform/opacity only and disabled by
	  `prefers-reduced-motion` (see mobile.css).
-->
<script lang="ts">
	import { onDestroy } from 'svelte';
	import Icon from './components/Icon.svelte';
	import type { BookSummary, LibraryIssue } from './contracts';
	import {
		SHELF_BATCH_SIZE,
		bookInitial,
		filterBooksByTitle,
		formatBookMeta,
		formatProgress,
		progressPercent,
		selectContinueBook,
		sortShelfBooks,
		nextBatchCount
	} from './shelf';

	let {
		books = [],
		issues = [],
		loading = false,
		writable = true,
		bootError = null,
		searchOpen = false,
		onOpenBook,
		onOpenSettings,
		onImport,
		onRequestDelete,
		onRetryScan,
		onToggleSearch,
		onCloseSearch,
		onWarmReader = () => {}
	}: {
		books?: BookSummary[];
		issues?: LibraryIssue[];
		loading?: boolean;
		writable?: boolean;
		bootError?: string | null;
		searchOpen?: boolean;
		onOpenBook: (book: BookSummary) => void;
		onOpenSettings: () => void;
		onImport: () => void;
		onRequestDelete: (book: BookSummary) => void;
		onRetryScan: () => void;
		onToggleSearch: () => void;
		onCloseSearch: () => void;
		/** Warm the lazily imported reader chunk as soon as a book is pressed. */
		onWarmReader?: () => void;
	} = $props();

	// ----- data ---------------------------------------------------------
	let query = $state('');

	const sortedAll = $derived(sortShelfBooks(books));
	const hero = $derived(selectContinueBook(sortedAll));
	const trimmedQuery = $derived(query.trim());
	const searchActive = $derived(trimmedQuery.length > 0);
	const showHero = $derived(hero !== null && !searchActive);
	const listBooks = $derived(
		filterBooksByTitle(
			searchActive || hero === null
				? sortedAll
				: sortedAll.filter((book) => book.bookId !== hero?.bookId),
			trimmedQuery
		)
	);

	let visibleCount = $state(SHELF_BATCH_SIZE);
	const visibleBooks = $derived(listBooks.slice(0, Math.max(visibleCount, SHELF_BATCH_SIZE)));
	const hasMore = $derived(visibleCount < listBooks.length);
	const isEmpty = $derived(!loading && books.length === 0 && bootError === null);
	const noResults = $derived(!loading && searchActive && listBooks.length === 0);

	$effect(() => {
		// A new search always starts from the first batch.
		void trimmedQuery;
		visibleCount = SHELF_BATCH_SIZE;
	});

	$effect(() => {
		if (searchOpen) return;
		// The shell can close search from the Android back gesture; the input
		// state must not keep filtering the visible list invisibly.
		if (query !== '') query = '';
	});

	// ----- search -------------------------------------------------------
	let searchInputEl = $state<HTMLInputElement | null>(null);
	let issuesOpen = $state(false);

	$effect(() => {
		if (!searchOpen) return;
		const frame = requestAnimationFrame(() => searchInputEl?.focus());
		return () => cancelAnimationFrame(frame);
	});

	function handleSearchInput(event: Event) {
		query = (event.currentTarget as HTMLInputElement).value;
	}

	function clearSearch() {
		query = '';
		searchInputEl?.focus();
	}

	// ----- incremental list --------------------------------------------
	let listEl = $state<HTMLElement | null>(null);
	let scrollFrame: number | null = null;

	function handleListScroll() {
		if (scrollFrame !== null) return;
		scrollFrame = requestAnimationFrame(() => {
			scrollFrame = null;
			const el = listEl;
			if (!el) return;
			const remaining = el.scrollHeight - el.scrollTop - el.clientHeight;
			if (remaining > 320) return;
			if (visibleCount >= listBooks.length) return;
			visibleCount = nextBatchCount(visibleCount, listBooks.length);
		});
	}

	onDestroy(() => {
		if (scrollFrame !== null) cancelAnimationFrame(scrollFrame);
	});

	// ----- long press → trash confirm ----------------------------------
	const LONG_PRESS_MS = 520;
	const PRESS_MOVE_TOLERANCE_PX = 12;
	const CLICK_SUPPRESS_MS = 700;

	let pressTimer: ReturnType<typeof setTimeout> | null = null;
	let pressedBookId: string | null = null;
	let pressStartX = 0;
	let pressStartY = 0;
	let suppressClickUntil = 0;

	function clearPressTimer() {
		if (pressTimer === null) return;
		clearTimeout(pressTimer);
		pressTimer = null;
	}

	function handlePointerDown(event: PointerEvent, book: BookSummary) {
		if (event.pointerType === 'mouse' && event.button !== 0) return;
		// The user is committing to open a book: warm the reader chunk now.
		onWarmReader();
		if (!writable) return;
		clearPressTimer();
		pressedBookId = book.bookId;
		pressStartX = event.clientX;
		pressStartY = event.clientY;
		pressTimer = setTimeout(() => {
			pressTimer = null;
			if (pressedBookId !== book.bookId) return;
			suppressClickUntil = performance.now() + CLICK_SUPPRESS_MS;
			onRequestDelete(book);
		}, LONG_PRESS_MS);
	}

	function handlePointerMove(event: PointerEvent) {
		if (pressTimer === null) return;
		const dx = event.clientX - pressStartX;
		const dy = event.clientY - pressStartY;
		if (Math.hypot(dx, dy) > PRESS_MOVE_TOLERANCE_PX) clearPressTimer();
	}

	function handlePointerEnd() {
		clearPressTimer();
		pressedBookId = null;
	}

	function handleBookClick(book: BookSummary) {
		if (performance.now() < suppressClickUntil) return;
		if (searchActive) query = '';
		onOpenBook(book);
	}
</script>

<div class="shelf" data-testid="shelf">
	<header class="shelf-header">
		<div class="brand">
			<h1 class="brand-title">沉浸阅读</h1>
			<p class="brand-subtitle">本地 EPUB · Markdown</p>
		</div>
		<div class="header-actions">
			<button
				type="button"
				class="icon-btn"
				aria-label="搜索书名"
				aria-expanded={searchOpen}
				data-testid="shelf-search"
				onclick={onToggleSearch}
			>
				<Icon name="search" />
			</button>
			<button
				type="button"
				class="icon-btn"
				aria-label="设置"
				data-testid="shelf-settings"
				onclick={onOpenSettings}
			>
				<Icon name="settings" />
			</button>
		</div>
	</header>

	{#if searchOpen}
		<div class="search-row">
			<Icon name="search" size={18} />
			<input
				bind:this={searchInputEl}
				class="search-input"
				type="search"
				inputmode="search"
				placeholder="搜索书名"
				aria-label="搜索书名"
				value={query}
				oninput={handleSearchInput}
			/>
			{#if query}
				<button type="button" class="search-clear" aria-label="清除搜索" onclick={clearSearch}>
					<Icon name="close" size={16} />
				</button>
			{/if}
			<button type="button" class="search-cancel" onclick={onCloseSearch}>取消</button>
		</div>
	{/if}

	<div class="shelf-scroll ir-scroll" bind:this={listEl} onscroll={handleListScroll}>
		{#if bootError}
			<div class="banner banner-error" role="alert">
				<Icon name="alert" size={18} />
				<span class="banner-text">书库读取失败 · {bootError}</span>
				<button type="button" class="banner-action" onclick={onRetryScan}>重试</button>
			</div>
		{/if}

		{#if !writable && !bootError}
			<div class="banner">
				<Icon name="alert" size={18} />
				<span class="banner-text">书库当前不可写，导入与删除暂不可用。</span>
			</div>
		{/if}

		{#if issues.length > 0}
			<div class="banner">
				<Icon name="alert" size={18} />
				<span class="banner-text">{issues.length} 个项目无法读取</span>
				<button
					type="button"
					class="banner-action"
					aria-expanded={issuesOpen}
					onclick={() => (issuesOpen = !issuesOpen)}
				>
					{issuesOpen ? '收起' : '查看'}
				</button>
			</div>
			{#if issuesOpen}
				<ul class="issue-list">
					{#each issues.slice(0, 20) as issue (issue.path)}
						<li class="issue-item">
							<span class="issue-path">{issue.path}</span>
							<span class="issue-message">{issue.message}</span>
						</li>
					{/each}
				</ul>
			{/if}
		{/if}

		{#if loading && books.length === 0}
			<div class="skeleton-list" aria-hidden="true">
				{#each Array.from({ length: 6 }, (_, index) => index) as index (index)}
					<div class="skeleton-row" style={`animation-delay: ${index * 60}ms`}>
						<span class="skeleton-cover"></span>
						<span class="skeleton-info">
							<span class="skeleton-line skeleton-line-title"></span>
							<span class="skeleton-line skeleton-line-meta"></span>
						</span>
					</div>
				{/each}
			</div>
		{:else if isEmpty}
			<div class="shelf-empty" data-testid="shelf-empty">
				<span class="empty-icon" aria-hidden="true"><Icon name="book" size={40} /></span>
				<h2 class="empty-title">书库还是空的</h2>
				<p class="empty-copy">导入 EPUB 或 Markdown，开始本地阅读。</p>
				<button type="button" class="primary-btn" onclick={onImport}>
					<Icon name="plus" size={20} />
					<span>导入 EPUB / Markdown</span>
				</button>
			</div>
		{:else}
			{#if showHero && hero}
				<button
					type="button"
					class="continue-card ir-pressable"
					data-testid="shelf-continue"
					onclick={() => handleBookClick(hero)}
					onpointerdown={(event) => handlePointerDown(event, hero)}
					onpointermove={handlePointerMove}
					onpointerup={handlePointerEnd}
					onpointercancel={handlePointerEnd}
					onpointerleave={handlePointerEnd}
					oncontextmenu={(event) => event.preventDefault()}
				>
					<span class="cover cover-lg" aria-hidden="true">{bookInitial(hero.title)}</span>
					<span class="continue-main">
						<span class="continue-label">继续阅读</span>
						<span class="continue-title">{hero.title}</span>
						<span class="continue-chapter">
							{hero.currentChapterTitle || '尚未开始阅读'}
						</span>
						<span class="progress-track" aria-hidden="true">
							<span
								class="progress-fill"
								style={`width: ${progressPercent(hero.progress)}%`}
							></span>
						</span>
					</span>
					<span class="continue-side">
						<span class="continue-percent">{formatProgress(hero.progress)}</span>
						<span class="continue-cta">继续</span>
					</span>
				</button>
			{/if}

			{#if visibleBooks.length > 0}
				<ul class="book-list">
					{#each visibleBooks as book (book.bookId)}
						<li class="book-item">
							<button
								type="button"
								class="book-row"
								data-testid={`shelf-book-${book.bookId}`}
								onclick={() => handleBookClick(book)}
								onpointerdown={(event) => handlePointerDown(event, book)}
								onpointermove={handlePointerMove}
								onpointerup={handlePointerEnd}
								onpointercancel={handlePointerEnd}
								onpointerleave={handlePointerEnd}
								oncontextmenu={(event) => event.preventDefault()}
							>
								<span class="cover cover-sm" aria-hidden="true">
									{bookInitial(book.title)}
								</span>
								<span class="book-main">
									<span class="book-title">{book.title}</span>
									<span class="book-meta">{formatBookMeta(book)}</span>
								</span>
								<span class="book-progress">
									<span class="book-percent">{formatProgress(book.progress)}</span>
								</span>
							</button>
						</li>
					{/each}
				</ul>
			{/if}

			{#if hasMore}
				<div class="list-more" aria-hidden="true">正在加载更多…</div>
			{/if}

			{#if noResults}
				<div class="search-empty">
					<p class="search-empty-title">没有找到匹配的书</p>
					<p class="search-empty-copy">换个书名试试。</p>
				</div>
			{/if}
		{/if}
	</div>

	<button
		type="button"
		class="import-fab ir-pressable"
		aria-label="导入书籍"
		data-testid="shelf-import"
		onclick={onImport}
	>
		<Icon name="plus" size={26} />
	</button>
</div>

<style>
	.shelf {
		position: absolute;
		inset: 0;
		display: flex;
		flex-direction: column;
		overflow: hidden;
		background: var(--bg);
	}

	.shelf-header {
		display: flex;
		flex: none;
		gap: 8px;
		align-items: flex-end;
		padding: calc(12px + max(var(--ir-safe-top, 0px), env(safe-area-inset-top, 0px)))
			max(8px, var(--ir-safe-right, 0px)) 12px
			calc(20px + max(var(--ir-safe-left, 0px), env(safe-area-inset-left, 0px)));
	}

	.brand {
		flex: 1;
		min-width: 0;
	}

	.brand-title {
		margin: 0;
		color: var(--heading);
		font-size: 24px;
		font-weight: 600;
		letter-spacing: 0.01em;
		line-height: 1.2;
	}

	.brand-subtitle {
		margin: 4px 0 0;
		color: var(--text-secondary);
		font-size: 12px;
	}

	.header-actions {
		display: flex;
		flex: none;
		gap: 4px;
		align-items: center;
	}

	.icon-btn {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 44px;
		height: 44px;
		padding: 0;
		border: 0;
		border-radius: var(--ir-radius-md);
		background: transparent;
		color: var(--text);
	}

	.icon-btn:active {
		background: var(--bg-secondary);
	}

	.search-row {
		display: flex;
		flex: none;
		gap: 8px;
		align-items: center;
		height: 48px;
		margin: 0 8px 8px;
		padding: 0 4px 0 14px;
		border-radius: var(--ir-radius-md);
		background: var(--bg-secondary);
		color: var(--text-secondary);
	}

	.search-input {
		flex: 1;
		min-width: 0;
		height: 44px;
		padding: 0 4px;
		border: 0;
		outline: none;
		background: transparent;
		color: var(--text);
		font-size: 15px;
		appearance: none;
	}

	.search-input::placeholder {
		color: var(--text-faded);
	}

	.search-input::-webkit-search-cancel-button {
		appearance: none;
	}

	.search-clear {
		display: inline-flex;
		flex: none;
		align-items: center;
		justify-content: center;
		width: 32px;
		height: 32px;
		padding: 0;
		border: 0;
		border-radius: 999px;
		background: transparent;
		color: var(--text-secondary);
	}

	.search-cancel {
		flex: none;
		min-width: 44px;
		height: 44px;
		padding: 0 8px;
		border: 0;
		background: transparent;
		color: var(--link);
		font-size: 14px;
	}

	.shelf-scroll {
		flex: 1;
		min-height: 0;
		padding: 0
			max(20px, var(--ir-safe-right, 0px)) calc(104px + max(var(--ir-safe-bottom, 0px), env(safe-area-inset-bottom, 0px)))
			calc(20px + max(var(--ir-safe-left, 0px), env(safe-area-inset-left, 0px)));
	}

	.banner {
		display: flex;
		gap: 8px;
		align-items: center;
		margin: 4px 0 12px;
		padding: 10px 12px;
		border: 1px solid var(--hr);
		border-radius: var(--ir-radius-md);
		background: var(--bg-secondary);
		color: var(--text-secondary);
		font-size: 13px;
		flex-wrap: wrap;
	}

	.banner-error {
		border-color: color-mix(in srgb, var(--danger) 36%, var(--hr));
	}

	.banner-text {
		flex: 1;
		min-width: 0;
		overflow-wrap: anywhere;
	}

	.banner-action {
		flex: none;
		min-width: 44px;
		min-height: 32px;
		padding: 0 10px;
		border: 0;
		border-radius: var(--ir-radius-sm);
		background: transparent;
		color: var(--link);
		font-size: 13px;
	}

	.issue-list {
		margin: 0 0 12px;
		padding: 0 0 0 4px;
		list-style: none;
	}

	.issue-item {
		padding: 6px 0;
		border-bottom: 1px solid var(--hr);
		font-size: 12px;
		color: var(--text-faded);
	}

	.issue-path {
		display: block;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.issue-message {
		display: block;
		margin-top: 2px;
		color: var(--text-secondary);
	}

	/* ----- skeleton ----- */
	.skeleton-list {
		display: flex;
		flex-direction: column;
	}

	.skeleton-row {
		display: flex;
		gap: 12px;
		align-items: center;
		min-height: 72px;
		padding: 8px 0;
		border-bottom: 1px solid var(--hr);
		animation: skeleton-pulse 1.4s ease-in-out infinite;
	}

	.skeleton-cover {
		flex: none;
		width: 44px;
		height: 60px;
		border-radius: 6px;
		background: var(--bg-secondary);
	}

	.skeleton-info {
		display: flex;
		flex: 1;
		flex-direction: column;
		gap: 8px;
	}

	.skeleton-line {
		height: 12px;
		border-radius: 6px;
		background: var(--bg-secondary);
	}

	.skeleton-line-title {
		width: 70%;
	}

	.skeleton-line-meta {
		width: 46%;
	}

	@keyframes skeleton-pulse {
		0%,
		100% {
			opacity: 0.55;
		}

		50% {
			opacity: 0.9;
		}
	}

	/* ----- empty ----- */
	.shelf-empty {
		display: flex;
		flex-direction: column;
		align-items: center;
		padding: 56px 12px 24px;
		text-align: center;
	}

	.empty-icon {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 64px;
		height: 64px;
		margin-bottom: 16px;
		border-radius: 50%;
		background: var(--bg-secondary);
		color: var(--text-secondary);
	}

	.empty-title {
		margin: 0;
		color: var(--heading);
		font-size: 18px;
		font-weight: 600;
	}

	.empty-copy {
		margin: 8px 0 24px;
		color: var(--text-secondary);
		font-size: 14px;
		line-height: 1.6;
	}

	.primary-btn {
		display: inline-flex;
		gap: 8px;
		align-items: center;
		justify-content: center;
		min-height: 48px;
		padding: 0 22px;
		border: 0;
		border-radius: var(--ir-radius-md);
		background: var(--link);
		color: var(--on-accent);
		font-size: 15px;
		font-weight: 500;
	}

	.primary-btn:active {
		opacity: 0.88;
	}

	/* ----- continue reading hero ----- */
	.continue-card {
		display: flex;
		gap: 14px;
		align-items: center;
		width: 100%;
		margin: 4px 0 20px;
		padding: 16px;
		border: 0;
		border-radius: var(--ir-radius-lg);
		background: var(--bg-secondary);
		color: var(--text);
		text-align: left;
	}

	.cover {
		display: inline-flex;
		flex: none;
		align-items: center;
		justify-content: center;
		border: 1px solid var(--hr);
		border-radius: 6px;
		background: var(--bg);
		color: var(--text-secondary);
		font-weight: 600;
	}

	.cover-lg {
		width: 56px;
		height: 76px;
		font-size: 24px;
	}

	.cover-sm {
		width: 44px;
		height: 60px;
		font-size: 19px;
	}

	.continue-main {
		display: flex;
		flex: 1;
		flex-direction: column;
		gap: 4px;
		min-width: 0;
	}

	.continue-label {
		color: var(--link);
		font-size: 12px;
		font-weight: 500;
		letter-spacing: 0.04em;
	}

	.continue-title {
		display: -webkit-box;
		overflow: hidden;
		color: var(--heading);
		font-size: 17px;
		font-weight: 600;
		line-height: 1.35;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		-webkit-line-clamp: 2;
	}

	.continue-chapter {
		overflow: hidden;
		color: var(--text-secondary);
		font-size: 13px;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.progress-track {
		display: block;
		width: 100%;
		height: 4px;
		margin-top: 6px;
		overflow: hidden;
		border-radius: 999px;
		background: color-mix(in srgb, var(--hr) 70%, transparent);
	}

	.progress-fill {
		display: block;
		height: 100%;
		border-radius: 999px;
		background: var(--link);
		transition: width var(--ir-duration) var(--ir-ease);
	}

	.continue-side {
		display: flex;
		flex: none;
		flex-direction: column;
		gap: 10px;
		align-items: flex-end;
	}

	.continue-percent {
		color: var(--text-secondary);
		font-size: 13px;
		font-variant-numeric: tabular-nums;
	}

	.continue-cta {
		padding: 6px 14px;
		border: 1px solid var(--link);
		border-radius: 999px;
		color: var(--link);
		font-size: 13px;
	}

	/* ----- book list ----- */
	.book-list {
		margin: 0;
		padding: 0;
		list-style: none;
	}

	.book-item {
		margin: 0;
	}

	.book-row {
		display: flex;
		gap: 12px;
		align-items: center;
		width: 100%;
		min-height: 72px;
		padding: 8px 0;
		border: 0;
		border-bottom: 1px solid var(--hr);
		border-radius: 0;
		background: transparent;
		color: var(--text);
		text-align: left;
	}

	.book-row:active {
		background: var(--bg-secondary);
	}

	.book-main {
		display: flex;
		flex: 1;
		flex-direction: column;
		gap: 4px;
		min-width: 0;
	}

	.book-title {
		overflow: hidden;
		color: var(--text);
		font-size: 16px;
		font-weight: 500;
		line-height: 1.35;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.book-meta {
		overflow: hidden;
		color: var(--text-faded);
		font-size: 13px;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.book-progress {
		display: flex;
		flex: none;
		flex-direction: column;
		gap: 2px;
		align-items: flex-end;
	}

	.book-percent {
		color: var(--text-secondary);
		font-size: 13px;
		font-variant-numeric: tabular-nums;
	}

	.list-more {
		padding: 16px 0;
		color: var(--text-faded);
		font-size: 12px;
		text-align: center;
	}

	.search-empty {
		padding: 48px 12px;
		text-align: center;
	}

	.search-empty-title {
		margin: 0;
		color: var(--text);
		font-size: 15px;
	}

	.search-empty-copy {
		margin: 6px 0 0;
		color: var(--text-secondary);
		font-size: 13px;
	}

	/* ----- import fab ----- */
	.import-fab {
		position: absolute;
		right: max(20px, var(--ir-safe-right, 0px));
		bottom: calc(20px + max(var(--ir-safe-bottom, 0px), env(safe-area-inset-bottom, 0px)));
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 56px;
		height: 56px;
		padding: 0;
		border: 0;
		border-radius: 50%;
		background: var(--link);
		color: var(--on-accent);
		box-shadow: 0 6px 18px rgb(0 0 0 / 0.22);
		z-index: 5;
	}

	.import-fab:active {
		opacity: 0.9;
	}
</style>
