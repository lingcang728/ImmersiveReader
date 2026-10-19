<script lang="ts">
    import { FONT_SCALE_MAX, FONT_SCALE_MIN, FONT_SCALE_STEP, READING_LINE_HEIGHTS, READING_WIDTHS, clampFontScale, fontScale, readingFontFamily, readingLineHeight, readingWidth } from '$lib/stores/app';
    function adjust(direction: number) { fontScale.set(clampFontScale($fontScale + direction * FONT_SCALE_STEP)); }
</script>
<div class="typography-card" data-testid="typography-controls">
    <div class="font-row">
        <span class="label">字号</span>
        <div class="font-control">
            <button type="button" aria-label="减小字号" disabled={$fontScale <= FONT_SCALE_MIN} onclick={() => adjust(-1)}>−</button>
            <input type="range" aria-label="字号" min={FONT_SCALE_MIN} max={FONT_SCALE_MAX} step={FONT_SCALE_STEP} value={$fontScale} oninput={(event) => fontScale.set(clampFontScale(Number(event.currentTarget.value)))} />
            <button type="button" aria-label="增大字号" disabled={$fontScale >= FONT_SCALE_MAX} onclick={() => adjust(1)}>+</button>
            <span class="percent" data-testid="settings-font-value">{Math.round($fontScale * 100)}%</span>
        </div>
    </div>
    <div class="row"><span class="label">行高</span><div class="segments" role="group" aria-label="行高">
        {#each READING_LINE_HEIGHTS as value}<button type="button" class:selected={$readingLineHeight === value} aria-pressed={$readingLineHeight === value} onclick={() => readingLineHeight.set(value)}>{value === 1.6 ? '紧凑' : value === 2 ? '宽松' : '标准'}</button>{/each}
    </div></div>
    <div class="row"><span class="label">页宽</span><div class="segments" role="group" aria-label="页宽">
        {#each READING_WIDTHS as value}<button type="button" class:selected={$readingWidth === value} aria-pressed={$readingWidth === value} onclick={() => readingWidth.set(value)}>{value === 680 ? '窄' : value === 840 ? '宽' : '标准'}</button>{/each}
    </div></div>
    <div class="row"><span class="label">字体</span><div class="segments" role="group" aria-label="正文字体">
        {#each ['sans', 'serif'] as value}<button type="button" class:selected={$readingFontFamily === value} aria-pressed={$readingFontFamily === value} onclick={() => readingFontFamily.set(value as 'sans' | 'serif')}>{value === 'sans' ? '无衬线' : '衬线'}</button>{/each}
    </div></div>
</div>
<style>
    .typography-card { background: var(--bg-secondary); border-radius: 14px; overflow: hidden; }
    .row, .font-row { padding: 12px 14px; border-bottom: 1px solid var(--hr); display: flex; align-items: center; gap: 12px; min-height: 60px; }
    .row:last-child { border: 0; }
    .font-row { flex-direction: column; align-items: stretch; gap: 12px; }
    .label { font-size: 15px; white-space: nowrap; flex: 1; }
    .segments { display: flex; gap: 4px; }
    button { min-height: 44px; min-width: 44px; padding: 6px 10px; border: 1px solid transparent; border-radius: 10px; background: transparent; color: var(--text-secondary); font: inherit; font-size: 14px; }
    button.selected { background: var(--bg); color: var(--text); border-color: var(--hr); box-shadow: 0 1px 3px #0001; }
    button:disabled { opacity: .4; }
    .font-control { display: flex; align-items: center; gap: 8px; }
    .font-control button { border-color: var(--hr); background: var(--bg); color: var(--text); font-size: 22px; }
    input { flex: 1; min-width: 0; width: 40px; accent-color: var(--link); }
    .percent { width: 42px; font-size: 13px; text-align: right; color: var(--text-secondary); }
    button:focus-visible, input:focus-visible { outline: 2px solid var(--link); outline-offset: 2px; }
</style>
