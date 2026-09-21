import * as path from 'path';
import * as fs from 'fs';

export function sanitizeFilename(title: string, id: string, maxLength = 100): string {
  // 移除 windows 非法字符
  let cleaned = title.replace(/[\\/:*?"<>|]/g, '');
  // 替换连续空白为单个空格，移除收尾空格
  cleaned = cleaned.replace(/\s+/g, ' ').trim();
  // 如果清理后为空，使用默认名
  if (!cleaned) {
    cleaned = 'untitled';
  }
  // 限制长度
  if (cleaned.length > maxLength) {
    cleaned = cleaned.slice(0, maxLength).trim();
  }
  // 拼接 ID 确保唯一性
  return `${cleaned}_${id}`;
}

export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * P3-29：wordCount 的规范口径 = 非空白的 Unicode 标量（code point）数。
 * String.prototype.length 数的是 UTF-16 码元，星外来字（如 emoji、生僻字）会
 * 被数成 2，与 Rust 端按 scalar 统计的口径分叉。用展开迭代按 code point 计数。
 * 空白集合用 `\p{White_Space}` 而非 `\s`：JS `\s` 多算 U+FEFF、漏算 U+0085
 * NEL，与 Rust `char::is_whitespace`（Unicode White_Space 属性）差两个码点。
 */
export function countWordChars(text: string): number {
  return [...text.replace(/\p{White_Space}+/gu, '')].length;
}

export function randomSleep(min = 2000, max = 5000): Promise<void> {
  const ms = Math.floor(Math.random() * (max - min + 1) + min);
  return sleep(ms);
}

class Logger {
  private logFile: string;

  /**
   * P2-27④：日志写在 app 目录且永不轮转，长跑会让 zhihu-packer.log 无限膨胀。
   * 保持在原位置（最小破坏），超过 10MB 时改名为 .1（仅保留一代）再写新文件。
   */
  private static readonly MAX_LOG_BYTES = 10 * 1024 * 1024;

  constructor() {
    this.logFile = path.resolve(process.cwd(), 'zhihu-packer.log');
  }

  setLogFile(filePath: string) {
    this.logFile = filePath;
  }

  private rotateIfOversized() {
    try {
      const stat = fs.statSync(this.logFile);
      if (stat.size <= Logger.MAX_LOG_BYTES) return;
      const rotated = `${this.logFile}.1`;
      try { fs.rmSync(rotated, { force: true }); } catch {}
      fs.renameSync(this.logFile, rotated);
    } catch {
      // 日志轮转失败绝不能影响主流程
    }
  }

  log(message: string, level: 'info' | 'warn' | 'error' = 'info') {
    const timestamp = new Date().toISOString();
    const formatted = `[${timestamp}] [${level.toUpperCase()}] ${message}`;
    
    if (level === 'error') {
      console.error(formatted);
    } else if (level === 'warn') {
      console.warn(formatted);
    } else {
      console.log(formatted);
    }

    try {
      this.rotateIfOversized();
      fs.appendFileSync(this.logFile, formatted + '\n', 'utf-8');
    } catch (e) {
      // ignore
    }
  }

  info(message: string) {
    this.log(message, 'info');
  }

  warn(message: string) {
    this.log(message, 'warn');
  }

  error(message: string) {
    this.log(message, 'error');
  }
}

export const logger = new Logger();

/**
 * 06-F-05：日志脱敏。URL 只保留 host + 最后一段路径（足够定位失败资源，
 * 不外泄完整 URL 及其签名参数）；本地绝对路径只保留最后两级（目录名+
 * 文件名），不外泄用户目录结构。
 */
export function redactUrlForLog(raw: string): string {
  try {
    const url = new URL(raw);
    const segments = url.pathname.split('/').filter(Boolean);
    const tail = segments.length > 0 ? segments[segments.length - 1] : '';
    return tail ? `${url.host}/…/${tail}` : url.host;
  } catch {
    return '[url]';
  }
}

export function redactPathForLog(raw: string): string {
  const segments = String(raw).split(/[\\/]+/).filter(Boolean);
  return segments.slice(-2).join('/') || '[path]';
}

// P3-7：调试快照（debug-*.html/png）保存的是已登录会话下的完整页面内容。
// 写新快照时顺手清掉超过 7 天的旧快照，避免敏感页面内容长期驻留磁盘。
const DEBUG_SNAPSHOT_MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000;

export function pruneDebugSnapshots(dir: string): void {
  try {
    const cutoff = Date.now() - DEBUG_SNAPSHOT_MAX_AGE_MS;
    for (const name of fs.readdirSync(dir)) {
      if (!name.startsWith('debug-')) continue;
      const filePath = path.join(dir, name);
      try {
        if (fs.statSync(filePath).mtimeMs < cutoff) fs.unlinkSync(filePath);
      } catch {
        // 单文件清理失败不影响主流程。
      }
    }
  } catch {
    // 目录不可读时静默跳过。
  }
}

import { Page } from 'playwright-core';

export async function evaluateClean<T>(page: Page, fn: (...args: any[]) => any, ...args: any[]): Promise<T> {
  let fnStr = fn.toString();
  // 替换 __name, __name2, __name3 等为恒等函数调用
  fnStr = fnStr.replace(/__name\d*\(/g, '((f)=>f)(');
  
  return page.evaluate(([code, ...params]) => {
    const cleanFn = new Function(`return (${code}).apply(null, arguments)`);
    return cleanFn.apply(null, params);
  }, [fnStr, ...args]) as Promise<T>;
}
