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

export function randomSleep(min = 2000, max = 5000): Promise<void> {
  const ms = Math.floor(Math.random() * (max - min + 1) + min);
  return sleep(ms);
}

class Logger {
  private logFile: string;

  constructor() {
    this.logFile = path.resolve(process.cwd(), 'zhihu-packer.log');
  }

  setLogFile(filePath: string) {
    this.logFile = filePath;
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
