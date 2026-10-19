<!--
	Mobile settings screen.

	Deliberately narrow: theme, reading typography, page-turn options, storage
	and about — the Android build has no Zhihu/podcast/continuous-reading
	surface, no desktop paths and no update checker. Typography/theme toggles
	write the shared `$lib/stores/app.ts` writables so the reader and the
	desktop app share one persisted preference set.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import * as api from '$lib/mobile/api';
	import { autoFocusMode, currentTheme } from '$lib/stores/app';
	import LibraryRepair from '$lib/components/LibraryRepair.svelte';
    import type { TrashItem } from './contracts';
	import TypographyControls from './components/TypographyControls.svelte';
	import { volumeKeyPaging } from '$lib/platform/device';
	import { getThemePairs } from '$lib/theme/themes';
	import type { PlatformCapabilities } from '$lib/platform/capabilities';
	import Icon from './components/Icon.svelte';
	import type { StorageUsage } from './contracts';
	import { formatBytes } from './shelf';
	import type { ToastKind } from './ui';

	let {
		caps,
		onBack,
		onToast
	}: {
		caps: PlatformCapabilities;
		onBack: () => void;
		onToast: (text: string, kind?: ToastKind) => void;
	} = $props();

	const themePairs = getThemePairs();

	const activeFamilyIndex = $derived(
		Math.max(
			0,
			themePairs.findIndex((pair) => pair.label === $currentTheme.label)
		)
	);

	function setThemeFamily(index: number) {
		const pair = themePairs[index];
		if (!pair) return;
		currentTheme.set($currentTheme.scheme === 'dark' ? pair.dark : pair.light);
	}

	function setScheme(scheme: 'light' | 'dark') {
		const pair = themePairs.find((entry) => entry.label === $currentTheme.label) ?? themePairs[0];
		currentTheme.set(scheme === 'dark' ? pair.dark : pair.light);
	}

	// ----- storage ------------------------------------------------------
	let storage = $state<StorageUsage | null>(null);
	let storageLoading = $state(true);
	let storageError = $state('');
	let clearing = $state(false);
	let clearArmed = $state(false);
	let clearTimer: ReturnType<typeof setTimeout> | null = null;

	async function loadStorage() {
		storageLoading = true;
		storageError = '';
		try {
			storage = await api.getStorageUsage();
		} catch (error) {
			storageError = api.errorText(error);
		} finally {
			storageLoading = false;
		}
	}

	function disarmClear() {
		if (clearTimer === null) return;
		clearTimeout(clearTimer);
		clearTimer = null;
	}

	function handleClearCache() {
		if (clearing) return;
		if (!clearArmed) {
			clearArmed = true;
			disarmClear();
			clearTimer = setTimeout(() => {
				clearTimer = null;
				clearArmed = false;
			}, 4000);
			return;
		}
		disarmClear();
		clearArmed = false;
		void runClearCache();
	}

	async function runClearCache() {
		clearing = true;
		try {
			const result = await api.clearSafeCache();
			onToast(`已清理缓存，释放 ${formatBytes(result.releasedBytes)}`, 'success');
			await loadStorage();
		} catch (error) {
			onToast(api.errorText(error), 'error');
		} finally {
			clearing = false;
		}
	}

    let trashOpen = $state(false);
    let trashItems = $state<TrashItem[]>([]);
    let trashBusy = $state(false);
    async function refreshTrash() {
        trashBusy = true;
        try { trashItems = await api.listTrash(); }
        catch (error) { onToast(api.errorText(error), 'error'); }
        finally { trashBusy = false; }
    }
    async function restoreItem(item: TrashItem) {
        if (trashBusy) return;
        trashBusy = true;
        try { await api.restoreTrashItem(item.trashId, item.revision); onToast('已恢复到书架', 'success'); await refreshTrash(); }
        catch (error) { onToast(api.errorText(error), 'error'); }
        finally { trashBusy = false; }
    }

	// ----- about --------------------------------------------------------
	let version = $state('');

	async function loadVersion() {
		try {
			version = await api.getAppVersion();
		} catch {
			version = '未知';
		}
	}

	onMount(() => {
		void loadStorage();
		void loadVersion();
		return () => {
			if (clearTimer !== null) clearTimeout(clearTimer);
		};
	});
