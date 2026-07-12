<script lang="ts">
	import { onMount } from "svelte";
	import { invoke } from "@tauri-apps/api/core";
	import {
		settingsOpen,
		currentTheme,
		fontScale,
		clampFontScale,
		FONT_SCALE_STEP,
		readingLineHeight,
		readingWidth,
		readingFontFamily,
		READING_LINE_HEIGHTS,
		READING_WIDTHS,
	} from "$lib/stores/app";
	import { getThemePairs } from "$lib/theme/themes";

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
	type SecretStatus = { configured: boolean; target: string; lastVerifiedAt?: string | null };
	type MigrationPreview = {
		items: Array<{ kind: string; exists: boolean; bytes: number; sensitive: boolean; conflict: boolean }>;
		totalBytes: number;
		conflictCount: number;
		sensitiveItemCount: number;
	};
	type CacheClearResult = { deletedItems: number; releasedBytes: number; skipped: Array<{ reason: string }> };
	type PublishTransaction = { transactionId: string; phase: string; bookId: string };
	type StateBackupResult = { backupPath: string; included: string[]; skipped: string[] };
	type MigrationRun = { migrationId: string; previewId: string; scope: string; status: string; receiptPath?: string | null };

	let locations: StorageLocations | null = null;
	let usage: StorageUsage | null = null;
	let secretStatus: SecretStatus | null = null;
	let migrationPreview: MigrationPreview | null = null;
	let publishRecovery: PublishTransaction[] = [];
	let migrationRuns: MigrationRun[] = [];
	let backupResult: StateBackupResult | null = null;
	let panelLoading = false;
	let actionBusy = false;
	let panelNotice = "";
	let apiKey = "";
	let stopSubscription: (() => void) | undefined;
	let storageRows: Array<[string, string, string, number]> = [];
	$: storageRows = locations
		? [
				["library", "Library", locations.libraryRoot, usage?.libraryBytes ?? 0],
				["data", "Data", locations.dataRoot, usage?.dataBytes ?? 0],
				["cache", "Cache", locations.cacheRoot, usage?.cacheBytes ?? 0],
				["logs", "Logs", locations.logsRoot, usage?.logsBytes ?? 0],
				["backups", "Backups", locations.backupsRoot, usage?.backupsBytes ?? 0],
				["runtime_state", "RuntimeState", locations.runtimeStateRoot, usage?.runtimeStateBytes ?? 0]
			]
		: [];

	onMount(() => {
		stopSubscription = settingsOpen.subscribe((open) => {
			if (open) void loadPanel();
		});
		return () => stopSubscription?.();
	});

	async function loadPanel() {
		if (panelLoading) return;
		panelLoading = true;
		panelNotice = "";
		try {
			[locations, usage, secretStatus, publishRecovery, migrationRuns] = await Promise.all([
				invoke<StorageLocations>("get_storage_locations"),
				invoke<StorageUsage>("get_storage_usage"),
				invoke<SecretStatus>("get_secret_status"),
				invoke<PublishTransaction[]>("get_publish_recovery_status"),
				invoke<MigrationRun[]>("get_migration_runs")
			]);
		} catch (error) {
			panelNotice = `设置状态读取失败：${String(error)}`;
		} finally {
			panelLoading = false;
		}
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
			panelNotice = `复制失败：${String(error)}`;
		}
	}

	async function revealDirectory(kind: string) {
		try {
			await invoke("reveal_storage_directory", { kind });
		} catch (error) {
			panelNotice = `无法打开目录：${String(error)}`;
		}
	}

	async function clearCache() {
		if (actionBusy || !window.confirm("仅清理不受保护的可重建缓存，不会删除 Library、Data 或 Backups。继续？")) return;
		actionBusy = true;
		try {
			const result = await invoke<CacheClearResult>("clear_safe_cache", {
				categories: ["podcast_completed", "zhihu_browser_cache", "general_temporary"],
				taskIds: []
			});
			panelNotice = `已清理 ${result.deletedItems} 项，释放 ${formatBytes(result.releasedBytes)}${result.skipped.length ? `，跳过 ${result.skipped.length} 项受保护任务` : ""}`;
		} catch (error) {
			panelNotice = `缓存清理失败：${String(error)}`;
		} finally {
			actionBusy = false;
		}
	}

	async function createStateBackup() {
		if (actionBusy || !window.confirm("创建当前 channel 的状态备份？Library、Cache、Logs、凭据和浏览器 Profile 会被排除。")) return;
		actionBusy = true;
		try {
			backupResult = await invoke<StateBackupResult>("create_state_backup");
			panelNotice = "状态备份已创建";
		} catch (error) {
			panelNotice = `状态备份失败：${String(error)}`;
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
			panelNotice = `Key 保存失败：${String(error)}`;
		} finally {
			actionBusy = false;
		}
	}

	async function deleteApiKey() {
		if (actionBusy || !window.confirm("删除当前 channel 的 DeepSeek Key？")) return;
		actionBusy = true;
		try {
			secretStatus = await invoke<SecretStatus>("delete_deepseek_api_key");
			panelNotice = "DeepSeek Key 已删除";
		} catch (error) {
			panelNotice = `Key 删除失败：${String(error)}`;
		} finally {
			actionBusy = false;
		}
	}

	async function previewMigration() {
		if (actionBusy) return;
		actionBusy = true;
		try {
			migrationPreview = await invoke<MigrationPreview>("preview_legacy_migration", { scope: "all" });
			panelNotice = "迁移 preview 已刷新；未写入任何数据";
		} catch (error) {
			panelNotice = `迁移 preview 失败：${String(error)}`;
		} finally {
			actionBusy = false;
		}
	}

	async function recoverPublish() {
		if (actionBusy || !publishRecovery.length || !window.confirm("恢复所有未完成的发布事务？")) return;
		actionBusy = true;
		try {
			publishRecovery = await invoke<PublishTransaction[]>("recover_publish_transactions", { transactionIds: null });
			panelNotice = "发布恢复检查已完成";
		} catch (error) {
			panelNotice = `发布恢复失败：${String(error)}`;
		} finally {
			actionBusy = false;
		}
	}

	const widthLabels: Record<number, string> = { 680: "窄", 760: "标准", 840: "宽" };

	function adjustFontScale(direction: number) {
		$fontScale = clampFontScale($fontScale + direction * FONT_SCALE_STEP);
	}
