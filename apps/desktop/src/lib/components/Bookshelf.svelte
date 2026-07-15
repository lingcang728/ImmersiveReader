<script lang="ts">
	import type { BookDetail, BookSummary, LibraryIssue, TemporaryItem } from '$lib/library/books';
	import type { TaskEvent, TaskSnapshot } from '$lib/tasks/sync';
	import TaskQueue from './TaskQueue.svelte';
	import './bookshelf.css';

	export let books: BookSummary[] = [];
	export let issues: LibraryIssue[] = [];
	export let temporaryItems: TemporaryItem[] = [];
	export let tasks: readonly TaskSnapshot[] = [];
	export let events: readonly TaskEvent[] = [];
	export let selectedBookDetail: BookDetail | null = null;
	export let trashCount = 0;
	export let loading = false;
	export let writable = true;
	export let libraryRoot = '';
	export let onOpenBook: (bookId: string) => void;
	export let onOpenDetails: (bookId: string) => void;
	export let onOpenSource: (source: string, sourceId?: string | null) => void;
	export let onCloseDetails: () => void;
	export let onFlowBook: (bookId: string) => void;
	export let onRefresh: () => void;
	export let onImport: () => void;
	export let onOpenFile: () => void;
	export let onOpenTemporary: (path: string) => void;
	export let onOpenZhihuWorkflow: () => void;
	export let onOpenPodcastWorkflow: () => void;
	export let onStartTask: (taskId: string) => void;
	export let onStartZhihuTask: (taskId: string, revision: number) => void;
	export let onOpenTaskResult: (taskId: string) => void;
	export let onRestartTask: (taskId: string) => void;
	export let onControlTask: (taskId: string, action: 'pause' | 'resume' | 'cancel' | 'cancel_and_discard', revision: number) => void;
	export let onControlZhihuTask: (taskId: string, action: 'pause' | 'resume' | 'cancel', revision: number) => void;
	export let onChooseLibrary: () => void;
	export let onOpenTrash: () => void;
	export let onRemoveBook: (bookId: string, title: string, chapterCount: number) => void;
	export let onDeleteBook: (bookId: string, title: string, chapterCount: number) => void;

	let query = '';
	let acquireOpen = false;
	let openCardMenu: string | null = null;
	let recoverableBytes = 0;
	/** Progressive chapter list in detail dialog: 40 at a time. */
	let detailChapterVisible = 40;
	let detailBookId: string | null = null;

	$: {
		const bookId = selectedBookDetail?.manifest.bookId ?? null;
		if (bookId !== detailBookId) {
			detailBookId = bookId;
			if (selectedBookDetail) {
				const chapters = selectedBookDetail.manifest.chapters;
				const current = selectedBookDetail.progress.current;
				const currentIndex = current
					? chapters.findIndex((chapter) => chapter.title === current)
					: -1;
				// Always include the current chapter in the first window.
				const ensureCurrent = currentIndex >= 0 ? currentIndex + 1 : 40;
				detailChapterVisible = Math.max(40, ensureCurrent);
			} else {
				detailChapterVisible = 40;
			}
		}
	}

	$: normalizedQuery = query.trim().toLocaleLowerCase();
	$: filteredBooks = normalizedQuery
		? books.filter((book) =>
				`${book.title} ${book.source} ${book.currentChapterTitle ?? ''}`
					.toLocaleLowerCase()
					.includes(normalizedQuery)
			)
		: books;
	$: resumeBook = books.find((book) => book.lastReadAt) ?? books[0];
	$: chapterTotal = books.reduce((sum, book) => sum + book.chapterCount, 0);

	function sourceLabel(source: string): string {
		return source === 'zhihu' ? '知乎' : source === 'podcast' ? '播客' : '手动';
	}

	function lastReadLabel(value?: string): string {
		if (!value) return '尚未开卷';
		const date = new Date(value);
		if (Number.isNaN(date.getTime())) return '有阅读记录';
		return new Intl.DateTimeFormat('zh-CN', {
			month: 'short',
			day: 'numeric',
			hour: '2-digit',
			minute: '2-digit'
		}).format(date);
	}

	$: recoverableBytes = tasks
		.filter((task) => task.recoverable)
		.reduce((sum, task) => sum + task.cacheLeaseBytes, 0);

	function closeMenus() {
		acquireOpen = false;
		openCardMenu = null;
	}

	function toggleAcquire() {
		openCardMenu = null;
		acquireOpen = !acquireOpen;
	}

	function toggleCardMenu(bookId: string) {
		acquireOpen = false;
		openCardMenu = openCardMenu === bookId ? null : bookId;
	}

	function runAcquire(action: () => void) {
		closeMenus();
		action();
	}