</script>

<div class="settings" data-testid="settings-screen">
	<header class="settings-header">
		<button type="button" class="back-btn" aria-label="返回书架" onclick={onBack}>
			<Icon name="back" />
		</button>
		<h1 class="settings-title">设置</h1>
	</header>

	<div class="settings-scroll ir-scroll">
		<section class="section">
			<h2 class="section-title">外观</h2>
			<div class="theme-grid">
				{#each themePairs as pair, index (pair.label)}
					<button
						type="button"
						class="theme-card"
						class:selected={index === activeFamilyIndex}
						aria-pressed={index === activeFamilyIndex}
						onclick={() => setThemeFamily(index)}
					>
						<span class="theme-swatches" aria-hidden="true">
							<span
								class="swatch swatch-light"
								style={`background: ${pair.light.vars['--bg']}; border-color: ${pair.light.vars['--hr']}`}
							></span>
							<span
								class="swatch swatch-dark"
								style={`background: ${pair.dark.vars['--bg']}; border-color: ${pair.dark.vars['--hr']}`}
							></span>
						</span>
						<span class="theme-label">{pair.label}</span>
					</button>
				{/each}
			</div>
			<div class="segmented scheme-segmented" role="group" aria-label="明暗模式">
				<button
					type="button"
					class="segment"
					class:selected={$currentTheme.scheme === 'light'}
					aria-pressed={$currentTheme.scheme === 'light'}
					onclick={() => setScheme('light')}
				>
					浅色
				</button>
				<button
					type="button"
					class="segment"
					class:selected={$currentTheme.scheme === 'dark'}
					aria-pressed={$currentTheme.scheme === 'dark'}
					onclick={() => setScheme('dark')}
				>
					深色
				</button>
			</div>
		</section>

		<section class="section">
			<h2 class="section-title">排版</h2>
            <TypographyControls />
		</section>

		<section class="section">
			<h2 class="section-title">翻页</h2>
			<div class="card">
				<button
					type="button"
					class="row row-switch"
					role="switch"
					aria-checked={$volumeKeyPaging}
					aria-label="音量键翻页"
					disabled={!caps.volumeKeyBridge}
					onclick={() => volumeKeyPaging.set(!$volumeKeyPaging)}
				>
					<span class="row-copy">
						<span class="row-label">音量键翻页</span>
						<span class="row-hint">
							{caps.volumeKeyBridge
								? '阅读时按音量键上下翻页'
								: '当前设备不支持音量键翻页'}
						</span>
					</span>
					<span class="switch" class:on={$volumeKeyPaging} aria-hidden="true">
						<span class="switch-thumb"></span>
					</span>
				</button>

				<button
					type="button"
					class="row row-switch"
					role="switch"
					aria-checked={$autoFocusMode}
					aria-label="自动进入专注"
					onclick={() => autoFocusMode.set(!$autoFocusMode)}
				>
					<span class="row-copy">
						<span class="row-label">自动进入专注</span>
						<span class="row-hint">打开书籍后自动开启专注模式</span>
					</span>
					<span class="switch" class:on={$autoFocusMode} aria-hidden="true">
						<span class="switch-thumb"></span>
					</span>
				</button>
			</div>
		</section>

		<section class="section">
			<h2 class="section-title">存储</h2>
			<div class="card">
				<div class="row">
					<span class="row-label">书库</span>
					<span class="row-value">
						{storageLoading ? '计算中…' : formatBytes(storage?.libraryBytes ?? 0)}
					</span>
				</div>
				<div class="row">
					<span class="row-label">缓存</span>
					<span class="row-value">
						{storageLoading ? '计算中…' : formatBytes(storage?.cacheBytes ?? 0)}
					</span>
				</div>
				<div class="row">
					<span class="row-label">数据与日志</span>
					<span class="row-value">
						{storageLoading
							? '计算中…'
							: formatBytes((storage?.dataBytes ?? 0) + (storage?.logsBytes ?? 0))}
					</span>
				</div>
				{#if storageError}
					<div class="row row-error">
						<span class="row-hint">存储用量读取失败</span>
						<button type="button" class="text-btn" onclick={() => void loadStorage()}>
							重试
						</button>
					</div>
				{/if}
				<div class="row">
					<button
						type="button"
						class="text-btn text-btn-strong"
						disabled={clearing || storageLoading}
						onclick={handleClearCache}
					>
						{clearing ? '正在清理…' : clearArmed ? '再次点击确认清理' : '清理缓存'}
					</button>
				</div>
			</div>
			<p class="section-note">清理缓存不会删除书库内容与阅读进度。</p>
		</section>

        <section class="section">
            <h2 class="section-title">书库维护</h2>
            <LibraryRepair onChanged={() => void refreshTrash()} />
            <button type="button" class="text-btn" aria-expanded={trashOpen} onclick={() => { trashOpen = !trashOpen; if(trashOpen) void refreshTrash(); }}>回收站</button>
            {#if trashOpen}
                <div class="card">
                    {#if trashBusy}<p class="section-note">读取中…</p>{:else if trashItems.length === 0}<p class="section-note">回收站为空</p>{/if}
                    {#each trashItems as item (item.trashId)}
                        <div class="row"><span class="row-copy">{item.title}</span><button type="button" class="text-btn" disabled={trashBusy} onclick={() => void restoreItem(item)}>恢复</button></div>
                    {/each}
                </div>
            {/if}
        </section>

		<section class="section">
			<h2 class="section-title">关于</h2>
			<div class="card">
				<div class="row">
					<span class="row-label">版本</span>
					<span class="row-value">{version || '读取中…'}</span>
				</div>
				<div class="row">
					<span class="row-label">阅读格式</span>
					<span class="row-value">Markdown · EPUB</span>
				</div>
				<div class="row">
					<span class="row-label">数据位置</span>
					<span class="row-value">仅保存在本机</span>
				</div>
			</div>
			<p class="section-note">
			书库、书签与阅读进度保存在本机。
			</p>
		</section>
	</div>
</div>

<style>
	.settings {
		position: fixed;
		inset: 0;
		z-index: 30;
		display: flex;
		flex-direction: column;
		overflow: hidden;
		background: var(--bg);
		color: var(--text);
	}

	.settings-header {
		display: flex;
		flex: none;
		gap: 4px;
		align-items: center;
		height: calc(52px + max(var(--ir-safe-top, 0px), env(safe-area-inset-top, 0px)));
		padding: max(var(--ir-safe-top, 0px), env(safe-area-inset-top, 0px)) 8px 0;
		border-bottom: 1px solid var(--hr);
	}

	.back-btn {
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
		color: var(--text);
	}

	.back-btn:active {
		background: var(--bg-secondary);
	}

	.settings-title {
		flex: 1;
		min-width: 0;
		margin: 0;
		padding: 0 8px;
		color: var(--heading);
		font-size: 17px;
		font-weight: 600;
	}

	.settings-scroll {
		flex: 1;
		min-height: 0;
		padding: 8px
			max(20px, var(--ir-safe-right, 0px)) calc(32px + max(var(--ir-safe-bottom, 0px), env(safe-area-inset-bottom, 0px)))
			calc(20px + max(var(--ir-safe-left, 0px), env(safe-area-inset-left, 0px)));
	}

	.section {
		margin-top: 20px;
	}

	.section:first-child {
		margin-top: 8px;
	}

	.section-title {
		margin: 0 0 8px 4px;
		color: var(--text-secondary);
		font-size: 13px;
		font-weight: 500;
		letter-spacing: 0.02em;
	}

	.section-note {
		margin: 8px 4px 0;
		color: var(--text-faded);
		font-size: 12px;
		line-height: 1.6;
	}

	.card {
		overflow: hidden;
		border-radius: var(--ir-radius-lg);
		background: var(--bg-secondary);
	}

	.row {
		display: flex;
		gap: 12px;
		align-items: center;
		justify-content: space-between;
		width: 100%;
		min-height: 56px;
		padding: 8px 16px;
		border: 0;
		background: transparent;
		color: var(--text);
		text-align: left;
	}

	.row + .row {
		border-top: 1px solid color-mix(in srgb, var(--hr) 70%, transparent);
	}

	.row-copy {
		display: flex;
		flex: 1;
		flex-direction: column;
		gap: 2px;
		min-width: 0;
	}

	.row-label {
		color: var(--text);
		font-size: 15px;
	}

	.row-hint {
		color: var(--text-faded);
		font-size: 12px;
		line-height: 1.5;
	}

	.row-value {
		flex: none;
		color: var(--text-secondary);
		font-size: 14px;
		font-variant-numeric: tabular-nums;
	}

	.row-switch:active {
		opacity: 0.92;
	}

	.row-switch:disabled {
		opacity: 0.55;
	}

	.row-error {
		color: var(--danger);
	}

	/* ----- theme picker ----- */
	.theme-grid {
		display: grid;
		grid-template-columns: repeat(3, minmax(0, 1fr));
		gap: 8px;
	}

	.theme-card {
		display: flex;
		flex-direction: column;
		gap: 8px;
		align-items: center;
		min-height: 72px;
		padding: 10px 8px;
		border: 1px solid transparent;
		border-radius: var(--ir-radius-md);
		background: var(--bg-secondary);
		color: var(--text-secondary);
	}

	.theme-card.selected {
		border-color: var(--link);
		color: var(--text);
	}

	.theme-swatches {
		display: flex;
		gap: 6px;
	}

	.swatch {
		display: inline-block;
		width: 22px;
		height: 22px;
		border: 1px solid var(--hr);
		border-radius: 50%;
	}

	.theme-label {
		font-size: 13px;
	}

	/* ----- segmented controls ----- */
	.segmented {
		display: flex;
		gap: 4px;
		padding: 4px;
		border-radius: var(--ir-radius-md);
		background: var(--bg-secondary);
	}

	.scheme-segmented {
		margin-top: 8px;
	}

	.segment {
		flex: 1;
		min-width: 56px;
		min-height: 40px;
		padding: 0 8px;
		border: 0;
		border-radius: var(--ir-radius-sm);
		background: transparent;
		color: var(--text-secondary);
		font-size: 14px;
		line-height: 1.2;
		white-space: nowrap;
	}

	.segment.selected {
		background: var(--bg);
		box-shadow:
			inset 0 0 0 1px var(--hr),
			0 1px 3px rgb(0 0 0 / 0.12);
		color: var(--text);
	}

	/* ----- font scale ----- */

	/* ----- switch ----- */
	.switch {
		position: relative;
		display: inline-flex;
		flex: none;
		align-items: center;
		width: 48px;
		height: 28px;
		padding: 3px;
		border-radius: 999px;
		background: var(--hr);
		transition: background var(--ir-duration-fast) var(--ir-ease);
	}

	.switch.on {
		background: var(--link);
	}

	.switch-thumb {
		display: block;
		width: 22px;
		height: 22px;
		border-radius: 50%;
		background: var(--bg);
		box-shadow: 0 1px 3px rgb(0 0 0 / 0.24);
		transition: transform var(--ir-duration-fast) var(--ir-ease);
	}

	.switch.on .switch-thumb {
		transform: translateX(20px);
	}

	/* ----- buttons ----- */
	.text-btn {
		min-height: 44px;
		padding: 0 12px;
		border: 0;
		border-radius: var(--ir-radius-sm);
		background: transparent;
		color: var(--link);
		font-size: 14px;
	}

	.text-btn:active {
		background: color-mix(in srgb, var(--bg) 55%, transparent);
	}

	.text-btn-strong {
		width: 100%;
		text-align: center;
	}

	.text-btn:disabled {
		opacity: 0.55;
	}
</style>
