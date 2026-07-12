import express from 'express';
import { getTasks, saveTask, deleteTask, clearCompletedTasks, resetRunningTasks, getTask, resetTaskForce } from './db.js';
import { createTask, queueTask, setSchedulerProgressCallback } from './scheduler.js';
import { logger, sanitizeFilename } from './utils.js';
import * as path from 'path';
import * as fs from 'fs';
import { exec } from 'child_process';
import { randomBytes } from 'crypto';
import { resolveArchiveOutputDir } from './runtime-paths.js';
import { resolveSidecarPort, writeReady } from './sidecar-protocol.js';
import { hasBearerToken } from './auth.js';

const app = express();
const HOST = '127.0.0.1';
const localToken = process.env.ZHIHU_PACKER_TOKEN || randomBytes(24).toString('hex');
const localHosts = new Set(['127.0.0.1', 'localhost', '::1']);

app.use(express.json());

app.get('/health', (_req, res) => {
  res.json({ engine: 'zhihu', status: 'ok' });
});

app.get('/api/status', requireLocalToken, (_req, res) => {
  res.json({ engine: 'zhihu', status: 'ready' });
});

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

// 静态文件托管
app.use(express.static(path.resolve(process.cwd(), 'public')));

app.get('/api/config', requireLocalToken, (_req, res) => {
  res.json({ success: true, token: localToken });
});

let clients: any[] = [];

// SSE (Server-Sent Events) 端点
app.get('/api/events', requireLocalToken, (req, res) => {
  res.setHeader('Content-Type', 'text/event-stream');
  res.setHeader('Cache-Control', 'no-cache');
  res.setHeader('Connection', 'keep-alive');
  res.flushHeaders();

  const clientId = Date.now();
  const newClient = {
    id: clientId,
    res
  };
  clients.push(newClient);

  req.on('close', () => {
    clients = clients.filter(c => c.id !== clientId);
  });
});

// 向所有客户端推送事件
function broadcast(event: any) {
  clients.forEach(c => {
    c.res.write(`data: ${JSON.stringify(event)}\n\n`);
  });
}

// 注册调度器的进度回调
setSchedulerProgressCallback((taskId, status, message) => {
  broadcast({
    type: 'progress',
    taskId,
    status,
    message,
    timestamp: Date.now()
  });
});

