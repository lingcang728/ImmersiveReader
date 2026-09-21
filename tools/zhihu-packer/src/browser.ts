import { chromium, Browser, BrowserContext, Cookie } from 'playwright-core';
import { spawn, ChildProcess } from 'child_process';
import * as path from 'path';
import * as fs from 'fs';
import * as net from 'net';
import { logger, redactPathForLog } from './utils.js';
import { resolveBrowserCacheDir, resolveBrowserExecutable, resolveProfileDir } from './runtime-paths.js';

let activeContext: BrowserContext | null = null;
let activeBrowser: Browser | null = null;
let activeObscuraProcess: ChildProcess | null = null;
let obscuraSpawnError: string | null = null;
let currentHeadlessMode: boolean | null = null;
let currentBackend: 'obscura' | 'playwright' | null = null;

/**
 * 浏览器生命周期串行锁（promise mutex）。
 * getBrowserContext / closeBrowserContext / getLoginStatus 的 check-then-act
 * 曾经没有互斥：两个并发 launch 会同时争抢同一 profile 目录必炸一方；
 * login-status 查询还会把进行中的有头登录/验证码窗口直接 close 掉（P1-8）。
 * 所有生命周期操作一律经此锁串行化。
 */
let browserLifecycleQueue: Promise<unknown> = Promise.resolve();

export function withBrowserLock<T>(fn: () => Promise<T>): Promise<T> {
  const run = browserLifecycleQueue.then(fn, fn);
  browserLifecycleQueue = run.then(() => undefined, () => undefined);
  return run;
}

let obscuraPort = process.env.OBSCURA_PORT ? Number(process.env.OBSCURA_PORT) : 0;
const chromeProfileDir = resolveProfileDir({ cwd: process.cwd(), environment: process.env });
const browserCacheDir = resolveBrowserCacheDir({ cwd: process.cwd(), environment: process.env });
const obscuraStorageDir = path.join(chromeProfileDir, '.obscura-profile');
// 兜底 UA（探测失败时使用）。正常路径下 UA 会与本机浏览器真实版本对齐，见 resolveUserAgent。
const fallbackUserAgent = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36';
let cachedUserAgent: string | null = null;

/**
 * 探测本机 Chrome/Edge 真实版本并生成匹配的 UA。
 * 写死旧版本号（如 Chrome/120）会随时间推移与真实浏览器指纹脱节，更易被反爬识别。
 */
async function resolveUserAgent(): Promise<string> {
  if (cachedUserAgent) return cachedUserAgent;
  for (const channel of ['chrome', 'msedge']) {
    try {
      const probe = await chromium.launch({ channel, headless: true });
      const version = probe.version();
      await probe.close();
      const major = version.split('.')[0];
      if (major && Number(major) > 0) {
        cachedUserAgent = `Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/${major}.0.0.0 Safari/537.36`;
        logger.info(`已探测本机浏览器版本 ${version}（channel: ${channel}），UA 对齐为 Chrome/${major}`);
        return cachedUserAgent;
      }
    } catch {
      // 该 channel 不存在，尝试下一个
    }
  }
  logger.warn('未能探测本机浏览器版本，使用兜底 UA（Chrome/120）。');
  cachedUserAgent = fallbackUserAgent;
  return cachedUserAgent;
}

function getCdpEndpoint() {
  return `ws://127.0.0.1:${obscuraPort}`;
}

function shouldUseObscura(headless: boolean) {
  // 且实测能通过知乎 zse-ck 反爬质询；而无头 Obscura 会被 zse-ck 反复质询、拿不到内容。
  // 仅当显式设置 ZHIHU_PACKER_BROWSER=obscura 时才使用 Obscura 后端。
  return headless && process.env.ZHIHU_PACKER_BROWSER === 'obscura';
}

type BrowserLaunchTarget =
  | { executablePath: string }
  | { channel: 'chrome' | 'msedge' };

export function browserLaunchTargets(
  headless: boolean,
  environment: Readonly<Record<string, string | undefined>>
): BrowserLaunchTarget[] {
  const managedExecutable = headless ? resolveBrowserExecutable(environment) : undefined;
  if (managedExecutable) {
    return [{ executablePath: managedExecutable }];
  }
  return [{ channel: 'chrome' }, { channel: 'msedge' }];
}

