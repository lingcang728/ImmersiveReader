import { chromium, Browser, BrowserContext, Cookie } from 'playwright-core';
import { spawn, ChildProcess } from 'child_process';
import * as path from 'path';
import * as fs from 'fs';
import * as net from 'net';
import { logger } from './utils.js';
import { resolveBrowserCacheDir, resolveBrowserExecutable, resolveProfileDir } from './runtime-paths.js';

let activeContext: BrowserContext | null = null;
let activeBrowser: Browser | null = null;
let activeObscuraProcess: ChildProcess | null = null;
let currentHeadlessMode: boolean | null = null;
let currentBackend: 'obscura' | 'playwright' | null = null;

let obscuraPort = process.env.OBSCURA_PORT ? Number(process.env.OBSCURA_PORT) : 0;
const chromeProfileDir = resolveProfileDir({ cwd: process.cwd(), environment: process.env });
const browserCacheDir = resolveBrowserCacheDir({ cwd: process.cwd(), environment: process.env });
const obscuraStorageDir = path.resolve(process.cwd(), '.obscura-profile');
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
  // 默认走 Playwright 持久化 Profile（.browser-profile）：它与 npm run login 的登录态共享，
  // 且实测能通过知乎 zse-ck 反爬质询；而无头 Obscura 会被 zse-ck 反复质询、拿不到内容。
  // 仅当显式设置 ZHIHU_PACKER_BROWSER=obscura 时才使用 Obscura 后端。
  return headless && process.env.ZHIHU_PACKER_BROWSER === 'obscura';
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

  activeObscuraProcess = spawn(executable, args, {
    stdio: 'ignore',
    windowsHide: true
  });

  activeObscuraProcess.on('exit', () => {
    activeObscuraProcess = null;
  });

  await waitForObscuraReady();
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

export async function syncCookiesToObscuraStorage(context: BrowserContext): Promise<void> {
  const cookies = await context.cookies();
  fs.mkdirSync(obscuraStorageDir, { recursive: true });
  fs.writeFileSync(
    path.join(obscuraStorageDir, 'cookies.json'),
    JSON.stringify(cookies.map(toObscuraCookie), null, 2),
    'utf-8'
  );
  logger.info(`已同步 ${cookies.length} 个 Cookie 到 Obscura 存储目录: ${obscuraStorageDir}`);
}

/**
 * 把 npm run login 同步出来的 .obscura-profile/cookies.json 显式注入到 Obscura Context。
 *
 * Obscura 通过 CDP 连接，并不会自动读取 storage-dir 里的 cookies.json，
 * 因此必须在连接后用 Playwright 的 addCookies 主动注入，否则无头任务始终是未登录态。
 * 同时还原被 toObscuraCookie 去掉的前导点，使 z_c0 等域 Cookie 对 www / zhuanlan 子域同时生效。
 */
async function injectStoredCookies(context: BrowserContext): Promise<void> {
  const file = path.join(obscuraStorageDir, 'cookies.json');
  if (!fs.existsSync(file)) {
    logger.warn(`未找到本地登录 Cookie 文件 (${file})，Obscura 将以未登录态运行。请先运行 npm run login。`);
    return;
  }

  let parsed: any;
  try {
    parsed = JSON.parse(fs.readFileSync(file, 'utf-8'));
  } catch (e: any) {
    logger.warn(`解析 ${file} 失败，跳过 Cookie 注入: ${e.message}`);
    return;
  }
  if (!Array.isArray(parsed) || parsed.length === 0) {
    return;
  }

  const mapSameSite = (s: any): 'Strict' | 'Lax' | 'None' => {
    const v = String(s || '').toLowerCase();
    if (v === 'strict') return 'Strict';
    if (v === 'none') return 'None';
    return 'Lax';
  };

  const cookies = parsed
    // __zse_ck 是知乎反爬的易失性质询令牌，注入旧值反而会与现场质询冲突，交由实时质询生成。
    .filter((c: any) => c && c.name && c.domain && c.name !== '__zse_ck')
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

export async function getBrowserContext(headless = true): Promise<BrowserContext> {
  const backend = shouldUseObscura(headless) ? 'obscura' : 'playwright';

  // 如果已存在 Context 且 headless 模式与当前请求的不一致，我们需要先关闭旧的
  if (activeContext && (currentHeadlessMode !== headless || currentBackend !== backend)) {
    logger.info(`切换浏览器模式：从 ${currentBackend}/${currentHeadlessMode} 切换为 ${backend}/${headless}。正在重启浏览器...`);
    await closeBrowserContext();
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

  const managedExecutable = resolveBrowserExecutable(process.env);
  const channels = managedExecutable ? [undefined] : ['chrome', 'msedge'];
  let lastError: any = null;
  const resolvedUserAgent = await resolveUserAgent();

  for (const channel of channels) {
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
      if (managedExecutable) {
        options.executablePath = managedExecutable;
      } else if (channel) {
        options.channel = channel;
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

      logger.info(`成功使用浏览器 channel: ${channel || 'playwright-default'} 启动 Playwright Context (headless: ${headless})`);
      return activeContext;
    } catch (e: any) {
      lastError = e;
    }
  }

  throw new Error(`无法启动 Playwright 浏览器，已尝试系统 Chrome、Edge 及默认 Chromium。错误信息: ${lastError?.message}`);
}

export async function getLoginStatus(): Promise<{ loggedIn: boolean }> {
  const wasActive = activeContext !== null;
  const context = await getBrowserContext(true);
  const cookies = await context.cookies();
  if (!wasActive) {
    await closeBrowserContext();
  }
  return { loggedIn: cookies.some(cookie => cookie.name === 'z_c0') };
}

export async function closeBrowserContext() {
  const backend = currentBackend;

  try {
    if (backend === 'obscura') {
      if (activeBrowser) {
        await activeBrowser.close().catch(() => {});
      }
    } else if (activeContext) {
      await activeContext.close();
    }
  } finally {
    if (activeObscuraProcess) {
      activeObscuraProcess.kill();
      activeObscuraProcess = null;
    }
    activeContext = null;
    activeBrowser = null;
    currentHeadlessMode = null;
    currentBackend = null;
  }
}
