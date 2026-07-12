<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { TaskSnapshot } from '$lib/tasks/sync';

  type ItemTypes = 'answers' | 'articles' | 'all';
  type SortBy = 'time' | 'vote';

  interface LoginStatus {
    loggedIn: boolean;
  }

  interface CreateRequest {
    peopleId: string;
    itemTypes: ItemTypes;
    topN: number | null;
    sortBy: SortBy;
  }

  export let tasks: readonly TaskSnapshot[] = [];
  export let onClose: () => void;
  export let onRefreshTasks: () => void;
  export let onStartTask: (taskId: string, revision: number) => void;
  export let onControlTask: (
    taskId: string,
    action: 'pause' | 'resume' | 'cancel',
    revision: number
  ) => void;
  export let onFallback: () => void;

  let peopleId = '';
  let itemTypes: ItemTypes = 'all';
  let sortBy: SortBy = 'time';
  let topN: number | '' = '';
  let loginStatus: LoginStatus | null = null;
  let createdTaskId = '';
  let busy = false;
  let errorText = '';
  let noticeText = '';

  $: createdTask = createdTaskId
    ? tasks.find((task) => task.id === createdTaskId) ?? null
    : null;

  function taskStateLabel(task: TaskSnapshot): string {
    if (task.lifecycleState === 'queued') return '等待开始';
    if (task.lifecycleState === 'starting') return '正在启动';
    if (task.lifecycleState === 'running') return '抓取中';
    if (task.lifecycleState === 'paused') return '已暂停';
    if (task.lifecycleState === 'terminal') {
      if (task.outcome === 'success') return '已完成';
      if (task.outcome === 'partial_success') return '部分完成';
      if (task.outcome === 'cancelled') return '已取消';
      return '失败';
    }
    return task.progress.label ?? '处理中';
  }

  function progressLabel(task: TaskSnapshot): string {
    if (task.progress.percent === undefined) return task.progress.label ?? '等待进度';
    return String(Math.round(task.progress.percent)) + '% · ' + (task.progress.label ?? '');
  }

  async function refreshLoginStatus() {
    try {
      loginStatus = await invoke<LoginStatus>('get_zhihu_login_status');
      errorText = '';
    } catch (error) {
      loginStatus = null;
      errorText = '无法读取登录状态：' + String(error);
    }
  }

  async function createTask() {
    const normalized = peopleId.trim();
    if (!normalized) {
      errorText = '请输入知乎答主 ID。';
      return;
    }
    if (topN !== '' && (!Number.isInteger(Number(topN)) || Number(topN) < 1 || Number(topN) > 5000)) {
      errorText = 'Top N 必须是 1–5000 的整数。';
      return;
    }
    busy = true;
    errorText = '';
    noticeText = '';
    const request: CreateRequest = {
      peopleId: normalized,
      itemTypes,
      topN: topN === '' ? null : Number(topN),
      sortBy
    };
    try {
      const snapshot = await invoke<TaskSnapshot>('create_zhihu_task', { request });
      createdTaskId = snapshot.id;
      noticeText = '任务已加入统一队列；可从这里或书架任务栏开始。';
      onRefreshTasks();
    } catch (error) {
      errorText = '创建任务失败：' + String(error);
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    void refreshLoginStatus();
  });
</script>