function findObscuraExecutable(): string | null {
  const candidates: string[] = [];

  if (process.env.OBSCURA_BIN) {
    candidates.push(process.env.OBSCURA_BIN);
  }

  if (process.env.LOCALAPPDATA) {
    const installRoot = path.join(process.env.LOCALAPPDATA, 'Programs', 'obscura');
    if (fs.existsSync(installRoot)) {
      for (const version of fs.readdirSync(installRoot).sort().reverse()) {
        candidates.push(path.join(installRoot, version, 'obscura.exe'));
      }
    }
    candidates.push(path.join(process.env.LOCALAPPDATA, 'Microsoft', 'WindowsApps', 'obscura.exe'));
  }

  candidates.push('obscura.exe');

  for (const candidate of candidates) {
    if (candidate === 'obscura.exe' || fs.existsSync(candidate)) {
      return candidate;
    }
  }

  return null;
}

async function isObscuraReady(): Promise<boolean> {
  if (!obscuraPort) {
    return false;
  }

  try {
    const response = await fetch(`http://127.0.0.1:${obscuraPort}/json/version`);
    return response.ok;
  } catch {
    return false;
  }
}

function isPortInUse(port: number): Promise<boolean> {
  return new Promise(resolve => {
    const socket = net.createConnection(port, '127.0.0.1');
    socket.once('connect', () => {
      socket.destroy();
      resolve(true);
    });
    socket.once('error', () => {
      socket.destroy();
      resolve(false);
    });
    socket.setTimeout(500, () => {
      socket.destroy();
      resolve(true);
    });
  });
}

async function ensureObscuraPort(): Promise<void> {
  if (obscuraPort) {
    return;
  }

  // 06-F-02：CDP 端口本身无鉴权是固有风险，固定的 9230-9330 扫描区间让它
  // 能被同机进程预测命中。优先随机高位端口（取在 Windows 动态端口段之下，
  // 避开出站连接的临时源端口），多次随机尝试都被占用再退回固定区间兜底。
  for (let attempt = 0; attempt < 32; attempt++) {
    const candidate = 20000 + Math.floor(Math.random() * 28000);
    if (!(await isPortInUse(candidate))) {
      obscuraPort = candidate;
      return;
    }
  }
  for (let port = 9230; port < 9330; port++) {
    if (!(await isPortInUse(port))) {
      obscuraPort = port;
      return;
    }
  }

  throw new Error('未能为 Obscura 找到可用端口，请设置 OBSCURA_PORT。');
}

async function waitForObscuraReady(timeoutMs = 10000): Promise<void> {
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeoutMs) {
    // spawn 失败/进程早退时快速失败，不再空等整个超时窗口（P1-7）。
    if (obscuraSpawnError) {
      throw new Error(`Obscura 进程启动失败: ${obscuraSpawnError}`);
    }
    if (!activeObscuraProcess) {
      throw new Error('Obscura 进程在等待 CDP 就绪期间退出。');
    }
    if (await isObscuraReady()) {
      return;
    }
    await new Promise(resolve => setTimeout(resolve, 250));
  }
  throw new Error(`Obscura CDP 服务未能在 ${timeoutMs}ms 内启动。`);
}

async function startObscuraServer(): Promise<void> {
  await ensureObscuraPort();

  if (await isObscuraReady()) {
    return;
  }

  const executable = findObscuraExecutable();
  if (!executable) {
    throw new Error('未找到 Obscura。请先安装 obscura.exe，或设置 OBSCURA_BIN 指向它。');
  }

  fs.mkdirSync(obscuraStorageDir, { recursive: true });

  const args = [
    'serve',
    '--host', '127.0.0.1',
    '--port', String(obscuraPort),
    '--storage-dir', obscuraStorageDir,
    '--user-agent', await resolveUserAgent(),
    '--allow-file-access',
    '--allow-private-network'
  ];

  if (process.env.OBSCURA_STEALTH === '1' || process.env.OBSCURA_STEALTH === 'true') {
    args.push('--stealth');
  }

  obscuraSpawnError = null;
  activeObscuraProcess = spawn(executable, args, {
    stdio: 'ignore',
    windowsHide: true
  });

  // spawn 失败（如 ENOENT）会异步触发 'error' 事件；没有监听器的 'error'
  // 事件会成为 uncaughtException 杀掉整个 sidecar（P1-7）。
  activeObscuraProcess.on('error', (err) => {
    obscuraSpawnError = err?.message || String(err);
    logger.error(`Obscura 进程错误: ${obscuraSpawnError}`);
    activeObscuraProcess = null;
  });

  activeObscuraProcess.on('exit', () => {
    activeObscuraProcess = null;
  });

  try {
    await waitForObscuraReady();
  } catch (err) {
    // CDP 就绪失败/超时时回收子进程，避免遗留孤儿浏览器进程（P1-7）。
    if (activeObscuraProcess) {
      activeObscuraProcess.kill();
      activeObscuraProcess = null;
    }
    throw err;
  }
  logger.info(`Obscura CDP 服务已启动: ${getCdpEndpoint()}`);
}

