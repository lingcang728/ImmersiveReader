import express from 'express';
import { getTasks, resetRunningTasks, getTask } from './db.js';
import { cancelTask, createTask, pauseTask, queueTask } from './scheduler.js';
import { logger } from './utils.js';
import { getLoginStatus } from './browser.js';
import { runLogin } from './login.js';
import { randomBytes } from 'crypto';
import { resolveSidecarPort, writeReady } from './sidecar-protocol.js';
import { hasBearerToken } from './auth.js';

// ── P1-7：进程级兜底 ──
// 任何漏网的 Promise 拒绝 / 同步异常只记日志，不许直接杀掉 sidecar；
// 否则一次抓取抖动会让宿主眼里整个引擎“崩退”，所有任务跟着挂。
process.on('unhandledRejection', (reason) => {
  const message = reason instanceof Error ? (reason.stack || reason.message) : String(reason);
  logger.error(`未处理的 Promise 拒绝（已拦截，进程保持运行）: ${message}`);
});
process.on('uncaughtException', (err) => {
  const message = err instanceof Error ? (err.stack || err.message) : String(err);
  logger.error(`未捕获异常（已拦截，进程保持运行）: ${message}`);
});

const app = express();
const HOST = '127.0.0.1';
const localToken = process.env.ZHIHU_PACKER_TOKEN || randomBytes(24).toString('hex');
const localHosts = new Set(['127.0.0.1', 'localhost', '::1']);

app.use(express.json());

function parseHost(hostHeader: string | undefined): string {
  return (hostHeader || '').split(':')[0].replace(/^\[|\]$/g, '').toLowerCase();
}

function isTrustedRequest(req: express.Request): boolean {
  const host = parseHost(req.headers.host);
  if (!localHosts.has(host)) return false;

  const origin = req.headers.origin;
  if (origin) {
    try {
      const originUrl = new URL(origin);
      if (!localHosts.has(originUrl.hostname.toLowerCase())) return false;
    } catch {
      return false;
    }
  }

  return true;
}

function hasValidToken(req: express.Request): boolean {
  return hasBearerToken(req.header('authorization'), localToken);
}

function requireLocalToken(req: express.Request, res: express.Response, next: express.NextFunction) {
  if (!isTrustedRequest(req)) {
    return res.status(403).json({ success: false, error: '拒绝非本机来源请求' });
  }
  if (!hasValidToken(req)) {
    return res.status(401).json({ success: false, error: '缺少或无效的本地控制令牌' });
  }
  next();
}

app.use('/api', requireLocalToken);

app.get('/health', (_req, res) => {
  res.json({ engine: 'zhihu', status: 'ok' });
});

app.get('/api/status', requireLocalToken, (_req, res) => {
  res.json({ engine: 'zhihu', status: 'ready' });
});

app.get('/api/login-status', requireLocalToken, async (_req, res) => {
  try {
    const status = await getLoginStatus();
    res.json({ success: true, data: status });
  } catch (e: any) {
    res.status(503).json({ success: false, error: e.message });
  }
});

let loginPromise: Promise<void> | null = null;

app.post('/api/login/start', requireLocalToken, (_req, res) => {
  if (loginPromise) {
    return res.json({ success: true, started: false, message: '登录流程已经在运行。' });
  }
  const currentLogin = runLogin();
  loginPromise = currentLogin;
  // 显式观察拒绝：登录流程任何逃逸的错误（含 finally 中的关闭异常）都已被
  // closeBrowserContext 内部消化或在此落日志，绝不成为 unhandledRejection（P1-7）。
  currentLogin
    .catch((err) => {
      logger.error(`登录流程异常结束: ${err?.message || err}`);
    })
    .finally(() => {
      if (loginPromise === currentLogin) {
        loginPromise = null;
      }
    });
  res.json({ success: true, started: true });
});

// API：获取任务列表
app.get('/api/tasks', requireLocalToken, (_req, res) => {
  try {
    const tasks = getTasks();
    res.json({ success: true, data: tasks });
  } catch (e: any) {
    res.status(500).json({ success: false, error: e.message });
  }
});