// API：获取任务列表
app.get('/api/tasks', requireLocalToken, (req, res) => {
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

// API: 强制一键重爬所有已有的作者
app.post('/api/tasks/force-restart-existing', requireLocalToken, async (req, res) => {
  try {
    const outputDir = resolveArchiveOutputDir({ cwd: process.cwd(), environment: process.env });
    if (!fs.existsSync(outputDir)) {
      return res.json({ success: true, count: 0, message: 'output 目录不存在，无需重跑' });
    }

    const folders = fs.readdirSync(outputDir).filter(f => {
      const fullPath = path.join(outputDir, f);
      return fs.statSync(fullPath).isDirectory();
    });

    const tasks = getTasks();
    let restartedCount = 0;

    for (const folder of folders) {
      // 模糊匹配数据库任务，找到该作者的任务
      const matchedTasks = tasks.filter(t => {
        const sanitizedAuthor = sanitizeFilename(t.author_name || '', t.author_id || '').replace(/_[^_]+$/, '');
        return sanitizedAuthor === folder || t.author_name === folder || t.author_id === folder;
      });

      for (const t of matchedTasks) {
        logger.info(`【一键重爬】正在重置并重跑任务: ${t.id} (作者: ${folder})`);
        
        // 1. 强制重置数据库状态
        resetTaskForce(t.id);

        queueTask(t.id);

        restartedCount++;
      }
    }

    res.json({ success: true, count: restartedCount, message: `已成功强制重爬 ${restartedCount} 个作者的归档任务` });
  } catch (e: any) {
    res.status(500).json({ success: false, error: e.message });
  }
});

// API: 重跑所有含失败条目的任务（登录态过期中断后，重新 login 即可一键断点续跑）
app.post('/api/tasks/retry-failed', requireLocalToken, (req, res) => {
  try {
    const candidates = getTasks().filter(
      t => t.status !== 'running' && (t.status === 'failed' || t.status === 'partial_success' || t.failed_count > 0)
    );
    let queued = 0;
    for (const t of candidates) {
      if (queueTask(t.id)) queued++;
    }
    res.json({
      success: true,
      count: queued,
      message: queued > 0
        ? `已将 ${queued} 个任务重新加入队列，将从失败条目断点续跑`
        : '没有需要重跑的失败任务'
    });
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

// API: 清理所有已结束的任务历史 (含成功/失败/暂停)
app.delete('/api/tasks/clear-completed', requireLocalToken, (req, res) => {
  try {
    const count = clearCompletedTasks();
    res.json({ success: true, count, message: `成功清理了 ${count} 个任务历史` });
  } catch (e: any) {
    res.status(500).json({ success: false, error: e.message });
  }
});

// API: 删除单个任务
app.delete('/api/tasks/:id', requireLocalToken, (req, res) => {
  const taskId = String(req.params.id);
  try {
    deleteTask(taskId);
    res.json({ success: true, message: `已成功删除任务 ${taskId}` });
  } catch (e: any) {
    res.status(500).json({ success: false, error: e.message });
  }
});

// API: 打开本地输出目录
app.post('/api/open-dir', requireLocalToken, (req, res) => {
  try {
    const outputDir = resolveArchiveOutputDir({ cwd: process.cwd(), environment: process.env });
    if (!fs.existsSync(outputDir)) {
      fs.mkdirSync(outputDir, { recursive: true });
    }
    
    let cmd = '';
    if (process.platform === 'win32') {
      cmd = `cmd /c start "" "${outputDir}"`;
    } else if (process.platform === 'darwin') {
      cmd = `open "${outputDir}"`;
    } else {
      cmd = `xdg-open "${outputDir}"`;
    }

    exec(cmd, (err) => {
      if (err) {
        logger.error(`无法打开输出目录: ${err.message}`);
        return res.status(500).json({ success: false, error: `无法打开目录: ${err.message}` });
      }
      res.json({ success: true });
    });
  } catch (e: any) {
    res.status(500).json({ success: false, error: e.message });
  }
});

// API: 打包下载 ZIP 归档
app.get('/api/download', requireLocalToken, async (req, res) => {
  try {
    const outputDir = resolveArchiveOutputDir({ cwd: process.cwd(), environment: process.env });
    if (!fs.existsSync(outputDir)) {
      return res.status(404).json({ success: false, error: '当前没有任何备份输出，请先开始备份任务' });
    }

    const files = fs.readdirSync(outputDir);
    if (files.length === 0) {
      return res.status(400).json({ success: false, error: '输出目录为空，无任何归档内容可供下载' });
    }

    const tempZip = path.resolve(process.cwd(), `zhihu-archive-${Date.now()}-${randomBytes(4).toString('hex')}.zip`);
    if (fs.existsSync(tempZip)) {
      try { fs.unlinkSync(tempZip); } catch (e) {}
    }

    let cmd = '';
    if (process.platform === 'win32') {
      cmd = `powershell -Command "Compress-Archive -Path '${outputDir}\\*' -DestinationPath '${tempZip}' -Force"`;
    } else {
      cmd = `zip -r "${tempZip}" .`;
    }

    logger.info(`正在生成归档压缩包: ${tempZip}`);
    exec(cmd, { cwd: outputDir }, (err) => {
      if (err) {
        logger.error(`生成压缩包出错: ${err.message}`);
        return res.status(500).json({ success: false, error: `生成压缩包出错: ${err.message}` });
      }

      if (!fs.existsSync(tempZip)) {
        return res.status(500).json({ success: false, error: '生成压缩包失败，文件未找到' });
      }

      logger.info(`压缩包已就绪，开始传输下载...`);
      res.download(tempZip, 'zhihu-archive.zip', (err2) => {
        if (err2) {
          logger.error(`下载文件传输失败: ${err2.message}`);
        }
        try { fs.unlinkSync(tempZip); } catch (e) {}
      });
    });
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
    logger.info(`Web 控制台已成功启动。控制面板: http://${HOST}:${address.port}`);
    logger.info(`本地控制令牌已启用。令牌不会写入前端静态文件，只通过同源 /api/config 获取。`);
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