async function createObscuraContext(): Promise<BrowserContext> {
  await startObscuraServer();
  activeBrowser = await chromium.connectOverCDP(getCdpEndpoint());
  activeContext = activeBrowser.contexts()[0] || await activeBrowser.newContext();
  currentBackend = 'obscura';
  return activeContext;
}

function toObscuraCookie(cookie: Cookie) {
  return {
    name: cookie.name,
    value: cookie.value,
    path: cookie.path || '/',
    domain: cookie.domain.replace(/^\./, '').toLowerCase(),
    secure: cookie.secure,
    http_only: cookie.httpOnly,
    expires: cookie.expires > 0 ? Math.floor(cookie.expires) : null,
    same_site: cookie.sameSite || 'Lax'
  };
}

// P2-1：cookies.json 曾是登录凭据的明文第二副本（Chromium profile 内的那份
// 有 Chromium 自己的 DPAPI，这面 mirror 没有）。改为经 DPAPI
// CryptProtectData(CurrentUser) 加密落盘为 cookies.dpapi；cookies.json 仅作
// 旧版本遗留读取，一旦读到即迁移删除。
const COOKIE_MIRROR_FILE = 'cookies.json';
const COOKIE_PROTECTED_FILE = 'cookies.dpapi';

// 由 PowerShell 调 CryptProtectData/CryptUnprotectData（DataProtectionScope::
// CurrentUser）——Node 没有可用的 DPAPI 绑定，而该文件读写只在登录同步/任务
// 注入时发生，单次 ~200ms 可接受。文件路径经环境变量传入，避免把凭据明文放进
// 命令行参数。
const DPAPI_SCRIPT = [
  'Add-Type -AssemblyName System.Security',
  '$scope=[System.Security.Cryptography.DataProtectionScope]::CurrentUser',
  '$in=[System.IO.File]::ReadAllBytes($env:DPAPI_IN)',
  'if ($env:DPAPI_MODE -eq "protect")',
  '{ $out=[System.Security.Cryptography.ProtectedData]::Protect($in,$null,$scope) }',
  'else',
  '{ $out=[System.Security.Cryptography.ProtectedData]::Unprotect($in,$null,$scope) }',
  '[System.IO.File]::WriteAllBytes($env:DPAPI_OUT,$out)'
].join('; ');

function dpapiTransformFile(mode: 'protect' | 'unprotect', inputPath: string, outputPath: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const ps = spawn('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', DPAPI_SCRIPT], {
      env: { ...process.env, DPAPI_MODE: mode, DPAPI_IN: inputPath, DPAPI_OUT: outputPath },
      stdio: ['ignore', 'ignore', 'pipe'],
      windowsHide: true
    });
    let stderr = '';
    ps.stderr?.on('data', (d) => { stderr += d; });
    const timer = setTimeout(() => {
      ps.kill();
      reject(new Error('DPAPI 调用超时'));
    }, 20_000);
    ps.on('error', (e) => { clearTimeout(timer); reject(e); });
    ps.on('exit', (code) => {
      clearTimeout(timer);
      if (code === 0 && fs.existsSync(outputPath)) resolve();
      else reject(new Error(`DPAPI ${mode} 退出码 ${code}: ${stderr.slice(0, 200)}`));
    });
  });
}

/**
 * 读取持久化 Cookie：优先 DPAPI 密文，其次旧明文 cookies.json（读到后顺手
 * 迁移成密文并删掉明文）。返回 null 表示无可用凭据。
 */
