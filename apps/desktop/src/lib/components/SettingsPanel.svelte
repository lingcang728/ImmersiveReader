<script lang="ts">
	import { onMount } from "svelte";
	import { invokeCommand as invoke } from "$lib/ipc";
	import { reportError } from "$lib/errors";
	import {
		settingsOpen,
		currentTheme,
		fontScale,
		clampFontScale,
		FONT_SCALE_STEP,
		readingLineHeight,
		readingWidth,
		readingFontFamily,
		autoFocusMode,
		READING_LINE_HEIGHTS,
		READING_WIDTHS,
	} from "$lib/stores/app";
	import { getThemePairs } from "$lib/theme/themes";
	import { checkForDesktopUpdate, downloadAndInstallDesktopUpdate, updateState } from "$lib/update/service";
	import WorkflowDialogShell from "./WorkflowDialogShell.svelte";

	const themePairs = getThemePairs();

	type StorageLocations = {
		channel: string;
		settingsPath: string;
		dataRoot: string;
		cacheRoot: string;
		logsRoot: string;
		runtimeStateRoot: string;
		backupsRoot: string;
		libraryRoot: string;
		runtimeRoot: string;
	};
	type StorageUsage = {
		libraryBytes: number;
		dataBytes: number;
		cacheBytes: number;
		logsBytes: number;
		backupsBytes: number;
		runtimeStateBytes: number;
	};
	type SecretStatus = {
		configured: boolean;
		maskedHint?: string | null;
		lastVerifiedAt?: string | null;
	};
	type MigrationPreview = {
		items: Array<{ kind: string; exists: boolean; bytes: number; sensitive: boolean; conflict: boolean }>;
		totalBytes: number;
		conflictCount: number;
		sensitiveItemCount: number;
	};
	type CacheClearResult = { deletedItems: number; releasedBytes: number; skipped: Array<{ reason: string }> };
	type PublishTransaction = { transactionId: string; phase: string; bookId: string };
	type StateBackupResult = { backupPath: string; included: string[]; skipped: string[] };
	type StateBackupInfo = {
		path: string;
		createdAt?: string | null;
		appVersion?: string | null;
		channel?: string | null;
		included: string[];
	};
	type StateRestoreResult = { restored: string[]; preRestorePath: string };
	type MigrationRun = { migrationId: string; previewId: string; scope: string; status: string; receiptPath?: string | null };

	let locations: StorageLocations | null = null;
	let usage: StorageUsage | null = null;
	let secretStatus: SecretStatus | null = null;
	let migrationPreview: MigrationPreview | null = null;
	let publishRecovery: PublishTransaction[] = [];
	let migrationRuns: MigrationRun[] = [];
	let backupResult: StateBackupResult | null = null;
	let stateBackups: StateBackupInfo[] = [];
	let panelLoading = false;
	let advancedLoading = false;
	let advancedOpen = false;
	let advancedLoaded = false;
	let actionBusy = false;
	let panelNotice = "";
	let apiKey = "";
	// P3-12: themed confirm dialog shared by the destructive actions below —
	// replaces unthemed window.confirm().
	let confirmRequest: { message: string; proceed: () => void } | null = null;
	let updateInstallArmed = false;
	let stopSubscription: (() => void) | undefined;
	let storageRows: Array<[string, string, string, number]> = [];
	$: storageRows = locations
		? [
				["library", "书库", locations.libraryRoot, usage?.libraryBytes ?? 0],
				["data", "应用数据", locations.dataRoot, usage?.dataBytes ?? 0],
				["cache", "缓存", locations.cacheRoot, usage?.cacheBytes ?? 0],
				["logs", "日志", locations.logsRoot, usage?.logsBytes ?? 0],
				["backups", "备份", locations.backupsRoot, usage?.backupsBytes ?? 0],
				["runtime_state", "运行时状态", locations.runtimeStateRoot, usage?.runtimeStateBytes ?? 0]
			]
		: [];
	$: updateBusy = ["checking", "downloading", "installing"].includes($updateState.status);
	$: updateProgress = $updateState.totalBytes
		? Math.min(100, Math.round($updateState.downloadedBytes / $updateState.totalBytes * 100))
		: null;
	$: updateStatusLabel = ({
		idle: "尚未检查",
		checking: "正在检查 GitHub Release",
		available: `发现新版本 ${$updateState.version}`,
		downloading: updateProgress === null ? "正在下载更新" : `正在下载 ${updateProgress}%`,
		installing: "正在安装，完成后会自动重启",
		failed: "更新失败",
		upToDate: "当前已是最新版本",
	} as const)[$updateState.status];

	onMount(() => {
		stopSubscription = settingsOpen.subscribe((open) => {
			if (open) {
				advancedOpen = false;
				advancedLoaded = false;
				void loadPanel();
			}
		});
		return () => stopSubscription?.();
	});

	async function loadPanel() {
		if (panelLoading) return;
		panelLoading = true;
		panelNotice = "";
		try {
			// Core panel: locations + secret only. Disk usage / recovery load on advanced expand.
			[locations, secretStatus] = await Promise.all([
				invoke<StorageLocations>("get_storage_locations"),
				invoke<SecretStatus>("get_secret_status")
			]);
		} catch (error) {
			panelNotice = `设置状态读取失败：${reportError("设置状态读取", error)}`;
		} finally {
			panelLoading = false;
		}
	}

	async function ensureAdvancedLoaded() {
		if (advancedLoaded || advancedLoading) return;
		advancedLoading = true;
		try {
			[usage, publishRecovery, migrationRuns, stateBackups] = await Promise.all([
				invoke<StorageUsage>("get_storage_usage"),
				invoke<PublishTransaction[]>("get_publish_recovery_status"),
				invoke<MigrationRun[]>("get_migration_runs"),
				invoke<StateBackupInfo[]>("list_state_backups")
			]);
			advancedLoaded = true;
		} catch (error) {
			panelNotice = `高级状态读取失败：${reportError("高级状态读取", error)}`;
		} finally {
			advancedLoading = false;
		}
	}

	async function toggleAdvanced() {
		advancedOpen = !advancedOpen;
		if (advancedOpen) await ensureAdvancedLoaded();
	}

	function formatBytes(bytes: number) {
		if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
		const units = ["B", "KB", "MB", "GB", "TB"];
		const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
		return `${(bytes / 1024 ** index).toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
	}

	async function copyPath(path: string) {
		try {
			await navigator.clipboard.writeText(path);
			panelNotice = "路径已复制";
		} catch (error) {
			panelNotice = `复制失败：${reportError("复制路径", error)}`;
		}
	}

	async function revealDirectory(kind: string) {
		try {
			await invoke("reveal_storage_directory", { kind });
		} catch (error) {
			panelNotice = `无法打开目录：${reportError("打开目录", error)}`;
		}
	}

	async function clearCache(confirmed = false) {
		if (actionBusy) return;
		if (!confirmed) {
			confirmRequest = {
				message: "仅清理不受保护的可重建缓存，不会删除 Library、Data 或 Backups。继续？",
				proceed: () => void clearCache(true)
			};
			return;
		}
		actionBusy = true;
		try {
			const result = await invoke<CacheClearResult>("clear_safe_cache", {
				categories: ["podcast_completed", "zhihu_browser_cache", "general_temporary"],
				taskIds: []
			});
			panelNotice = `已清理 ${result.deletedItems} 项，释放 ${formatBytes(result.releasedBytes)}${result.skipped.length ? `，跳过 ${result.skipped.length} 项受保护任务` : ""}`;
		} catch (error) {
			panelNotice = `缓存清理失败：${reportError("缓存清理", error)}`;
		} finally {
			actionBusy = false;
		}
	}

	async function createStateBackup(confirmed = false) {
		if (actionBusy) return;
		if (!confirmed) {
			confirmRequest = {
				message: "创建当前渠道的状态备份？Library、Cache、Logs、凭据和浏览器 Profile 会被排除。",
				proceed: () => void createStateBackup(true)
			};
			return;
		}
		actionBusy = true;
		try {
			backupResult = await invoke<StateBackupResult>("create_state_backup");
			stateBackups = await invoke<StateBackupInfo[]>("list_state_backups");
			panelNotice = "状态备份已创建";
		} catch (error) {
			panelNotice = `状态备份失败：${reportError("状态备份", error)}`;
		} finally {
			actionBusy = false;
		}
	}

	function formatBackupTime(createdAt?: string | null): string {
		if (!createdAt) return "未知时间";
		const date = new Date(createdAt);
		return Number.isNaN(date.getTime()) ? createdAt : date.toLocaleString();
	}

	async function restoreStateBackup(backup: StateBackupInfo, confirmed = false) {
		if (actionBusy) return;
		if (!confirmed) {
			confirmRequest = {
				message: `恢复到 ${formatBackupTime(backup.createdAt)} 的状态备份？当前设置与任务数据库会先自动快照到 pre-restore 目录再被替换，完成后建议重启应用使恢复生效。`,
				proceed: () => void restoreStateBackup(backup, true)
			};
			return;
		}
		actionBusy = true;
		try {
			const result = await invoke<StateRestoreResult>("restore_state_backup", { backupPath: backup.path });
			panelNotice = result.restored.length
				? `已恢复 ${result.restored.join("、")}；建议重启应用使任务历史与设置完全生效`
				: "备份中没有可恢复的内容";
		} catch (error) {
			panelNotice = `恢复失败：${reportError("状态恢复", error)}`;
		} finally {
			actionBusy = false;
		}
	}

	async function saveApiKey() {
		if (actionBusy || !apiKey.trim()) {
			panelNotice = "请输入 API Key";
			return;
		}
		actionBusy = true;
		try {
			secretStatus = await invoke<SecretStatus>("set_deepseek_api_key", { apiKey });
			apiKey = "";
			panelNotice = "Key 已写入 Credential Manager；界面不会显示它";
		} catch (error) {
			panelNotice = `Key 保存失败：${reportError("保存 API Key", error)}`;
		} finally {
			actionBusy = false;
		}
	}

	async function deleteApiKey(confirmed = false) {
		if (actionBusy) return;
		if (!confirmed) {
			confirmRequest = {
				message: "删除当前渠道的 DeepSeek Key？",
				proceed: () => void deleteApiKey(true)
			};
			return;
		}
		actionBusy = true;
		try {
			secretStatus = await invoke<SecretStatus>("delete_deepseek_api_key");
			panelNotice = "DeepSeek Key 已删除";
		} catch (error) {
			panelNotice = `Key 删除失败：${reportError("删除 API Key", error)}`;
		} finally {
			actionBusy = false;
		}
	}

	async function previewMigration() {
		if (actionBusy) return;
		actionBusy = true;
		try {
			migrationPreview = await invoke<MigrationPreview>("preview_legacy_migration", { scope: "all" });
			panelNotice = "迁移预览已刷新；未写入任何数据";
		} catch (error) {
			panelNotice = `迁移预览失败：${reportError("迁移预览", error)}`;
		} finally {
			actionBusy = false;
		}
	}

	async function recoverPublish(confirmed = false) {
		if (actionBusy || !publishRecovery.length) return;
		if (!confirmed) {
			confirmRequest = {
				message: "恢复所有未完成的发布事务？",
				proceed: () => void recoverPublish(true)
			};
			return;
		}
		actionBusy = true;
		try {
			publishRecovery = await invoke<PublishTransaction[]>("recover_publish_transactions", { transactionIds: null });
			panelNotice = "发布恢复检查已完成";
		} catch (error) {
			panelNotice = `发布恢复失败：${reportError("发布恢复", error)}`;
		} finally {
			actionBusy = false;
		}
	}

	const widthLabels: Record<number, string> = { 680: "窄", 760: "标准", 840: "宽" };

	function adjustFontScale(direction: number) {
		$fontScale = clampFontScale($fontScale + direction * FONT_SCALE_STEP);
	}

	function runConfirmed() {
		const request = confirmRequest;
		confirmRequest = null;
		request?.proceed();
	}

	function closePanel() {
		confirmRequest = null;
		settingsOpen.set(false);
	}

	async function installUpdate() {
		updateInstallArmed = false;
		await downloadAndInstallDesktopUpdate();
	}

	// P2-32: native <dialog> gives Esc (cancel), a top-layer surface and a
	// focus trap; the action handles autofocus + focus restore like
	// WorkflowDialogShell. keydown stays stop-propagated so global
	// shortcuts (Ctrl+F/O, reading keys) cannot fire while the modal is up;
	// Esc itself still reaches us through the cancel event.
	function settingsDialog(node: HTMLDialogElement) {
		const previousFocus =
			document.activeElement instanceof HTMLElement ? document.activeElement : null;
		if (typeof node.showModal === "function" && !node.open) {
			try {
				node.showModal();
			} catch {
				/* already open or unsupported: degrade to inline panel */
			}
		}
		const target =
			node.querySelector<HTMLElement>(
				'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])'
			) ?? node;
		target.focus();
		return {
			destroy() {
				if (node.open) {
					try {
						node.close();
					} catch {
						/* already closed */
					}
				}
				previousFocus?.focus();
			}
		};
	}

	function handleDialogCancel(event: Event) {
		event.preventDefault();
		closePanel();
	}

	function handleDialogClick(event: MouseEvent) {
		// Backdrop clicks land on the <dialog> element itself.
		if (event.target === event.currentTarget) closePanel();
	}

	// radiogroup 键盘约定：方向键在选项间移动并选中。
	function radioGroupKeydown(event: KeyboardEvent) {
		if (
			event.key !== "ArrowLeft" &&
			event.key !== "ArrowRight" &&
			event.key !== "ArrowUp" &&
			event.key !== "ArrowDown"
		) {
			return;
		}
		const group = event.currentTarget as HTMLElement;
		const radios = Array.from(group.querySelectorAll<HTMLElement>('[role="radio"]'));
		const index = radios.indexOf(document.activeElement as HTMLElement);
		if (index < 0 || radios.length < 2) return;
		event.preventDefault();
		const step = event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1 : 1;
		const next = radios[(index + step + radios.length) % radios.length];
		next.focus();
		next.click();
	}
</script>

{#if $settingsOpen}
	<dialog
		id="settings-panel"
		class="settings-dialog"
		use:settingsDialog
		aria-modal="true"
		aria-labelledby="settings-panel-title"
		on:cancel={handleDialogCancel}
		on:click={handleDialogClick}
		on:keydown|stopPropagation
	>
		<div class="settings-panel" role="document">
			<div class="settings-header">
				<div>
					<div class="settings-title" id="settings-panel-title">设置</div>
					<div class="settings-subtitle">外观 · 阅读 · 服务</div>
				</div>
				<button class="close-btn" type="button" on:click={closePanel} aria-label="关闭设置">×</button>
			</div>
			{#if panelLoading}
				<div class="status-line">正在读取本地状态…</div>
			{/if}
			{#if panelNotice}
				<div class="notice" role="status">{panelNotice}</div>
			{/if}

			<div class="settings-title section-title">外观</div>
			<div class="settings-title nested-title" id="settings-theme-label">主题</div>
			<div
				class="theme-grid"
				role="radiogroup"
				tabindex="-1"
				aria-labelledby="settings-theme-label"
				on:keydown={radioGroupKeydown}
			>
				{#each themePairs as pair}
					<button
						class="theme-option"
						class:active={$currentTheme.name === pair.light.name}
						role="radio"
						aria-checked={$currentTheme.name === pair.light.name}
						on:click={() => ($currentTheme = pair.light)}
					>
						<div
							class="theme-preview"
							style="background: {pair.light.vars['--bg']}; color: {pair.light.vars['--text']}"
						>
							Aa
						</div>
						<span>{pair.label} 浅色</span>
					</button>
					<button
						class="theme-option"
						class:active={$currentTheme.name === pair.dark.name}
						role="radio"
						aria-checked={$currentTheme.name === pair.dark.name}
						on:click={() => ($currentTheme = pair.dark)}
					>
						<div
							class="theme-preview"
							style="background: {pair.dark.vars['--bg']}; color: {pair.dark.vars['--text']}"
						>
							Aa
						</div>
						<span>{pair.label} 深色</span>
					</button>
				{/each}
			</div>

			<div class="settings-title section-title">阅读排版</div>
			<div class="typo-rows">
				<div class="typo-row">
					<span class="typo-label">字号</span>
					<div class="typo-options">
						<button class="typo-btn" on:click={() => adjustFontScale(-1)} title="缩小 (Ctrl+-)" aria-label="缩小字号">−</button>
						<span class="typo-value" aria-live="polite">{Math.round($fontScale * 100)}%</span>
						<button class="typo-btn" on:click={() => adjustFontScale(1)} title="放大 (Ctrl+=)" aria-label="放大字号">+</button>
					</div>
				</div>
				<div class="typo-row">
					<span class="typo-label">行距</span>
					<div
						class="typo-options"
						role="radiogroup"
						tabindex="-1"
						aria-label="行距"
						on:keydown={radioGroupKeydown}
					>
						{#each READING_LINE_HEIGHTS as lh}
							<button
								class="typo-btn"
								class:active={$readingLineHeight === lh}
								role="radio"
								aria-checked={$readingLineHeight === lh}
								on:click={() => ($readingLineHeight = lh)}
							>
								{lh.toFixed(1)}
							</button>
						{/each}
					</div>
				</div>
				<div class="typo-row">
					<span class="typo-label">栏宽</span>
					<div
						class="typo-options"
						role="radiogroup"
						tabindex="-1"
						aria-label="栏宽"
						on:keydown={radioGroupKeydown}
					>
						{#each READING_WIDTHS as w}
							<button
								class="typo-btn"
								class:active={$readingWidth === w}
								role="radio"
								aria-checked={$readingWidth === w}
								on:click={() => ($readingWidth = w)}
							>
								{widthLabels[w]}
							</button>
						{/each}
					</div>
				</div>
				<div class="typo-row">
					<span class="typo-label">字体</span>
					<div
						class="typo-options"
						role="radiogroup"
						tabindex="-1"
						aria-label="字体"
						on:keydown={radioGroupKeydown}
					>
						<button
							class="typo-btn"
							class:active={$readingFontFamily === "sans"}
							role="radio"
							aria-checked={$readingFontFamily === "sans"}
							on:click={() => ($readingFontFamily = "sans")}
						>
							黑体
						</button>
						<button
							class="typo-btn typo-serif"
							class:active={$readingFontFamily === "serif"}
							role="radio"
							aria-checked={$readingFontFamily === "serif"}
							on:click={() => ($readingFontFamily = "serif")}
						>
							宋体
						</button>
					</div>
				</div>
				<div class="typo-row">
					<span class="typo-label">自动沉浸</span>
					<label class="toggle-switch" title="打开文章后自动进入沉浸模式">
						<input
							type="checkbox"
							aria-label="打开文章后自动进入沉浸模式"
							checked={$autoFocusMode}
							on:change={() => ($autoFocusMode = !$autoFocusMode)}
						/>
						<span class="toggle-slider" aria-hidden="true"></span>
					</label>
				</div>
			</div>

			<div class="settings-title section-title">软件更新</div>
			<div class="update-card">
				<div class="update-head">
					<div>
						<strong>{updateStatusLabel}</strong>
						{#if $updateState.status === "failed"}
							<span>{$updateState.error}</span>
						{:else if $updateState.status === "available"}
							<span>当前 {$updateState.currentVersion}{#if $updateState.sizeBytes} · {formatBytes($updateState.sizeBytes)}{/if}</span>
						{:else}
							<span>版本 {$updateState.currentVersion || "读取中"} · 每天最多静默检查一次</span>
						{/if}
					</div>
					<button type="button" class="action-btn" disabled={updateBusy} on:click={() => void checkForDesktopUpdate(true)}>
						{$updateState.status === "checking" ? "检查中…" : "检查更新"}
					</button>
				</div>
				{#if $updateState.status === "downloading" && updateProgress !== null}
					<progress value={updateProgress} max="100">{updateProgress}%</progress>
				{/if}
				{#if $updateState.status === "available"}
					<div class="update-release">
						<div><strong>沉浸阅读 {$updateState.version}</strong><span>{$updateState.notes || "本次 Release 未填写更新说明。"}</span></div>
						<button type="button" class="action-btn update-primary" on:click={() => (updateInstallArmed = true)}>下载安装</button>
					</div>
				{/if}
				{#if updateInstallArmed}
					<div class="update-confirm" role="alert">
						<div><strong>安装沉浸阅读 {$updateState.version}？</strong><span>应用会自动重启，书库、阅读进度与服务配置不会被删除。</span></div>
						<button type="button" class="action-btn" disabled={updateBusy} on:click={() => (updateInstallArmed = false)}>取消</button>
						<button type="button" class="action-btn update-primary" disabled={updateBusy} on:click={() => void installUpdate()}>确认安装</button>
					</div>
				{/if}
			</div>

			<div class="settings-title section-title">AI 服务与书库</div>
			<div class="credential-row">
				<span>
					{#if secretStatus?.configured}
						DeepSeek 已配置{#if secretStatus.maskedHint}（{secretStatus.maskedHint}）{/if}
					{:else}
						未配置 DeepSeek Key
					{/if}
				</span>
				{#if secretStatus?.configured}<button type="button" class="mini-btn danger" disabled={actionBusy} on:click={() => void deleteApiKey()}>删除</button>{/if}
			</div>
			<div class="credential-form">
				<input type="password" bind:value={apiKey} autocomplete="new-password" placeholder="输入 Key（不会显示或写入磁盘）" aria-label="DeepSeek API Key" />
				<button type="button" class="mini-btn" disabled={actionBusy || !apiKey.trim()} on:click={() => void saveApiKey()}>保存</button>
			</div>
			{#if locations}
				<div class="status-card library-card">
					<strong>当前书库</strong>
					<span title={locations.libraryRoot}>{locations.libraryRoot}</span>
					<button type="button" class="mini-btn" on:click={() => revealDirectory("library")}>打开</button>
				</div>
			{/if}

			<button type="button" class="advanced-toggle" aria-expanded={advancedOpen} on:click={() => void toggleAdvanced()}>
				{advancedOpen ? "收起高级" : "高级：路径、缓存、备份与恢复"}
			</button>
			{#if advancedOpen}
				<div class="advanced-block">
					{#if advancedLoading}
						<div class="status-line">正在读取存储与恢复状态…</div>
					{/if}
					<div class="settings-title nested-title">存储路径</div>
					{#if locations}
						<div class="path-list">
							{#each storageRows as row}
								<div class="path-row">
									<span>{row[1]}</span>
									<div class="path-value">
										<code title={row[2]}>{row[2]}</code>
										<small>{formatBytes(row[3])}</small>
									</div>
									<div class="path-actions">
										<button type="button" class="mini-btn" on:click={() => copyPath(row[2])}>复制</button>
										<button type="button" class="mini-btn" on:click={() => revealDirectory(row[0])}>打开</button>
									</div>
								</div>
							{/each}
						</div>
					{:else}
						<div class="status-line">路径状态不可用</div>
					{/if}

					<div class="settings-title nested-title">维护与恢复</div>
					<div class="action-grid">
						<button type="button" class="action-btn" disabled={actionBusy} on:click={() => void clearCache()}>安全清理缓存</button>
						<button type="button" class="action-btn" disabled={actionBusy} on:click={() => void previewMigration()}>刷新迁移预览</button>
						<button type="button" class="action-btn" disabled={actionBusy} on:click={() => void createStateBackup()}>创建状态备份</button>
					</div>
					{#if migrationPreview}
						<div class="status-card">
							<strong>迁移预览（只读）</strong>
							<span>{migrationPreview.items.length} 项 · {formatBytes(migrationPreview.totalBytes)} · 冲突 {migrationPreview.conflictCount} · 敏感 {migrationPreview.sensitiveItemCount}</span>
						</div>
					{/if}
					{#if migrationRuns.length}
						<div class="status-card">
							<strong>迁移记录 {migrationRuns.length}</strong>
							<span>{migrationRuns.slice(0, 4).map((run) => `${run.scope} · ${run.status}`).join("；")}</span>
						</div>
					{:else if advancedLoaded}
						<div class="status-card"><strong>迁移恢复</strong><span>没有已记录的迁移；真实迁移仍需独立授权。</span></div>
					{/if}
					{#if backupResult}
						<div class="status-card">
							<strong>状态备份已创建</strong>
							<span>{backupResult.backupPath}</span>
						</div>
					{/if}
					{#if stateBackups.length}
						<div class="status-card">
							<strong>状态备份 {stateBackups.length} 份</strong>
							{#each stateBackups.slice(0, 5) as backup}
								<div class="backup-row">
									<span>{formatBackupTime(backup.createdAt)}{backup.appVersion ? ` · v${backup.appVersion}` : ""}</span>
									<button type="button" class="mini-btn" disabled={actionBusy} on:click={() => void restoreStateBackup(backup)}>恢复</button>
								</div>
							{/each}
						</div>
					{/if}
					{#if publishRecovery.length}
						<div class="status-card recovery-card">
							<strong>待恢复发布 {publishRecovery.length}</strong>
							<span>{publishRecovery.map((item) => item.bookId).join("；")}</span>
							<button type="button" class="mini-btn" disabled={actionBusy} on:click={() => void recoverPublish()}>执行恢复检查</button>
						</div>
					{:else if advancedLoaded}
						<div class="status-card"><strong>发布恢复</strong><span>没有待恢复的发布事务。</span></div>
					{/if}
				</div>
			{/if}
		</div>
		{#if confirmRequest}
			<WorkflowDialogShell
				titleId="settings-confirm-title"
				descriptionId="settings-confirm-desc"
				title="确认操作"
				description={confirmRequest.message}
				maxWidth="420px"
				onClose={() => (confirmRequest = null)}
			>
				<div slot="footer" class="confirm-actions">
					<button type="button" class="wf-quiet" on:click={() => (confirmRequest = null)}>取消</button>
					<button type="button" class="wf-primary" on:click={runConfirmed}>确认</button>
				</div>
			</WorkflowDialogShell>
		{/if}
	</dialog>
{/if}

<style>
	.settings-dialog {
		padding: 0;
		border: 0;
		background: transparent;
		color: var(--text);
		width: min(calc(100vw - 48px), 760px);
		max-width: calc(100vw - 16px);
		max-height: calc(100vh - 16px);
		animation: fadeIn 0.15s ease;
	}
	.settings-dialog::backdrop {
		background: rgba(0, 0, 0, 0.15);
	}
	.settings-panel {
		background: var(--bg);
		border: 1px solid var(--hr);
		border-radius: 12px;
		padding: 24px;
		width: 100%;
		min-width: 0;
		box-sizing: border-box;
		max-height: min(calc(100vh - 48px), 90vh);
		overflow-x: hidden;
		overflow-y: auto;
		box-shadow: 0 8px 32px rgba(0, 0, 0, 0.12);
	}
	.nested-title {
		font-size: 12px;
		font-weight: 600;
		margin: 12px 0 10px;
		color: var(--text-secondary);
	}
	.backup-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		font-size: 12px;
		color: var(--text-secondary);
	}
	.backup-row + .backup-row {
		margin-top: 6px;
	}
	.advanced-toggle {
		margin-top: 22px;
		width: 100%;
		border: 1px dashed var(--hr);
		border-radius: 8px;
		background: transparent;
		color: var(--text-secondary);
		font: inherit;
		font-size: 12px;
		padding: 10px 12px;
		cursor: pointer;
		text-align: left;
	}
	.advanced-toggle:hover {
		border-color: var(--link);
		color: var(--text);
	}
	.advanced-block {
		margin-top: 12px;
		min-width: 0;
	}
	.library-card {
		margin-top: 12px;
	}
	.settings-header {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		margin-bottom: 18px;
	}
	.settings-subtitle,
	.status-line {
		font-size: 11px;
		color: var(--text-faded);
	}
	.close-btn {
		border: 0;
		border-radius: 6px;
		background: transparent;
		color: var(--text-secondary);
		font-size: 24px;
		line-height: 1;
		cursor: pointer;
	}
	.close-btn:hover {
		color: var(--text);
		background: var(--bg-secondary);
	}
	.close-btn:focus-visible {
		outline: 2px solid var(--link);
		outline-offset: 2px;
	}
	.notice {
		margin-bottom: 14px;
		padding: 8px 10px;
		border-radius: 8px;
		background: var(--bg-secondary);
		color: var(--text-secondary);
		font-size: 11px;
	}
	.section-title {
		margin-top: 24px;
	}
	.path-list {
		display: flex;
		flex-direction: column;
		gap: 7px;
	}
	.path-row {
		display: grid;
		grid-template-columns: 78px minmax(0, 1fr) auto;
		align-items: center;
		gap: 8px;
		font-size: 11px;
		color: var(--text-secondary);
	}
	.path-row code {
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		color: var(--text-faded);
	}
	.path-value {
		display: flex;
		align-items: center;
		gap: 8px;
		min-width: 0;
	}
	.path-value small {
		flex: none;
		color: var(--text-faded);
		font-size: 10px;
	}
	.path-actions {
		display: flex;
		gap: 4px;
	}
	.mini-btn,
	.action-btn {
		border: 1px solid var(--hr);
		border-radius: 7px;
		background: var(--bg);
		color: var(--text-secondary);
		font-size: 11px;
		padding: 5px 9px;
		cursor: pointer;
	}
	.mini-btn:hover,
	.action-btn:hover:not(:disabled) {
		border-color: var(--link);
		color: var(--text);
	}
	.mini-btn:disabled,
	.action-btn:disabled {
		cursor: not-allowed;
		opacity: 0.55;
	}
	.mini-btn.danger {
		color: var(--danger);
	}
	.confirm-actions {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
	}
	.action-grid {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		gap: 8px;
	}
	.status-card {
		display: flex;
		align-items: center;
		gap: 8px;
		flex-wrap: wrap;
		margin-top: 10px;
		padding: 9px 10px;
		border-radius: 8px;
		background: var(--bg-secondary);
		font-size: 11px;
		color: var(--text-secondary);
	}
	.update-card {
		padding: 11px 12px;
		border: 1px solid var(--hr);
		border-radius: 8px;
		background: var(--bg-secondary);
	}
	.update-head,
	.update-release,
	.update-confirm {
		display: grid;
		grid-template-columns: minmax(0, 1fr) auto;
		align-items: center;
		gap: 10px;
	}
	.update-head > div,
	.update-release > div,
	.update-confirm > div {
		display: grid;
		gap: 3px;
		min-width: 0;
	}
	.update-card strong { color: var(--text); font-size: 12px; }
	.update-card span { color: var(--text-faded); font-size: 11px; line-height: 1.45; overflow-wrap: anywhere; }
	.update-card progress { width: 100%; height: 5px; margin-top: 9px; accent-color: var(--link); }
	.update-release,
	.update-confirm { margin-top: 9px; padding-top: 9px; border-top: 1px solid var(--hr); }
	.update-confirm { grid-template-columns: minmax(0, 1fr) auto auto; }
	.update-primary { border-color: var(--link); color: var(--text); }
	.status-card strong {
		color: var(--text);
	}
	.status-card span {
		flex: 1 1 100%;
	}
	.credential-row,
	.credential-form {
		display: flex;
		align-items: center;
		gap: 8px;
	}
	.credential-row {
		justify-content: space-between;
		font-size: 11px;
		color: var(--text-secondary);
	}
	.credential-form {
		margin-top: 8px;
	}
	.credential-form input {
		min-width: 0;
		flex: 1;
		border: 1px solid var(--hr);
		border-radius: 7px;
		background: var(--bg-secondary);
		color: var(--text);
		padding: 7px 9px;
		font-size: 11px;
	}
	.settings-title {
		font-size: 14px;
		font-weight: 600;
		color: var(--text);
		margin-bottom: 16px;
	}
	.theme-grid {
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: 8px;
	}
	.theme-option {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 6px;
		padding: 10px;
		border: 1.5px solid var(--hr);
		border-radius: 12px;
		background: var(--bg);
		cursor: pointer;
		transition: all 0.3s cubic-bezier(0.2, 0.8, 0.2, 1);
		position: relative;
		overflow: hidden;
	}
	.theme-option::after {
		content: ''; position: absolute; inset: 0;
		/* 与 .icon-btn::after 同一套主题派生 sheen——纯白渐变在暗主题下是奶雾。 */
		background: linear-gradient(135deg, color-mix(in srgb, var(--text) 10%, transparent) 0%, transparent 50%, color-mix(in srgb, var(--text) 4%, transparent) 100%);
		opacity: 0; transition: opacity 0.3s ease;
		pointer-events: none;
	}
	.theme-option:hover {
		border-color: var(--text-faded);
		transform: translateY(-2px);
		box-shadow: 0 6px 16px rgba(0, 0, 0, 0.08), inset 0 1px 1px color-mix(in srgb, var(--text) 20%, transparent);
	}
	.theme-option:hover::after {
		opacity: 1;
	}
	.theme-option:active {
		transform: translateY(0);
		box-shadow: 0 2px 4px rgba(0, 0, 0, 0.05);
	}
	.theme-option.active {
		border-color: var(--link);
		background: var(--bg-secondary);
	}
	.theme-option span {
		font-size: 11px;
		color: var(--text-secondary);
	}
	.theme-preview {
		width: 100%;
		height: 40px;
		border-radius: 4px;
		display: flex;
		align-items: center;
		justify-content: center;
		font-size: 16px;
		font-weight: 500;
	}


	.typo-rows {
		display: flex;
		flex-direction: column;
		gap: 10px;
	}
	.typo-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
	}
	.typo-label {
		font-size: 12px;
		color: var(--text-secondary);
	}
	.typo-options {
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.typo-btn {
		min-width: 44px;
		height: 28px;
		padding: 0 10px;
		border: 1.5px solid var(--hr);
		border-radius: 8px;
		background: var(--bg);
		color: var(--text-secondary);
		font-size: 12px;
		cursor: pointer;
		transition: all 0.25s cubic-bezier(0.2, 0.8, 0.2, 1);
	}
	.typo-btn:hover {
		border-color: var(--text-faded);
		color: var(--text);
		transform: translateY(-1px);
	}
	.typo-btn.active {
		border-color: var(--link);
		background: var(--bg-secondary);
		color: var(--text);
	}
	.typo-serif {
		font-family: Georgia, "Source Han Serif SC", "Noto Serif SC", "STSong", "SimSun", serif;
	}
	.typo-value {
		min-width: 48px;
		text-align: center;
		font-size: 12px;
		color: var(--text);
	}

	.toggle-switch {
		position: relative;
		display: inline-block;
		width: 40px;
		height: 22px;
		cursor: pointer;
	}
	.toggle-switch input {
		position: absolute;
		opacity: 0;
		width: 0;
		height: 0;
	}
	.toggle-slider {
		position: absolute;
		inset: 0;
		border-radius: 999px;
		background: var(--hr);
		transition: background 0.2s ease;
	}
	.toggle-slider::before {
		content: '';
		position: absolute;
		left: 2px;
		top: 2px;
		width: 18px;
		height: 18px;
		border-radius: 50%;
		background: var(--bg);
		transition: transform 0.2s ease;
	}
	.toggle-switch input:checked + .toggle-slider {
		background: var(--link);
	}
	.toggle-switch input:checked + .toggle-slider::before {
		transform: translateX(18px);
	}
	.toggle-switch input:focus-visible + .toggle-slider {
		outline: 2px solid var(--link);
		outline-offset: 2px;
	}

	@keyframes fadeIn {
		from { opacity: 0; }
		to { opacity: 1; }
	}
</style>
