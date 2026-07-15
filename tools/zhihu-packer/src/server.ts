import express from 'express';
import { getTasks, saveTask, resetRunningTasks, getTask } from './db.js';
import { cancelTask, createTask, queueTask } from './scheduler.js';
import { logger } from './utils.js';
import { getLoginStatus } from './browser.js';
import { runLogin } from './login.js';
import { randomBytes } from 'crypto';
import { resolveSidecarPort, writeReady } from './sidecar-protocol.js';
import { hasBearerToken } from './auth.js';

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
  loginPromise = runLogin().finally(() => {
    loginPromise = null;
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
    saveTask({ id: taskId, status: 'paused' });
    res.json({ success: true, message: 'Pause signal sent' });
  } catch (e: any) {
    res.status(500).json({ success: false, error: e.message });
  }
});

app.post('/api/tasks/:id/cancel', requireLocalToken, (req, res) => {
  const taskId = String(req.params.id);
  try {
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
