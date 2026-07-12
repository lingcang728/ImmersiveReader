<script lang="ts">
	import type { BookSummary, LibraryIssue, TemporaryItem } from '$lib/library/books';
	import type { TaskSnapshot } from '$lib/tasks/sync';
	import './bookshelf.css';

	export let books: BookSummary[] = [];
	export let issues: LibraryIssue[] = [];
	export let temporaryItems: TemporaryItem[] = [];
	export let tasks: readonly TaskSnapshot[] = [];
	export let trashCount = 0;
	export let loading = false;
	export let writable = true;
	export let libraryRoot = '';
	export let onOpenBook: (bookId: string) => void;
	export let onFlowBook: (bookId: string) => void;
	export let onRefresh: () => void;
	export let onImport: () => void;
	export let onOpenFile: () => void;
	export let onOpenTemporary: (path: string) => void;
	export let onLaunchTool: (tool: 'zhihu' | 'podcast') => void;
	export let onStartTask: (taskId: string) => void;
	export let onRestartTask: (taskId: string) => void;
	export let onControlTask: (taskId: string, action: 'pause' | 'resume' | 'cancel' | 'cancel_and_discard', revision: number) => void;
	export let onChooseLibrary: () => void;
	export let onOpenTrash: () => void;
	export let onRemoveBook: (bookId: string, title: string, chapterCount: number) => void;
	export let onDeleteBook: (bookId: string, title: string, chapterCount: number) => void;

	let query = '';
	let acquireOpen = false;
	let openCardMenu: string | null = null;
	let recoverableBytes = 0;

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

	function taskKindLabel(kind: TaskSnapshot['kind']): string {
		return kind === 'podcast' ? '播客转写' : '知乎归档';
	}

	function taskStateLabel(task: TaskSnapshot): string {
		if (task.requiredAction === 'login') return '等待登录';
		if (task.requiredAction === 'captcha') return '等待验证码';
		if (task.requiredAction === 'configure_secret') return '需要配置密钥';
		if (task.requiredAction === 'free_disk_space') return '磁盘空间不足';
		if (task.requiredAction === 'approve_budget') return '等待预算确认';
		if (task.lifecycleState === 'terminal') {
			if (task.outcome === 'success') return '已完成';
			if (task.outcome === 'partial_success') return '部分完成';
			if (task.outcome === 'cancelled') return '已取消';
			if (task.outcome === 'interrupted') return '已中断';
			return '失败';
		}
		if (task.lifecycleState === 'paused') return '已暂停';
		if (task.lifecycleState === 'pausing') return '正在暂停';
		if (task.lifecycleState === 'queued') return '等待开始';
		return task.progress.label ?? '正在处理';
	}

	function taskProgress(task: TaskSnapshot): number | null {
		if (task.progress.mode !== 'determinate' || task.progress.percent === undefined) return null;
		return Math.max(0, Math.min(100, task.progress.percent));
	}

	function formatBytes(bytes: number): string {
		if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
		if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
		return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
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
							on:click={() => runAcquire(() => onLaunchTool('zhihu'))}
						>
							归档知乎
						</button>
						<button
							type="button"
							on:click={() => runAcquire(() => onLaunchTool('podcast'))}
						>
							转写播客
						</button>
						<button type="button" on:click={() => runAcquire(onImport)}>
							导入文件夹
						</button>
						<button type="button" on:click={() => runAcquire(onOpenFile)}>
							临时打开
						</button>
						<div class="menu-hint">生产工具在外部窗口运行；完成后回这里刷新即可看到新书。</div>
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
		{#if tasks.length > 0}
			<section class="task-rail" aria-label="统一任务队列" aria-live="polite">
				<header>
					<div>
						<strong>任务队列</strong>
						<span>{tasks.length} 项 · 可恢复材料 {formatBytes(recoverableBytes)}</span>
					</div>
					<span class="task-source">由沉浸阅读统一管理</span>
				</header>
				<div class="task-list">
					{#each tasks.slice(0, 4) as task (task.id)}
						{@const percent = taskProgress(task)}
						<div class="task-row">
							<span class:zhihu={task.kind === 'zhihu'} class="task-kind">
								{taskKindLabel(task.kind)}
							</span>
							<div class="task-copy">
								<strong>{taskStateLabel(task)}</strong>
								<small>{task.engineStage} · {task.engineStatus}</small>
							</div>
							{#if percent !== null}
								<span class="task-progress" aria-label={`进度 ${Math.round(percent)}%`}>
									<i style={`transform:scaleX(${percent / 100})`}></i>
								</span>
								<output>{Math.round(percent)}%</output>
							{:else}
								<span class="task-pulse" aria-hidden="true"></span>
							{/if}
							{#if task.kind === 'podcast' && task.lifecycleState === 'queued'}
								<button type="button" class="task-start" on:click={() => onStartTask(task.id)}>
									开始
								</button>
							{/if}
							{#if task.kind === 'podcast' && task.canPause}
								<button type="button" class="task-start" on:click={() => onControlTask(task.id, 'pause', task.revision)}>暂停</button>
							{/if}
							{#if task.kind === 'podcast' && task.canResume}
								<button type="button" class="task-start" on:click={() => onControlTask(task.id, 'resume', task.revision)}>恢复</button>
							{/if}
							{#if task.kind === 'podcast' && task.canCancel}
								<button type="button" class="task-start" on:click={() => onControlTask(task.id, 'cancel', task.revision)}>取消</button>
							{/if}
							{#if task.kind === 'podcast' && task.lifecycleState === 'terminal' && task.canRetry && task.requiredAction !== 'approve_budget'}
								<button type="button" class="task-start" on:click={() => onRestartTask(task.id)}>
									重新转写 revision
								</button>
							{/if}
						</div>
					{/each}
				</div>
			</section>
		{/if}

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
					<button class="quiet-action" on:click={() => onLaunchTool('zhihu')}>归档知乎</button>
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
	</div>
</section>