async function readStoredCookies(): Promise<any[] | null> {
  const protectedFile = path.join(obscuraStorageDir, COOKIE_PROTECTED_FILE);
  const legacyFile = path.join(obscuraStorageDir, COOKIE_MIRROR_FILE);
  if (fs.existsSync(protectedFile)) {
    const tempPlain = `${protectedFile}.read-${process.pid}`;
    try {
      await dpapiTransformFile('unprotect', protectedFile, tempPlain);
      const parsed = JSON.parse(fs.readFileSync(tempPlain, 'utf-8'));
      if (Array.isArray(parsed) && parsed.length > 0) return parsed;
      return null;
    } catch (e: any) {
      logger.warn(`解密登录凭据失败（按未登录处理）: ${e?.message || e}`);
      return null;
    } finally {
      try { fs.unlinkSync(tempPlain); } catch { /* 临时文件清理失败无害 */ }
    }
  }
  if (!fs.existsSync(legacyFile)) return null;
  try {
    const parsed = JSON.parse(fs.readFileSync(legacyFile, 'utf-8'));
    if (!Array.isArray(parsed) || parsed.length === 0) return null;
    // 旧明文还在 → 立刻迁移为密文，尽量缩短明文驻留窗口。
    const tempPlain = `${protectedFile}.tmp-${process.pid}`;
    const tempProt = `${protectedFile}.enc-${process.pid}`;
    try {
      fs.writeFileSync(tempPlain, JSON.stringify(parsed), 'utf-8');
      await dpapiTransformFile('protect', tempPlain, tempProt);
      fs.renameSync(tempProt, protectedFile);
      fs.unlinkSync(tempPlain);
      fs.unlinkSync(legacyFile);
      logger.info('已将旧明文 cookies.json 迁移为 DPAPI 加密存储。');
    } catch {
      try { fs.unlinkSync(tempPlain); } catch { /* ignore */ }
      try { fs.unlinkSync(tempProt); } catch { /* ignore */ }
    }
    return parsed;
  } catch (e: any) {
    logger.warn(`解析登录凭据文件失败: ${e?.message || e}`);
    return null;
  }
}

export async function syncCookiesToObscuraStorage(context: BrowserContext): Promise<void> {
  const cookies = await context.cookies();
  fs.mkdirSync(obscuraStorageDir, { recursive: true });
  // P3-28：写入必须原子化（tmp+rename），否则进程在 writeFileSync 中途被杀会
  // 留下半截文件，下次注入解析失败即丢登录态。P2-1：密文优先——先把明文写到
  // 临时文件，DPAPI 加密后原子替换 cookies.dpapi 并删除明文临时文件与旧
  // cookies.json；DPAPI 不可用（非 Windows 调试等）才退回明文 mirror。
  const protectedFile = path.join(obscuraStorageDir, COOKIE_PROTECTED_FILE);
  const legacyFile = path.join(obscuraStorageDir, COOKIE_MIRROR_FILE);
  const tempPlain = `${protectedFile}.tmp-${process.pid}`;
  const tempProt = `${protectedFile}.enc-${process.pid}`;
  fs.writeFileSync(tempPlain, JSON.stringify(cookies.map(toObscuraCookie), null, 2), 'utf-8');
  try {
    await dpapiTransformFile('protect', tempPlain, tempProt);
    fs.renameSync(tempProt, protectedFile);
    try { fs.unlinkSync(tempPlain); } catch { /* ignore */ }
    try { fs.unlinkSync(legacyFile); } catch { /* ignore */ }
    logger.info(`已同步 ${cookies.length} 个 Cookie（DPAPI 加密）到 ${redactPathForLog(protectedFile)}`);
    return;
  } catch (e: any) {
    try { fs.unlinkSync(tempProt); } catch { /* ignore */ }
    logger.warn(`DPAPI 加密不可用，回退明文 Cookie 存储: ${e?.message || e}`);
  }
  fs.renameSync(tempPlain, legacyFile);
  try {
    fs.chmodSync(legacyFile, 0o600);
  } catch {
    // 权限收紧失败不影响凭据本身可用性。
  }
  logger.info(`已同步 ${cookies.length} 个 Cookie 到 Obscura 存储目录: ${redactPathForLog(legacyFile)}`);
}

/**
 * 把桌面登录流程同步的 .obscura-profile/cookies.json 显式注入到 Obscura Context。
 *
 * Obscura 通过 CDP 连接，并不会自动读取 storage-dir 里的 cookies.json，
 * 因此必须在连接后用 Playwright 的 addCookies 主动注入，否则无头任务始终是未登录态。
 * 同时还原被 toObscuraCookie 去掉的前导点，使 z_c0 等域 Cookie 对 www / zhuanlan 子域同时生效。
 */
