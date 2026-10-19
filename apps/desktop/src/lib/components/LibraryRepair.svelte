<script lang="ts">
    import { invokeWithTimeout } from '$lib/ipc';
    export let onChanged: () => void = () => {};
    let busy = false;
    let count = 0;
    let titles: string[] = [];
    let message = '';
    async function repair() {
        if (busy) return;
        busy = true;
        try {
            const apply = count > 0;
            const result = await invokeWithTimeout<{groups: string[]; duplicateCount: number; mergedCount: number}>('repair_library_duplicates', {apply}, 120000);
            count = apply ? 0 : result.duplicateCount; titles = result.groups;
            message = apply ? `已合并 ${result.mergedCount} 份重复书目，原副本可在回收站恢复。` : count ? `发现 ${count} 份相同副本：${titles.slice(0,3).join('、')}。合并会保留最新进度和全部书签，原副本进入回收站。` : '没有发现内容完全相同的书目。';
            if (apply) onChanged();
        } catch (error) { count = 0; message = `检查未完成：${String(error)}`; onChanged(); }
        finally { busy = false; }
    }
</script>
<div class="library-repair">
    <button type="button" onclick={repair} disabled={busy}>{busy ? '正在检查…' : count ? `合并 ${count} 份相同副本` : '检查重复书目'}</button>
    {#if message}<p role="status">{message}</p>{/if}
</div>
<style>
    .library-repair { min-width: 0; }
    button { min-height: 44px; padding: 8px 12px; border: 1px solid var(--hr); border-radius: 10px; background: var(--bg); color: var(--text-secondary); font: inherit; font-size: 13px; cursor: pointer; }
    button:disabled { opacity: .6; cursor: wait; }
    p { max-width: 38rem; margin: 8px 0; color: var(--text-secondary); font-size: 13px; line-height: 1.6; overflow-wrap: anywhere; }
</style>
