import { getBrowserContext, closeBrowserContext } from './browser.js';
import * as fs from 'fs';
import * as path from 'path';
import { logger } from './utils.js';
import { resolveArchiveOutputDir } from './runtime-paths.js';

export async function runDoctor(): Promise<boolean> {
  let allPassed = true;
  logger.info('开始运行 Doctor 系统自检...');

  // 1. 检查 Node.js 版本
  const nodeVersion = process.versions.node;
  const [major, minor] = nodeVersion.split('.').map(Number);
  logger.info(`Node.js 版本: v${nodeVersion}`);
  if (major < 22 || (major === 22 && minor < 13)) {
    logger.error('警告: 本系统推荐 Node.js 版本 >= 22.13.0（原生 SQLite 支持稳定版本）。请考虑升级 Node.js。');
    allPassed = false;
  } else {
    logger.info('Node.js 版本检查通过。');
  }

  // 2. 检查输出目录读写权限
  const outputDir = resolveArchiveOutputDir({ cwd: process.cwd(), environment: process.env });
  try {
    if (!fs.existsSync(outputDir)) {
      fs.mkdirSync(outputDir, { recursive: true });
    }
    const testFile = path.join(outputDir, '.write-test');
    fs.writeFileSync(testFile, 'test', 'utf-8');
    fs.unlinkSync(testFile);
    logger.info(`输出目录读写权限检查通过: ${outputDir}`);
  } catch (e: any) {
    logger.error(`输出目录读写权限检查失败: ${e.message}`);
    allPassed = false;
  }

  // 3. 检查浏览器/CDP 启动和知乎连通性
  logger.info('正在尝试启动浏览器/CDP 并测试知乎连通性...');
  let context;
  try {
    context = await getBrowserContext(true);
    const page = await context.newPage();
    
    logger.info('正在请求 https://www.zhihu.com ...');
    const response = await page.goto('https://www.zhihu.com', { timeout: 25000, waitUntil: 'domcontentloaded' });
    const status = response?.status();
    
    if (status && status >= 200 && status < 400) {
      logger.info(`知乎连通性检查成功，HTTP 状态码: ${status}`);
    } else {
      logger.warn(`知乎连通性检查警告，HTTP 状态码: ${status || '未知'}`);
    }
    
    await page.close();
    logger.info('浏览器/CDP 启动与基本测试通过。');
  } catch (e: any) {
    logger.error(`浏览器/CDP 启动或知乎连接失败: ${e.message}`);
    allPassed = false;
  } finally {
    await closeBrowserContext();
  }

  if (allPassed) {
    logger.info('=== Doctor 自检全部通过 ===');
  } else {
    logger.error('=== Doctor 自检未全部通过，请检查上述错误 ===');
  }

  return allPassed;
}