async function injectStoredCookies(context: BrowserContext): Promise<void> {
  const parsed = await readStoredCookies();
  if (!parsed) {
    const file = path.join(obscuraStorageDir, COOKIE_PROTECTED_FILE);
    logger.warn(`未找到可用登录凭据 (${redactPathForLog(file)})，Obscura 将以未登录态运行。请在沉浸阅读的知乎获取面板登录。`);
    return;
  }

  const mapSameSite = (s: any): 'Strict' | 'Lax' | 'None' => {
    const v = String(s || '').toLowerCase();
    if (v === 'strict') return 'Strict';
    if (v === 'none') return 'None';
    return 'Lax';
  };

  const nowSec = Math.floor(Date.now() / 1000);
  const cookies = parsed
    // __zse_ck 是知乎反爬的易失性质询令牌，注入旧值反而会与现场质询冲突，交由实时质询生成。
    // expires 已过的 Cookie 一并剔除——注入死凭据没有意义，也让旧 mirror 里
    // 永驻的过期条目随下次 sync 被清掉。
    .filter((c: any) => c && c.name && c.domain && c.name !== '__zse_ck'
      && (typeof c.expires !== 'number' || !Number.isFinite(c.expires) || c.expires <= 0 || c.expires > nowSec))
    .map((c: any) => {
      const domain = String(c.domain).startsWith('.') ? String(c.domain) : `.${c.domain}`;
      const cookie: any = {
        name: String(c.name),
        value: String(c.value ?? ''),
        domain,
        path: c.path || '/',
        httpOnly: !!c.http_only,
        secure: !!c.secure,
        sameSite: mapSameSite(c.same_site)
      };
      if (typeof c.expires === 'number' && c.expires > 0) {
        cookie.expires = c.expires;
      }
      return cookie;
    });

  try {
    await context.addCookies(cookies);
    const hasLogin = cookies.some((c: any) => c.name === 'z_c0');
    logger.info(`已向 Obscura Context 注入 ${cookies.length} 个本地 Cookie${hasLogin ? '（含登录态 z_c0）' : '（未发现 z_c0 登录态）'}。`);
  } catch (e: any) {
    logger.warn(`向 Obscura 注入 Cookie 失败: ${e.message}`);
  }
}

/**
 * 互杀防护（P2）：有头登录窗口与任务抓取共享同一个持久化 profile——一方持有
 * context 时另一方换模式只能先把对方 close 掉。于是「任务运行中点登录」会
 * 杀掉正在抓取的浏览器，「登录窗口开着跑任务」会把用户正在验证的窗口关掉。
 * 用目的标记拒绝越权切换，而不是杀死对方：
 * - `interactive`（登录）期间，task 请求无头 context → 明确报错；
 * - `task`（抓取）持有期间，interactive 请求有头 context → 明确报错。
 * 任务内部的人机验证有头窗口属于 task 目的，不受第一条限制。
 */
export type BrowserPurpose = 'task' | 'interactive';
let interactiveSessionActive = false;
let taskBrowsingActive = false;

export function markInteractiveSession(active: boolean): void {
  interactiveSessionActive = active;
}

export function markTaskBrowsing(active: boolean): void {
  taskBrowsingActive = active;
}

export function getBrowserContext(headless = true, purpose: BrowserPurpose = 'task'): Promise<BrowserContext> {
  return withBrowserLock(() => getBrowserContextLocked(headless, purpose));
}