// API：创建任务
app.post('/api/tasks', requireLocalToken, async (req, res) => {
  const { peopleId, itemTypes, topN, sortBy } = req.body;
  if (typeof peopleId !== 'string' || !/^[a-zA-Z0-9_-]{1,80}$/.test(peopleId)) {
    return res.status(400).json({ success: false, error: 'peopleId 只能包含字母、数字、下划线和短横线，长度 1-80' });
  }
  if (itemTypes && !['answers', 'articles', 'all'].includes(itemTypes)) {
    return res.status(400).json({ success: false, error: 'itemTypes 必须是 answers、articles 或 all' });
  }
  if (sortBy && !['time', 'vote'].includes(sortBy)) {
    return res.status(400).json({ success: false, error: 'sortBy 必须是 time 或 vote' });
  }
  const parsedTopN = topN === undefined || topN === null || topN === '' ? null : Number(topN);
  if (parsedTopN !== null && (!Number.isInteger(parsedTopN) || parsedTopN <= 0 || parsedTopN > 5000)) {
    return res.status(400).json({ success: false, error: 'topN 必须为空或 1-5000 的正整数' });
  }
  try {
    const taskId = await createTask(peopleId, itemTypes || 'all', {
      topN: parsedTopN,
      sortBy: sortBy || 'time'
    });
    res.json({ success: true, taskId });
  } catch (e: any) {
    res.status(500).json({ success: false, error: e.message });
  }
});

app.get('/api/tasks/:id', requireLocalToken, (req, res) => {
  const task = getTask(String(req.params.id));
  if (!task) return res.status(404).json({ success: false, error: '任务不存在' });
  res.json({ success: true, data: task });
});

// API：启动/恢复任务
app.post('/api/tasks/:id/start', requireLocalToken, async (req, res) => {
  const taskId = String(req.params.id);
  try {
    // 入队前校验：不存在的任务直接 404（P1-7），终态任务（cancelled/success）
    // 409——它们永不再被调度（P1-9）。paused/failed/partial_success 走恢复语义。
    const task = getTask(taskId);
    if (!task) {
      return res.status(404).json({ success: false, error: '任务不存在' });
    }
    if (task.status === 'cancelled' || task.status === 'success') {
      return res.status(409).json({ success: false, error: `任务已是终态(${task.status})，不可再启动` });
    }
    const queued = queueTask(taskId);
    res.json({ success: true, queued, message: queued ? 'Task queued' : 'Task already queued or running' });
  } catch (e: any) {
    res.status(500).json({ success: false, error: e.message });
  }
});

// API: 暂停任务
app.post('/api/tasks/:id/pause', requireLocalToken, async (req, res) => {
  const taskId = String(req.params.id);
  try {
    // 走调度器 pauseTask：统一语义，不存在的任务不再被 saveTask 误插成行（P1-9）。
    if (!getTask(taskId)) {
      return res.status(404).json({ success: false, error: '任务不存在' });
    }
    if (!pauseTask(taskId)) {
      return res.status(409).json({ success: false, error: '任务已结束，无法暂停' });
    }
    res.json({ success: true, message: 'Pause signal sent' });
  } catch (e: any) {
    res.status(500).json({ success: false, error: e.message });
  }
});

app.post('/api/tasks/:id/cancel', requireLocalToken, (req, res) => {
  const taskId = String(req.params.id);
  try {
    if (!getTask(taskId)) {
      return res.status(404).json({ success: false, error: '任务不存在' });
    }
    const cancelled = cancelTask(taskId);
    if (!cancelled) return res.status(409).json({ success: false, error: '任务无法取消' });
    res.json({ success: true, message: 'Cancel signal sent' });
  } catch (e: any) {
    res.status(500).json({ success: false, error: e.message });
  }
});

export function startServer(port = 3000) {
  const server = app.listen(port, HOST, () => {
    const address = server.address();
    if (!address || typeof address === 'string') {
      logger.error('无法读取 sidecar 动态端口。');
      return;
    }
    writeReady('zhihu', process.pid, address.port);
    try {
      resetRunningTasks();
    } catch (err: any) {
      logger.error(`启动时重置残留任务状态失败: ${err.message}`);
    }
    logger.info(`知乎 sidecar 已在 ${HOST}:${address.port} 启动。`);
    logger.info('本地控制令牌已启用，仅通过桌面应用内存传递。');
  });
  return server;
}

import { fileURLToPath } from 'url';
const isMain = process.argv[1] && (
  process.argv[1] === fileURLToPath(import.meta.url) || 
  process.argv[1].endsWith('server.ts') || 
  process.argv[1].endsWith('server.js')
);
if (isMain) {
  startServer(resolveSidecarPort());
}