<div class="zhihu-modal" role="presentation" on:click|self={onClose}>
  <dialog open class="zhihu-panel" aria-labelledby="zhihu-title">
    <header class="zhihu-header">
      <div>
        <span class="eyebrow">ZHIHU WORKFLOW</span>
        <h1 id="zhihu-title">归档知乎</h1>
        <p>任务、登录态和抓取进度都留在沉浸阅读的统一队列中。</p>
      </div>
      <button type="button" class="close-button" aria-label="关闭" on:click={onClose}>×</button>
    </header>

    <div class="zhihu-body">
      <section class="login-card" aria-label="知乎登录状态">
        <div>
          <span class="label">登录状态</span>
          <strong class:ready={loginStatus?.loggedIn === true}>
            {loginStatus === null ? '检查中…' : loginStatus.loggedIn ? '已登录' : '需要登录'}
          </strong>
        </div>
        <div class="login-actions">
          <button type="button" class="quiet-button" on:click={() => void refreshLoginStatus()}>刷新</button>
          <button type="button" class="quiet-button" on:click={onFallback}>打开旧版登录/控制台</button>
        </div>
      </section>

      <label class="field-row">
        <span>知乎答主 ID</span>
        <input bind:value={peopleId} placeholder="例如 xiao-xue-shi-46-24" autocomplete="off" />
      </label>

      <fieldset class="choice-field">
        <legend>内容类型</legend>
        <label><input type="radio" bind:group={itemTypes} value="all" />回答 + 文章</label>
        <label><input type="radio" bind:group={itemTypes} value="answers" />仅回答</label>
        <label><input type="radio" bind:group={itemTypes} value="articles" />仅文章</label>
      </fieldset>

      <div class="options-grid">
        <label class="field-row">
          <span>排序</span>
          <select bind:value={sortBy}>
            <option value="time">发布时间（新到旧）</option>
            <option value="vote">点赞数（高到低）</option>
          </select>
        </label>
        <label class="field-row">
          <span>Top N（留空为全部）</span>
          <input type="number" min="1" max="5000" step="1" bind:value={topN} placeholder="例如 5" />
        </label>
      </div>

      {#if createdTask}
        <section class="task-card" aria-label="知乎任务结果">
          <div class="task-title">
            <strong>{taskStateLabel(createdTask)}</strong>
            <span>{createdTask.id}</span>
          </div>
          <p>{progressLabel(createdTask)}</p>
          {#if createdTask.lifecycleState === 'queued'}
            <button type="button" class="primary-button" on:click={() => onStartTask(createdTask.id, createdTask.revision)}>开始抓取</button>
          {:else if createdTask.canPause}
            <button type="button" class="secondary-button" on:click={() => onControlTask(createdTask.id, 'pause', createdTask.revision)}>暂停</button>
          {:else if createdTask.canResume}
            <button type="button" class="secondary-button" on:click={() => onControlTask(createdTask.id, 'resume', createdTask.revision)}>恢复</button>
          {:else if createdTask.canCancel}
            <button type="button" class="secondary-button" on:click={() => onControlTask(createdTask.id, 'cancel', createdTask.revision)}>取消</button>
          {/if}
          {#if createdTask.lifecycleState === 'terminal'}
            <div class="result-card">
              <strong>结果页</strong>
              <span>{createdTask.progress.label ?? '任务已结束'}</span>
              {#if createdTask.errorMessage}<small>{createdTask.errorMessage}</small>{/if}
            </div>
          {/if}
        </section>
      {/if}

      {#if errorText}<p class="message error" role="alert">{errorText}</p>{/if}
      {#if noticeText}<p class="message success" role="status">{noticeText}</p>{/if}
    </div>

    <footer class="zhihu-footer">
      <button type="button" class="quiet-button" on:click={onFallback}>保留并打开旧版控制台</button>
      <div>
        <button type="button" class="quiet-button" on:click={onClose}>稍后处理</button>
        <button type="button" class="primary-button" disabled={busy} on:click={() => void createTask()}>{busy ? '创建中…' : '加入任务队列'}</button>
      </div>
    </footer>
  </dialog>
</div>

<style>
  .zhihu-modal { position: fixed; inset: 0; z-index: 40; display: grid; place-items: center; padding: 24px; background: rgba(5, 12, 24, .68); backdrop-filter: blur(8px); color: var(--text); }
  .zhihu-panel { width: min(650px, 100%); max-height: min(760px, 92vh); overflow: auto; margin: 0; border: 1px solid color-mix(in srgb, var(--link) 45%, var(--hr)); border-radius: 20px; padding: 0; background: var(--bg-secondary); box-shadow: 0 28px 90px rgba(0, 0, 0, .45); }
  .zhihu-header { display: flex; justify-content: space-between; gap: 20px; padding: 28px 30px 22px; border-bottom: 1px solid var(--hr); }
  .eyebrow { color: var(--link); font-size: 10px; letter-spacing: .16em; }
  h1 { margin: 8px 0 6px; font-size: 26px; }
  .zhihu-header p { margin: 0; color: var(--text-secondary); font-size: 13px; }
  .close-button { width: 34px; height: 34px; border: 1px solid var(--hr); border-radius: 50%; background: transparent; color: var(--text-secondary); font-size: 22px; cursor: pointer; }
  .zhihu-body { display: grid; gap: 16px; padding: 24px 30px; }
  .login-card, .task-title, .zhihu-footer, .login-actions { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .login-card, .task-card, .result-card { border: 1px solid var(--hr); border-radius: 12px; padding: 13px 14px; background: color-mix(in srgb, var(--bg) 45%, transparent); }
  .login-card { padding: 12px 14px; }
  .label, .field-row span, .choice-field legend { display: block; color: var(--text-faded); font-size: 11px; }
  .login-card strong { display: block; margin-top: 4px; color: #d6a84f; }
  .login-card strong.ready { color: #82d3a4; }
  .field-row { display: grid; gap: 6px; }
  .field-row input, .field-row select { min-width: 0; border: 1px solid var(--hr); border-radius: 8px; padding: 9px 10px; background: var(--bg); color: var(--text); font: inherit; }
  .choice-field { display: flex; flex-wrap: wrap; gap: 14px; border: 1px solid var(--hr); border-radius: 10px; padding: 10px 12px; color: var(--text-secondary); font-size: 13px; }
  .choice-field legend { padding: 0 5px; }
  .choice-field label { display: flex; align-items: center; gap: 6px; }
  .options-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }
  .task-card { display: grid; gap: 10px; }
  .task-title span { max-width: 260px; overflow: hidden; color: var(--text-faded); font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
  .task-card p, .result-card span, .result-card small, .message { margin: 0; color: var(--text-secondary); font-size: 12px; line-height: 1.5; }
  .result-card { display: grid; gap: 4px; border-color: color-mix(in srgb, var(--link) 45%, var(--hr)); }
  .result-card strong { color: var(--link); }
  .result-card small { color: #ef8b8b; }
  .message.error { color: #ef8b8b; }
  .message.success { color: #82d3a4; }
  .zhihu-footer { padding: 18px 30px 24px; border-top: 1px solid var(--hr); }
  .zhihu-footer > div { display: flex; gap: 8px; }
  .quiet-button, .secondary-button, .primary-button { border: 1px solid var(--hr); border-radius: 9px; padding: 9px 13px; cursor: pointer; font: inherit; font-size: 12px; }
  .quiet-button { background: transparent; color: var(--text-secondary); }
  .secondary-button { background: var(--bg); color: var(--text); }
  .primary-button { border-color: var(--link); background: var(--link); color: white; }
  .quiet-button:hover, .secondary-button:hover { border-color: var(--link); color: var(--text); }
  .primary-button:disabled { cursor: not-allowed; opacity: .45; }
  @media (max-width: 620px) { .options-grid { grid-template-columns: 1fr; } .zhihu-header, .zhihu-body, .zhihu-footer { padding-left: 18px; padding-right: 18px; } .zhihu-footer { align-items: stretch; flex-direction: column; } .zhihu-footer > div { justify-content: flex-end; } }
</style>