async function getBrowserContextLocked(headless: boolean, purpose: BrowserPurpose): Promise<BrowserContext> {
  const backend = shouldUseObscura(headless) ? 'obscura' : 'playwright';

  if (headless && purpose === 'task' && interactiveSessionActive) {
    throw new Error('知乎登录/验证窗口正在使用中，请先完成或关闭该窗口再运行任务。');
  }
  if (!headless && purpose === 'interactive' && taskBrowsingActive) {
    throw new Error('归档任务正在使用浏览器，请先暂停或取消任务后再登录。');
  }
  // 在锁内立旗：interactive 的窗口从创建起就受保护，不存在「窗口已开、
  // 标记未立」的竞态窗口被任务请求关掉。
  if (purpose === 'interactive') {
    interactiveSessionActive = true;
  }

  // 如果已存在 Context 且 headless 模式与当前请求的不一致，我们需要先关闭旧的
  if (activeContext && (currentHeadlessMode !== headless || currentBackend !== backend)) {
    logger.info(`切换浏览器模式：从 ${currentBackend}/${currentHeadlessMode} 切换为 ${backend}/${headless}。正在重启浏览器...`);
    await closeBrowserContextLocked();
  }

  if (activeContext) {
    return activeContext;
  }

  if (backend === 'obscura') {
    activeContext = await createObscuraContext();
    currentHeadlessMode = headless;

    await activeContext.addInitScript(() => {
      Object.defineProperty(navigator, 'webdriver', {
        get: () => undefined,
      });
      Object.defineProperty(navigator, 'languages', {
        get: () => ['zh-CN', 'zh', 'en'],
      });
      Object.defineProperty(navigator, 'plugins', {
        get: () => [1, 2, 3, 4, 5],
      });
      (window as any).chrome = {
        runtime: {},
        loadTimes: () => {},
        csi: () => {},
        app: {}
      };
    });

    await injectStoredCookies(activeContext);

    logger.info(`成功连接 Obscura Context (headless: ${headless}, endpoint: ${getCdpEndpoint()})`);
    return activeContext;
  }

  const launchTargets = browserLaunchTargets(headless, process.env);
  let lastError: any = null;
  const resolvedUserAgent = await resolveUserAgent();

  for (const target of launchTargets) {
    try {
      const options: any = {
        headless,
        viewport: { width: 1280, height: 800 },
        userAgent: resolvedUserAgent,
        args: [
          '--disable-blink-features=AutomationControlled',
          '--no-sandbox',
          '--disable-infobars',
          '--disk-cache-dir=' + browserCacheDir,
        ]
      };
      if ('executablePath' in target) {
        options.executablePath = target.executablePath;
      } else {
        options.channel = target.channel;
      }
      fs.mkdirSync(browserCacheDir, { recursive: true });
      activeContext = await chromium.launchPersistentContext(chromeProfileDir, options);
      currentHeadlessMode = headless;
      currentBackend = 'playwright';

      // 注入防检测指纹伪装
      await activeContext.addInitScript(() => {
        // 隐藏 webdriver 特征
        Object.defineProperty(navigator, 'webdriver', {
          get: () => undefined,
        });
        // 伪装 languages
        Object.defineProperty(navigator, 'languages', {
          get: () => ['zh-CN', 'zh', 'en'],
        });
        // 伪装 plugins 长度防止被判空
        Object.defineProperty(navigator, 'plugins', {
          get: () => [1, 2, 3, 4, 5],
        });
        // 伪装 chrome 属性
        (window as any).chrome = {
          runtime: {},
          loadTimes: () => {},
          csi: () => {},
          app: {}
        };
      });

      const browserLabel = 'executablePath' in target
        ? target.executablePath
        : target.channel;
      logger.info(`成功使用浏览器 ${browserLabel} 启动 Playwright Context (headless: ${headless})`);
      return activeContext;
    } catch (e: any) {
      lastError = e;
    }
  }

  throw new Error(`无法启动 Playwright 浏览器，已尝试系统 Chrome、Edge 及默认 Chromium。错误信息: ${lastError?.message}`);
}

/**
 * 读取本地持久化登录凭据（.obscura-profile/cookies.dpapi）中是否存在有效 z_c0。
 * 每次成功的登录/人机验证流程都会通过 syncCookiesToObscuraStorage 刷新该文件，
 * 因此它是 sidecar 侧的登录态事实来源，可用于无浏览器的只读判定。
 */
async function hasStoredLoginCookie(): Promise<boolean> {
  const parsed = await readStoredCookies();
  if (!parsed) {
    return false;
  }
  const now = Math.floor(Date.now() / 1000);
  return parsed.some((c: any) =>
    c && c.name === 'z_c0'
    && (typeof c.expires !== 'number' || !Number.isFinite(c.expires) || c.expires <= 0 || c.expires > now));
}

/**
 * 只读登录态查询（P1-8）：
 * - 绝不新建/切换 context——曾经有头登录或验证码窗口活跃时，这里的模式检查会
 *   直接 closeBrowserContext，把用户正在验证的窗口杀掉并泄漏新 headless context；
 * - 有活跃 context 时只读其 Cookie；没有活跃 context 时只读持久化凭据文件，
 *   不为了一次状态查询拉起浏览器。
 */