</script>

<svelte:window
	on:keydown={(e) => {
		if (e.key === 'Escape') closeMenus();
	}}
	on:click={closeMenus}
/>

<section class="bookshelf" aria-label="沉浸阅读书架">
	<header class="bs-header">
		<div class="brand">
			<span class="brand-mark" aria-hidden="true" title="沉浸阅读">
				<!-- Open book: blue accent via currentColor -->
				<svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
					<path
						d="M4 5.5c2.2-1.2 4.3-1.5 6.5-.4V18c-2.2-1.1-4.3-.9-6.5.3V5.5Z"
						stroke="currentColor"
						stroke-width="1.6"
						stroke-linejoin="round"
					/>
					<path
						d="M20 5.5c-2.2-1.2-4.3-1.5-6.5-.4V18c2.2-1.1 4.3-.9 6.5.3V5.5Z"
						stroke="currentColor"
						stroke-width="1.6"
						stroke-linejoin="round"
					/>
					<path
						d="M12 5.2v12.6"
						stroke="currentColor"
						stroke-width="1.4"
						stroke-linecap="round"
						opacity="0.9"
					/>
				</svg>
			</span>
			<span class="brand-name">沉浸阅读</span>
			<span class="brand-sub">书库</span>
		</div>
		<label class="search">
			<svg
				width="14"
				height="14"
				viewBox="0 0 24 24"
				fill="none"
				stroke="currentColor"
				stroke-width="1.75"
				aria-hidden="true"
				><circle cx="11" cy="11" r="7" /><path d="M20 20l-3.5-3.5" /></svg
			>
			<input
				bind:value={query}
				type="search"
				placeholder="搜索文集、篇目、来源…"
				aria-label="搜索书架"
				autocomplete="off"
				autocorrect="off"
				autocapitalize="off"
				spellcheck="false"
				name="bookshelf-search"
			/>
		</label>
		<div class="bs-header-actions">
			<button
				type="button"
				class="icon-action"
				on:click|stopPropagation={onRefresh}
				disabled={loading}
				title="刷新书库"
				aria-label="刷新书库"
			>
				<svg
					width="15"
					height="15"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="1.75"
					aria-hidden="true"
					><path d="M21 12a9 9 0 1 1-2.6-6.3" /><path d="M21 3v6h-6" /></svg
				>
			</button>
			<button type="button" class="menu-trigger" on:click={onOpenTrash}>
				回收站{trashCount > 0 ? ` ${trashCount}` : ''}
			</button>
			<div class="acquire-wrap">
				<button
					type="button"
					class="menu-trigger"
					aria-haspopup="menu"
					aria-expanded={acquireOpen}
					on:click|stopPropagation={toggleAcquire}
				>
					获取内容
					<span aria-hidden="true">▾</span>
				</button>
				{#if acquireOpen}
					<div class="acquire-menu" role="group" aria-label="获取内容">
						<button
							type="button"
							on:click={() => runAcquire(onOpenZhihuWorkflow)}
						>
							归档知乎
						</button>
						<button
							type="button"
							on:click={() => runAcquire(onOpenPodcastWorkflow)}
						>
							转写播客
						</button>
						<button type="button" on:click={() => runAcquire(onImport)}>
							导入文件夹
						</button>
						<button type="button" on:click={() => runAcquire(onOpenFile)}>
							临时打开
						</button>
						<div class="menu-hint">播客可在这里预检、确认预算并加入统一任务队列。</div>
					</div>
				{/if}
			</div>
		</div>
	</header>

	<div class="bs-body">
		{#if !writable}
			<div class="state-banner error" role="alert">
				<span>书库不可写。请选择可写目录：{libraryRoot}</span>
				<button class="recover-action" on:click={onChooseLibrary}>选择书库</button>
			</div>
		{/if}
		{#if issues.length > 0}
			<details class="state-banner warning">
				<summary>{issues.length} 个书目无法载入</summary>
				{#each issues as issue}
					<p><strong>{issue.path}</strong><br />{issue.message}</p>
				{/each}
			</details>
		{/if}
		<TaskQueue
			{tasks}
			{events}
			{recoverableBytes}
			{onStartTask}
			{onStartZhihuTask}
			{onOpenTaskResult}
			{onRestartTask}
			{onControlTask}
			{onControlZhihuTask}
		/>

		{#if loading}
			<div class="empty-state" aria-live="polite">
				<span class="loading-dot"></span>
				<p>正在扫描书库…</p>
			</div>
		{:else if resumeBook}
			<section class="resume" aria-label="继续阅读">
				<div class="resume-meta">
					<span class="resume-label">继续阅读</span>
					<h1>{resumeBook.title}</h1>
					<p>{resumeBook.currentChapterTitle ?? '从第一篇开始'}</p>
					<div class="progress-row">
						<span class="thin-bar"
							><i style={`width:${Math.round(resumeBook.progress * 100)}%`}></i></span
						>
						<span>{Math.round(resumeBook.progress * 100)}%</span>
					</div>
				</div>
				<button class="btn-resume" on:click={() => onOpenBook(resumeBook.bookId)}>接着读</button>
			</section>
		{:else}
			<div class="empty-state">
				<h1>书架还是空的</h1>
				<p>导入一个 Markdown 文件夹，或从知乎归档内容开始。</p>
				<div>
					<button class="btn-resume" on:click={onImport}>导入书库</button>
					<button class="quiet-action" on:click={onOpenZhihuWorkflow}>归档知乎</button>
					<button class="quiet-action" on:click={onOpenPodcastWorkflow}>转写播客</button>
				</div>
			</div>
		{/if}

		{#if books.length > 0}
			<div class="section-head">
				<h2>文集</h2>
				<span>{filteredBooks.length} 部 · 共 {chapterTotal} 篇</span>
			</div>
			<div class="collection-grid">
				{#each filteredBooks as book (book.bookId)}
					<article class="book-card">
						<div class="book-card-top">
							<span class:zhihu={book.source === 'zhihu'} class="badge"
								>{sourceLabel(book.source)}</span
							>
							<div class="card-menu-wrap">
								<button
									type="button"
									class="card-menu-btn"
									aria-label={`文集操作：${book.title}`}
									aria-haspopup="menu"
									aria-expanded={openCardMenu === book.bookId}
									on:click|stopPropagation={() => toggleCardMenu(book.bookId)}
								>
									⋯
								</button>
								{#if openCardMenu === book.bookId}
									<div class="card-menu" role="group" aria-label="文集操作">
										<button
											type="button"
											on:click={() => {
												closeMenus();
												onRemoveBook(book.bookId, book.title, book.chapterCount);
											}}
										>
											移出书架
										</button>
										<button
											type="button"
											class="danger"
											on:click={() => {
												closeMenus();
												onDeleteBook(book.bookId, book.title, book.chapterCount);
											}}
										>
											删除本地文件…
										</button>
									</div>
								{/if}
							</div>
						</div>
						<h3>{book.title}</h3>
						<div class="book-stats">
							<span>{book.chapterCount} 篇 · 已读 {book.readCount}</span>
							<span>{lastReadLabel(book.lastReadAt)}</span>
						</div>
						<div class="progress-row">
							<span class="thin-bar"
								><i style={`width:${Math.round(book.progress * 100)}%`}></i></span
							>
							<span>{Math.round(book.progress * 100)}%</span>
						</div>
						<div class="book-actions">
							<span>{book.currentChapterTitle ?? '尚未开卷'}</span>
							<div>
								<button class="act-secondary" on:click={() => onOpenDetails(book.bookId)}>详情</button>
								<button class="act-primary" on:click={() => onOpenBook(book.bookId)}>精读</button>
								<button class="act-secondary" on:click={() => onFlowBook(book.bookId)}
									>连读 ↗</button
								>
							</div>
						</div>
					</article>
				{:else}
					<p class="no-result">没有匹配的文集。</p>
				{/each}
			</div>
		{/if}

		{#if temporaryItems.length > 0}
			<section class="temp-section">
				<div class="section-head">
					<h2>临时内容</h2>
					<span>播客转录 · 不自动归档</span>
				</div>
				{#each temporaryItems as item (item.path)}
					<div class="temp-item">
						<span class="temp-badge">播客</span>
						<span>{item.title}</span>
						<small>{lastReadLabel(item.modifiedAt)}</small>
						<button on:click={() => onOpenTemporary(item.path)}>打开</button>
					</div>
				{/each}
			</section>
		{/if}

		{#if selectedBookDetail}
			{@const chapters = selectedBookDetail.manifest.chapters}
			{@const currentTitle = selectedBookDetail.progress.current || ''}
			{@const currentIdx = Math.max(
				0,
				chapters.findIndex((c) => c.title === currentTitle)
			)}
			<div class="book-detail-backdrop" role="presentation">
				<dialog open class="book-detail-dialog" aria-labelledby="book-detail-title">
					<header class="book-detail-header">
						<div class="book-detail-heading">
							<h2 id="book-detail-title">{selectedBookDetail.manifest.title}</h2>
							<p class="book-detail-summary">
								{chapters.length} 章 · 进度 {Math.round(selectedBookDetail.progress.position * 100)}% ·
								{lastReadLabel(selectedBookDetail.manifest.updatedAt)}
							</p>
						</div>
						<button type="button" class="close-detail" aria-label="关闭详情" on:click={onCloseDetails}>×</button>
					</header>
					<div class="book-detail-body">
						<div class="book-detail-actions">
							<button
								type="button"
								class="detail-primary"
								on:click={() => onOpenBook(selectedBookDetail.manifest.bookId)}
							>
								继续阅读
							</button>
							{#if selectedBookDetail.manifest.source === 'zhihu' && selectedBookDetail.manifest.sourceId}
								<button
									type="button"
									class="source-link"
									on:click={() =>
										onOpenSource(
											selectedBookDetail.manifest.source,
											selectedBookDetail.manifest.sourceId
										)}>打开知乎主页</button
								>
							{/if}
						</div>

						<section class="chapter-section" aria-label="目录">
							<header class="chapter-section-header">
								<h3>目录</h3>
								<span
									>{Math.min(detailChapterVisible, chapters.length)} / {chapters.length}</span
								>
							</header>
							<ol class="chapter-list">
								{#each chapters.slice(0, detailChapterVisible) as chapter, index}
									<li class:current={index === currentIdx || chapter.title === currentTitle}>
										<span>{chapter.title}</span>
										<small>{chapter.date ?? ''}</small>
									</li>
								{/each}
							</ol>
							{#if detailChapterVisible < chapters.length}
								<button
									type="button"
									class="load-more-chapters"
									on:click={() => (detailChapterVisible += 40)}
								>
									加载更多（+40）
								</button>
							{/if}
						</section>

						<details class="tech-details">
							<summary>技术信息</summary>
							<dl class="book-detail-meta">
								<div><dt>来源</dt><dd>{sourceLabel(selectedBookDetail.manifest.source)}</dd></div>
								<div><dt>书目 ID</dt><dd>{selectedBookDetail.manifest.bookId}</dd></div>
								<div><dt>生成时间</dt><dd>{selectedBookDetail.manifest.generatedAt}</dd></div>
								<div><dt>更新时间</dt><dd>{selectedBookDetail.manifest.updatedAt}</dd></div>
								<div><dt>当前章节</dt><dd>{currentTitle || '未开始'}</dd></div>
							</dl>
							{#if selectedBookDetail.provenance}
								<div class="provenance-grid">
									<div>
										<span>版本</span>
										<strong>{selectedBookDetail.provenance.revision ?? '—'}</strong>
									</div>
									<div>
										<span>引擎</span>
										<strong>{selectedBookDetail.provenance.engineVersion ?? '—'}</strong>
									</div>
								</div>
							{/if}
							{#if selectedBookDetail.taskRecords.length > 0}
								<p class="book-detail-note">
									关联任务 {selectedBookDetail.taskRecords.length} 条（默认折叠）
								</p>
							{/if}
						</details>
					</div>
				</dialog>
			</div>
		{/if}
	</div>
</section>