</script>

{#if $settingsOpen}
	<!-- svelte-ignore a11y-click-events-have-key-events -->
	<!-- svelte-ignore a11y-no-static-element-interactions -->
	<div
		class="settings-overlay"
		on:click={() => ($settingsOpen = false)}
		role="presentation"
	>
		<div
			class="settings-panel"
			on:click|stopPropagation
			on:keydown|stopPropagation
			role="presentation"
		>
			<div class="settings-header">
				<div>
					<div class="settings-title">设置</div>
					<div class="settings-subtitle">{locations?.channel ?? "当前 channel"}</div>
				</div>
				<button class="close-btn" type="button" on:click={() => ($settingsOpen = false)} aria-label="关闭设置">×</button>
			</div>
			{#if panelLoading}
				<div class="status-line">正在读取本地状态…</div>
			{/if}
			{#if panelNotice}
				<div class="notice" role="status">{panelNotice}</div>
			{/if}

			<div class="settings-title">主题</div>
			<div class="theme-grid">
				{#each themePairs as pair}
					<button
						class="theme-option"
						class:active={$currentTheme.name === pair.light.name}
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

			<div class="settings-title typo-title">排版</div>
			<div class="typo-rows">
				<div class="typo-row">
					<span class="typo-label">字号</span>
					<div class="typo-options">
						<button class="typo-btn" on:click={() => adjustFontScale(-1)} title="缩小 (Ctrl+-)">−</button>
						<span class="typo-value">{Math.round($fontScale * 100)}%</span>
						<button class="typo-btn" on:click={() => adjustFontScale(1)} title="放大 (Ctrl+=)">+</button>
					</div>
				</div>
				<div class="typo-row">
					<span class="typo-label">行距</span>
					<div class="typo-options">
						{#each READING_LINE_HEIGHTS as lh}
							<button
								class="typo-btn"
								class:active={$readingLineHeight === lh}
								on:click={() => ($readingLineHeight = lh)}
							>
								{lh.toFixed(1)}
							</button>
						{/each}
					</div>
				</div>
				<div class="typo-row">
					<span class="typo-label">栏宽</span>
					<div class="typo-options">
						{#each READING_WIDTHS as w}
							<button
								class="typo-btn"
								class:active={$readingWidth === w}
								on:click={() => ($readingWidth = w)}
							>
								{widthLabels[w]}
							</button>
						{/each}
					</div>
				</div>
				<div class="typo-row">
					<span class="typo-label">字体</span>
					<div class="typo-options">
						<button
							class="typo-btn"
							class:active={$readingFontFamily === "sans"}
							on:click={() => ($readingFontFamily = "sans")}
						>
							黑体
						</button>
						<button
							class="typo-btn typo-serif"
							class:active={$readingFontFamily === "serif"}
							on:click={() => ($readingFontFamily = "serif")}
						>
							宋体
						</button>
					</div>
				</div>
			</div>

			<div class="settings-title section-title">存储路径</div>
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

			<div class="settings-title section-title">维护与恢复</div>
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
					<strong>迁移恢复记录 {migrationRuns.length}</strong>
					<span>{migrationRuns.slice(0, 4).map((run) => `${run.scope} · ${run.status}`).join("；")}</span>
				</div>
			{/if}
			{#if backupResult}
				<div class="status-card">
					<strong>状态备份已创建</strong>
					<span>{backupResult.backupPath} · 包含 {backupResult.included.join("、") || "无"} · 排除 {backupResult.skipped.join("、")}</span>
				</div>
			{/if}
			{#if publishRecovery.length}
				<div class="status-card recovery-card">
					<strong>待恢复发布事务 {publishRecovery.length}</strong>
					<span>{publishRecovery.map((item) => `${item.bookId} · ${item.phase}`).join("；")}</span>
					<button type="button" class="mini-btn" disabled={actionBusy} on:click={() => void recoverPublish()}>执行恢复检查</button>
				</div>
			{/if}

			<div class="settings-title section-title">凭据</div>
			<div class="credential-row">
				<span>{secretStatus?.configured ? "DeepSeek Key 已配置" : "未配置 DeepSeek Key"}</span>
				{#if secretStatus?.configured}<button type="button" class="mini-btn danger" disabled={actionBusy} on:click={() => void deleteApiKey()}>删除</button>{/if}
			</div>
			<div class="credential-form">
				<input type="password" bind:value={apiKey} autocomplete="new-password" placeholder="输入 Key（不会显示或写入 JSON）" aria-label="DeepSeek API Key" />
				<button type="button" class="mini-btn" disabled={actionBusy || !apiKey.trim()} on:click={() => void saveApiKey()}>保存</button>
			</div>
		</div>
	</div>
{/if}

<style>
	.settings-overlay {
		position: fixed;
		inset: 0;
		z-index: 40;
		background: rgba(0, 0, 0, 0.15);
		display: flex;
		align-items: center;
		justify-content: center;
		animation: fadeIn 0.15s ease;
	}
	.settings-panel {
		background: var(--bg);
		border: 1px solid var(--hr);
		border-radius: 12px;
		padding: 24px;
		max-width: 760px;
		width: min(94vw, 760px);
		max-height: 90vh;
		overflow-y: auto;
		box-shadow: 0 8px 32px rgba(0, 0, 0, 0.12);
		animation: scaleIn 0.2s ease;
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
		background: transparent;
		color: var(--text-secondary);
		font-size: 24px;
		line-height: 1;
		cursor: pointer;
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
		color: var(--danger, #a33);
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
		background: linear-gradient(135deg, rgba(255, 255, 255, 0.4) 0%, rgba(255, 255, 255, 0) 50%, rgba(255, 255, 255, 0.1) 100%);
		opacity: 0; transition: opacity 0.3s ease;
		pointer-events: none;
	}
	.theme-option:hover {
		border-color: var(--text-faded);
		transform: translateY(-2px);
		box-shadow: 0 6px 16px rgba(0, 0, 0, 0.08), inset 0 1px 1px rgba(255, 255, 255, 0.2);
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

	.typo-title {
		margin-top: 20px;
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

	@keyframes fadeIn {
		from { opacity: 0; }
		to { opacity: 1; }
	}
	@keyframes scaleIn {
		from { transform: scale(0.95); opacity: 0; }
		to { transform: scale(1); opacity: 1; }
	}
</style>