export function getLoginStatus(): Promise<{ loggedIn: boolean }> {
  return withBrowserLock(async () => {
    if (activeContext) {
      try {
        const cookies = await activeContext.cookies();
        return { loggedIn: cookies.some(cookie => cookie.name === 'z_c0') };
      } catch (e: any) {
        logger.warn(`读取活跃浏览器 Cookie 失败，改用本地登录凭据判断: ${e?.message || e}`);
      }
    }
    return { loggedIn: await hasStoredLoginCookie() };
  });
}

export function closeBrowserContext(): Promise<void> {
  return withBrowserLock(closeBrowserContextLocked);
}

/**
 * 尽力而为的远端登出：知乎的退出登录入口是 GET https://www.zhihu.com/logout，
 * 按请求携带的 Cookie 定位并使服务端会话失效。失败只记日志——本地凭据无论
 * 如何都会删除（残余风险：若请求未生效，z_c0 将留待其自然过期）。
 */
async function attemptRemoteLogout(cookieHeader: string): Promise<void> {
  const controller = new AbortController();
  // 宿主侧 HTTP 总时限 15s——远端登出给 5s 上限，本地清除必须先于它完成。
  const timer = setTimeout(() => controller.abort(), 5000);
  try {
    await fetch('https://www.zhihu.com/logout', {
      method: 'GET',
      redirect: 'manual',
      signal: controller.signal,
      headers: {
        cookie: cookieHeader,
        referer: 'https://www.zhihu.com/',
        'user-agent': await resolveUserAgent()
      }
    });
    logger.info('已向知乎发送退出登录请求。');
  } catch (e: any) {
    logger.warn(`远端退出登录未完成（本地登录数据仍会清除）: ${e?.message || e}`);
  } finally {
    clearTimeout(timer);
  }
}

/**
 * 退出登录 / 清除登录数据（06-F-01）：先尽力让知乎服务端会话失效，再关闭
 * 浏览器并删除整份本地登录档案——.obscura-profile 的 DPAPI Cookie 文件、
 * Chromium profile 目录（Cookies 库、localStorage 等）与浏览器缓存目录。
 * 浏览器被登录窗口或归档任务占用时拒绝执行（与互杀防护同一规则）。
 */
export function clearLoginData(): Promise<void> {
  return withBrowserLock(async () => {
    if (taskBrowsingActive || interactiveSessionActive) {
      throw new Error('浏览器正被登录窗口或归档任务占用，请先完成或取消后再退出登录。');
    }
    // 先取 Cookie 再删本地数据：远端登出放最后（尽力而为），宿主 HTTP 时限内
    // 本地清除必定已完成——即使响应超时，用户态也已经真正退出。
    const stored = await readStoredCookies();
    const cookieHeader = (stored || [])
      .filter((c: any) => c && c.name && c.value !== undefined)
      .map((c: any) => `${c.name}=${c.value}`)
      .join('; ');
    await closeBrowserContextLocked();
    for (const dir of [obscuraStorageDir, chromeProfileDir, browserCacheDir]) {
      fs.rmSync(dir, { recursive: true, force: true, maxRetries: 5, retryDelay: 300 });
    }
    logger.info('已清除知乎登录数据（Cookie、浏览器档案与缓存目录）。');
    if (cookieHeader) {
      await attemptRemoteLogout(cookieHeader);
    }
  });
}

/** 锁内关闭实现：必须容忍任何异常（常用于 finally），绝不能把 rejection 抛给调用方（P1-7）。 */
async function closeBrowserContextLocked(): Promise<void> {
  const backend = currentBackend;

  try {
    if (backend === 'obscura') {
      if (activeBrowser) {
        await activeBrowser.close().catch(() => {});
      } else if (activeContext) {
        await activeContext.close().catch(() => {});
      }
    } else if (activeContext) {
      await activeContext.close().catch((e: any) => {
        logger.warn(`关闭 Playwright Context 失败: ${e?.message || e}`);
      });
    }
  } finally {
    if (activeObscuraProcess) {
      try {
        activeObscuraProcess.kill();
      } catch {
        // kill 不允许把异常带出清理路径（P1-7）。
      }
      activeObscuraProcess = null;
    }
    activeContext = null;
    activeBrowser = null;
    currentHeadlessMode = null;
    currentBackend = null;
  }
}
