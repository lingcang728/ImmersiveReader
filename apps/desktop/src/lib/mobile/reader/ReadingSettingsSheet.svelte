<!--
	Reading settings sheet: theme, typography, paging and focus toggles.

	State lives in the shared stores (`$lib/stores/app.ts`) so the desktop app
	and the mobile shell agree on typography; every control applies instantly
	and persists through the store subscriptions. Volume-key paging shares the
	`volumeKeyPaging` store from `$lib/platform/device.ts` with the shell.
-->
<script lang="ts">
	import SheetBase from './SheetBase.svelte';
	import { currentTheme, autoFocusMode } from '$lib/stores/app';
	import TypographyControls from '../components/TypographyControls.svelte';
	import { getThemePairs, type Theme } from '$lib/theme/themes';
	import { volumeKeyPaging } from '$lib/platform/device';

	interface Props {
		open: boolean;
		mode: 'markdown' | 'epub' | 'file';
		focusActive: boolean;
		onToggleFocus: () => void;
		onClose: () => void;
	}

	let { open, focusActive, onToggleFocus, onClose }: Props = $props();

	const themePairs = getThemePairs();
	const currentThemeName = $derived($currentTheme.name);

	function selectTheme(theme: Theme) {
		currentTheme.set(theme);
	}

	function toggleVolumePaging() {
		volumeKeyPaging.update((enabled) => !enabled);
	}

</script>

<SheetBase {open} testid="reading-settings-sheet" title="阅读设置" height="80" {onClose}>
	<section class="settings-group">
		<h3 class="settings-label">主题</h3>
		<div class="theme-grid">
			{#each themePairs as pair (pair.label)}
				<button
					type="button"
					class="theme-swatch"
					class:active={currentThemeName === pair.light.name}
					style:background={pair.light.vars['--bg']}
					style:color={pair.light.vars['--text']}
					onclick={() => selectTheme(pair.light)}
					aria-label="{pair.label} 亮色"
				>
					<span class="theme-dot" style:background={pair.light.vars['--link']}></span>
					<span>{pair.label}</span>
				</button>
				<button
					type="button"
					class="theme-swatch dark"
					class:active={currentThemeName === pair.dark.name}
					style:background={pair.dark.vars['--bg']}
					style:color={pair.dark.vars['--text']}
					onclick={() => selectTheme(pair.dark)}
					aria-label="{pair.label} 暗色"
				>
					<span class="theme-dot" style:background={pair.dark.vars['--link']}></span>
					<span>暗</span>
				</button>
			{/each}
		</div>
	</section>

	<section class="settings-group"><h3 class="settings-label">排版</h3><TypographyControls /></section>

	<section class="settings-group">
		<h3 class="settings-label">翻页与专注</h3>
		<div class="settings-row">
			<span class="settings-row-label">音量键翻页</span>
			<button
				type="button"
				class="settings-toggle"
				class:on={$volumeKeyPaging}
				aria-pressed={$volumeKeyPaging}
				aria-label="音量键翻页"
				onclick={toggleVolumePaging}
			><span class="settings-knob"></span></button>
		</div>
		<div class="settings-row">
			<span class="settings-row-label">专注模式</span>
			<button
				type="button"
				class="settings-toggle"
				class:on={focusActive}
				aria-pressed={focusActive}
				aria-label="专注模式"
				onclick={onToggleFocus}
			><span class="settings-knob"></span></button>
		</div>
        <div class="settings-row"><span class="settings-row-label">自动进入专注</span><button type="button" class="settings-toggle" class:on={$autoFocusMode} aria-pressed={$autoFocusMode} aria-label="自动进入专注" onclick={() => autoFocusMode.update(value => !value)}><span class="settings-knob"></span></button></div>
    </section>
</SheetBase>

<style>
	.settings-group {
		padding: 10px 0 4px;
		border-bottom: 1px solid var(--hr);
	}

	.settings-group:last-child {
		border-bottom: 0;
	}

	.settings-label {
		margin: 0 0 10px;
		font-size: 13px;
		font-weight: 500;
		color: var(--text-secondary);
	}

	.settings-row {
		display: flex;
		align-items: center;
		gap: 8px;
		padding-bottom: 10px;
	}

	.settings-row-label {
		flex: 1;
		font-size: 15px;
	}


	.settings-toggle {
		position: relative;
		width: 48px;
		height: 28px;
		flex: 0 0 auto;
		border: 0;
		border-radius: 999px;
		background: var(--hr);
	}

	.settings-toggle.on {
		background: var(--link);
	}

	.settings-knob {
		position: absolute;
		top: 4px;
		left: 4px;
		width: 20px;
		height: 20px;
		border-radius: 50%;
		background: var(--bg);
		transition: transform 0.18s ease;
	}

	.settings-toggle.on .settings-knob {
		transform: translateX(20px);
	}

	.theme-grid {
		display: grid;
		grid-template-columns: repeat(3, minmax(0, 1fr));
		gap: 8px;
		padding-bottom: 12px;
	}

	.theme-swatch {
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 6px;
		min-height: 46px;
		border: 1px solid var(--hr);
		border-radius: 12px;
		font-size: 13.5px;
	}

	.theme-swatch.active {
		border-color: var(--link);
		box-shadow: 0 0 0 2px color-mix(in srgb, var(--link) 28%, transparent);
	}

	.theme-dot {
		width: 10px;
		height: 10px;
		border-radius: 50%;
	}

	@media (prefers-reduced-motion: reduce) {
		.settings-knob {
			transition: none;
		}
	}
</style>
