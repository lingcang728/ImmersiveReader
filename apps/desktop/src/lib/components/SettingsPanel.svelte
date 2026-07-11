<script lang="ts">
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
		max-width: 400px;
		width: 90vw;
		box-shadow: 0 8px 32px rgba(0, 0, 0, 0.12);
		animation: scaleIn 0.2s ease;
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
