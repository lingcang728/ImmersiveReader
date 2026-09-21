import express from 'express';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { randomBytes, timingSafeEqual } from 'crypto';
import { resolveArchiveOutputDir } from './runtime-paths.js';

const app = express();
// P3-28：不暴露 Express 指纹（与 sidecar server.ts 一致）。
app.disable('x-powered-by');
const port = Number(process.env.ZHIHU_READER_PORT) || 3080;
// P3-28：默认只绑回环地址 —— output/ 里是私人归档内容，绑 0.0.0.0 会把整个
// 归档暴露给局域网。确需局域网预览时显式传 --lan 或 ZHIHU_READER_HOST=0.0.0.0。
const host = process.env.ZHIHU_READER_HOST || (process.argv.includes('--lan') ? '0.0.0.0' : '127.0.0.1');
const isLoopbackOnly = ['127.0.0.1', 'localhost', '::1'].includes(host);

// 06-F-08：非回环监听时要求一次性随机 token —— 仅靠 "host 检查+警告" 意味着
// 局域网里任何设备都能拉走全部归档。用户从终端打印的带 ?token= URL 进入，
// 首个合法请求种下 HttpOnly Cookie，后续静态子资源（css/js/img）自动携带。
const accessToken = isLoopbackOnly ? '' : randomBytes(16).toString('hex');

function tokenMatches(provided: string): boolean {
  if (!accessToken || !provided) return false;
  const a = Buffer.from(provided);
  const b = Buffer.from(accessToken);
  return a.length === b.length && timingSafeEqual(a, b);
}

app.use((req, res, next) => {
  if (!accessToken) {
    next();
    return;
  }
  const cookieOk = (req.headers.cookie || '')
    .split(';')
    .some((part) => {
      const trimmed = part.trim();
      const eq = trimmed.indexOf('=');
      return eq > 0 && trimmed.slice(0, eq) === 'reader_token' && tokenMatches(trimmed.slice(eq + 1));
    });
  const queryOk = tokenMatches(String(req.query.token || ''));
  if (!cookieOk && !queryOk) {
    res.status(403).end('Forbidden');
    return;
  }
  if (queryOk && !cookieOk) {
    res.setHeader('Set-Cookie', `reader_token=${accessToken}; HttpOnly; SameSite=Strict; Path=/`);
  }
  next();
});
// P3-5：与生产归档目录解析保持一致（IMMERSIVE_ZHIHU_OUTPUT /
// IMMERSIVE_LIBRARY_ROOT\知乎 / 兜底 output），而不是死磕 cwd\output。
const outputDir = resolveArchiveOutputDir({ cwd: process.cwd(), environment: process.env });

// 13-F9：serve-static/send 用 fs.stat，symlink 永远被跟随（没有 follow:false
// 这个选项）——--lan 下 output/ 里一个指向外面的软链就会把归档外文件暴露
// 给局域网。逐请求 realpath 一次：落点在真实 output 目录之外的一律 403。
const outputDirReal = fs.realpathSync(outputDir);
app.use((req, res, next) => {
  let decoded: string;
  try {
    decoded = decodeURIComponent(req.path);
  } catch {
    res.status(400).end();
    return;
  }
  fs.realpath(path.join(outputDir, decoded), (err, real) => {
    if (err || real === outputDirReal || real.startsWith(outputDirReal + path.sep)) {
      next();
    } else {
      res.status(403).end();
    }
  });
});

// 静态托管 output 目录下的文件
app.use(express.static(outputDir));

console.log('📂 正在静态托管目录:', outputDir);

// 绑 0.0.0.0 时打印首个非内部 IPv4，用户从其他设备直接访问这个地址。
function lanDisplayHost(): string {
  if (host !== '0.0.0.0') return host;
  for (const infos of Object.values(os.networkInterfaces())) {
    for (const info of infos || []) {
      if (info.family === 'IPv4' && !info.internal) return info.address;
    }
  }
  return 'localhost';
}

app.listen(port, host, () => {
  const displayHost = isLoopbackOnly ? host : lanDisplayHost();
  const tokenSuffix = accessToken ? `?token=${accessToken}` : '';
  console.log('\n======================================================');
  console.log('🚀 沉浸式 Markdown 阅读器本地托管服务启动成功！');
  console.log('======================================================');
  console.log(`👉 通用独立阅读器：http://${displayHost}:${port}/universal-reader.html${tokenSuffix}`);
  console.log(`👉 答主归档阅读器：http://${displayHost}:${port}/reader.html${tokenSuffix}`);
  if (isLoopbackOnly) {
    console.log('ℹ️  仅监听本机回环地址；如需局域网访问请加 --lan 或设置 ZHIHU_READER_HOST。');
  } else {
    console.warn(`⚠️  正在监听 ${host} —— 局域网内设备需使用上方带 token 的完整链接才可访问归档内容！`);
  }
  console.log('======================================================\n');
});
export {};
