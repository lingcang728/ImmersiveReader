import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  clearCompletedTasks,
  closeDb,
  getTask,
  initDb,
  resetRunningTasks,
  saveTask,
  tryStartTask,
} from '../src/db.ts';
import {
  cancelTask,
  markTaskQueueable,
  pauseTask,
  queueTask,
} from '../src/scheduler.ts';

function freshDb(): string {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'zhihu-lifecycle-'));
  closeDb();
  initDb(path.join(root, 'zhihu.db'));
  return root;
}

function seedTask(id: string, status: 'pending' | 'running' | 'paused' | 'cancelled' | 'success' | 'failed' | 'partial_success') {
  saveTask({
    id,
    input_url: `https://www.zhihu.com/people/${id}`,
    author_id: `author-${id}`,
    status,
    created_at: Date.now(),
  });
}

const drainQueue = () => new Promise(resolve => setTimeout(resolve, 50));

test('tryStartTask only permits the pending -> running transition', () => {
  const root = freshDb();
  try {
    for (const status of ['running', 'paused', 'cancelled', 'failed', 'success', 'partial_success'] as const) {
      seedTask(`t-${status}`, status);
      assert.equal(tryStartTask(`t-${status}`), false, `status ${status} must not start`);
      assert.equal(getTask(`t-${status}`)?.status, status, `status ${status} must be preserved`);
    }
    seedTask('t-pending', 'pending');
    assert.equal(tryStartTask('t-pending'), true);
    assert.equal(getTask('t-pending')?.status, 'running');
    assert.equal(tryStartTask('t-pending'), false, 'running task must not be started twice');
  } finally {
    closeDb();
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('cancel writes a true cancelled terminal state', () => {
  const root = freshDb();
  try {
    assert.equal(cancelTask('ghost'), false, 'missing task cannot be cancelled');

    seedTask('c-pending', 'pending');
    seedTask('c-paused', 'paused');
    seedTask('c-running', 'running');
    for (const id of ['c-pending', 'c-paused', 'c-running']) {
      assert.equal(cancelTask(id), true, `${id} should cancel`);
      assert.equal(getTask(id)?.status, 'cancelled');
    }
    assert.equal(cancelTask('c-pending'), true, 'cancel is idempotent');

    seedTask('c-success', 'success');
    assert.equal(cancelTask('c-success'), false, 'terminal success cannot be cancelled');
    assert.equal(getTask('c-success')?.status, 'success');

    // cancelled is terminal: never queueable, never resurrected by startup reset.
    assert.equal(markTaskQueueable('c-pending'), false);
    assert.equal(queueTask('c-pending'), false);
    assert.equal(getTask('c-pending')?.status, 'cancelled');
    resetRunningTasks();
    assert.equal(getTask('c-pending')?.status, 'cancelled');
    assert.equal(getTask('c-running')?.status, 'cancelled', 'cancelled must survive running-task reset');

    assert.ok(clearCompletedTasks() >= 4);
    assert.equal(getTask('c-pending'), null, 'cancelled tasks are cleaned up as completed');
  } finally {
    closeDb();
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('pause only applies to non-terminal tasks', () => {
  const root = freshDb();
  try {
    assert.equal(pauseTask('ghost'), false);
    // P2-27③：暂停不存在的任务绝不能经 saveTask 的插入分支造出幻影行。
    assert.equal(getTask('ghost'), null, 'pausing a missing task must not insert a stub row');

    seedTask('p-running', 'running');
    seedTask('p-pending', 'pending');
    assert.equal(pauseTask('p-running'), true);
    assert.equal(pauseTask('p-pending'), true);
    assert.equal(getTask('p-running')?.status, 'paused');
    assert.equal(getTask('p-pending')?.status, 'paused');
    assert.equal(pauseTask('p-running'), true, 'pause is idempotent');

    seedTask('p-success', 'success');
    seedTask('p-cancelled', 'cancelled');
    assert.equal(pauseTask('p-success'), false);
    assert.equal(pauseTask('p-cancelled'), false);
    assert.equal(getTask('p-cancelled')?.status, 'cancelled');
  } finally {
    closeDb();
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('a queued task cancelled before dequeue never runs and stays cancelled', async () => {
  const root = freshDb();
  try {
    seedTask('q-early', 'pending');
    assert.equal(queueTask('q-early'), true);
    // 同步取消——出队微任务执行时 DB 已是 cancelled，tryStartTask 会拒绝，
    // 绝不进入 running，也不会触碰浏览器（P1-9）。
    assert.equal(cancelTask('q-early'), true);
    await drainQueue();
    assert.equal(getTask('q-early')?.status, 'cancelled');
    assert.equal(queueTask('q-early'), false, 'cancelled task must never be scheduled again');
  } finally {
    closeDb();
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('a queued task paused then resumed still honours a later cancel', async () => {
  const root = freshDb();
  try {
    seedTask('q-pause', 'pending');
    assert.equal(queueTask('q-pause'), true);
    assert.equal(pauseTask('q-pause'), true, 'queued task can be paused');
    // /start 对已入队但被暂停的任务执行恢复语义：翻回 pending，不重复入队。
    assert.equal(queueTask('q-pause'), true);
    assert.equal(getTask('q-pause')?.status, 'pending');
    assert.equal(cancelTask('q-pause'), true);
    await drainQueue();
    assert.equal(getTask('q-pause')?.status, 'cancelled');
  } finally {
    closeDb();
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('queueTask refuses missing and running tasks', () => {
  const root = freshDb();
  try {
    assert.equal(queueTask('ghost'), false, 'missing task must not be queued');
    seedTask('q-running', 'running');
    assert.equal(queueTask('q-running'), false);
    seedTask('q-failed', 'failed');
    assert.equal(markTaskQueueable('q-failed'), true, 'failed tasks may be requeued as pending');
    assert.equal(getTask('q-failed')?.status, 'pending');
    // 立即取消，防止出队时真正运行。
    assert.equal(cancelTask('q-failed'), true);
  } finally {
    closeDb();
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('runQueue tail observes failures instead of leaking unhandled rejections', () => {
  const here = path.dirname(fileURLToPath(import.meta.url));
  const scheduler = fs.readFileSync(path.join(here, '..', 'src', 'scheduler.ts'), 'utf8');
  const runQueueSection = scheduler.slice(scheduler.indexOf('runQueue = runQueue'));
  assert.ok(runQueueSection.includes('.catch('), 'queue tail must catch task rejections');
});
