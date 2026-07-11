import { getBrowserContext, closeBrowserContext, syncCookiesToObscuraStorage } from './browser.js';
import { scrapePeopleIndex, ScrapedIndexItem } from './indexer.js';
import { scrapeAnswer, scrapeArticle, writeMarkdownFile } from './extractor.js';
import { 
  saveTask, 
  getTask, 
  saveItem, 
  saveTaskItem, 
  getTaskItems, 
  refreshTaskCounts, 
  getAuthorSuccessItems,
  replaceTaskIndex,
  tryStartTask,
  Task
} from './db.js';
import { logger, randomSleep, sleep, sanitizeFilename } from './utils.js';
import * as path from 'path';
import * as fs from 'fs';
import { resolveArchiveOutputDir } from './runtime-paths.js';

// SSE 控制台或日志全局事件总线回调
let progressCallback: ((taskId: string, status: string, message: string) => void) | null = null;
let runQueue: Promise<void> = Promise.resolve();
const queuedTaskIds = new Set<string>();

export function setSchedulerProgressCallback(cb: typeof progressCallback) {
  progressCallback = cb;
}

export function queueTask(taskId: string): boolean {
  if (queuedTaskIds.has(taskId)) {
    logger.warn(`任务已经在运行队列中: ${taskId}`);
    return false;
  }

  queuedTaskIds.add(taskId);
  runQueue = runQueue
    .catch(err => logger.error(`任务队列上一个任务异常: ${err?.message || err}`))
    .then(() => runTaskInternal(taskId))
    .finally(() => {
      queuedTaskIds.delete(taskId);
    });
  return true;
}

function emitProgress(taskId: string, status: string, message: string) {
  logger.info(`[Task ${taskId}] Status: ${status} | ${message}`);
  if (progressCallback) {
    progressCallback(taskId, status, message);
  }
}

/**
 * 交互式解决人机验证
 * 关闭无头浏览器 -> 开启有头浏览器 -> 等待用户滑动验证 -> 自动检测通过 -> 关闭有头浏览器 -> 重建无头浏览器并继续
 */
async function handleCaptchaInteractively(taskId: string, targetUrl: string): Promise<any> {
  emitProgress(taskId, 'running', '⚠️ 检测到防爬验证！正在打开有头浏览器，请手动完成验证码...');
  
  // 1. 关闭当前的无头浏览器
  await closeBrowserContext();
  
  // 2. 以有头模式重新启动浏览器
  const context = await getBrowserContext(false);
  const page = await context.newPage();
  
  // 3. 打开触发验证码的页面，或者知乎首页
  try {
    await page.goto(targetUrl, { waitUntil: 'domcontentloaded', timeout: 30000 });
  } catch (e: any) {
    logger.warn(`打开验证页面出错: ${e.message}`);
  }
  
  emitProgress(taskId, 'running', '👉 请在弹出的浏览器窗口中手动完成人机验证。完成验证后，程序会自动检测并通过，恢复后台抓取。');

  const maxWait = 5 * 60 * 1000; // 最多等待5分钟
  const checkInterval = 2000;
  let elapsed = 0;
  let success = false;
  
  while (elapsed < maxWait) {
    try {
      if (page.isClosed()) {
        logger.warn('人机验证浏览器窗口已被关闭。');
        break;
      }
      
      const currentUrl = page.url();
      const isUnhuman = currentUrl.includes('unhuman') || currentUrl.includes('captcha') || currentUrl.includes('signin');
      const cookies = await context.cookies();
      const hasLoginCookie = cookies.some(c => c.name === 'z_c0');
      const profileExists = await page.$('.AppHeader-profile, .AppHeader-user').then(el => !!el);
      
      if (!isUnhuman && (hasLoginCookie || profileExists)) {
        success = true;
        break;
      }
    } catch (err) {
      break;
    }
    await sleep(checkInterval);
    elapsed += checkInterval;
  }
  
  if (success) {
    emitProgress(taskId, 'running', '✅ 验证通过！正在重新切换回无头抓取...');
  } else {
    emitProgress(taskId, 'failed', '❌ 验证超时或窗口关闭。任务将停止，避免无限等待。');
  }
  
  // 4. 关闭有头浏览器
  await syncCookiesToObscuraStorage(context);
  await closeBrowserContext();

  if (!success) {
    throw new Error('CAPTCHA_REQUIRED: 人机验证超时或窗口关闭');
  }
  
  // 5. 重新启动无头浏览器并返回新 page
  const newContext = await getBrowserContext(true);
  const newPage = await newContext.newPage();
  return newPage;
}

/**
 * 创建知乎归档任务
 */
export async function createTask(
  peopleId: string,
  itemTypes: 'answers' | 'articles' | 'all',
  options: {
    topN?: number | null;
    sortBy?: 'time' | 'vote';
    outputDir?: string;
  } = {}
): Promise<string> {
  const taskId = `task_${peopleId}_${Date.now()}`;
  const outputDir = options.outputDir || resolveArchiveOutputDir({
    cwd: process.cwd(),
    environment: process.env,
  });
  const sortBy = options.sortBy || 'time';
  const topN = options.topN || null;

  logger.info(`正在创建新任务: ${taskId} (People: ${peopleId}, Types: ${itemTypes})`);

  // 保存初始任务到 SQLite
  saveTask({
    id: taskId,
    input_url: `https://www.zhihu.com/people/${peopleId}`,
    author_id: peopleId,
    author_name: '', // 抓取完索引后再更新
    item_types: itemTypes,
    output_dir: outputDir,
    sort_by: sortBy,
    top_n: topN,
    status: 'pending',
    index_status: 'pending',
    index_completed_at: null,
    total_count: 0,
    success_count: 0,
    failed_count: 0
  });

  return taskId;
}

/**
 * 运行或恢复任务 (包含索引抓取和正文抓取)
 */
export async function runTask(taskId: string) {
  return runTaskInternal(taskId);
}

async function runTaskInternal(taskId: string) {
  const task = getTask(taskId);
  if (!task) {
    throw new Error(`任务不存在: ${taskId}`);
  }

  if (!tryStartTask(taskId)) {
    logger.warn(`任务已经在运行中: ${taskId}`);
    return;
  }

  emitProgress(taskId, 'running', '任务开始启动...');

  let context;
  try {
    context = await getBrowserContext(true); // 无头模式
    let page = await context.newPage();

    // 2. 检查并拉取索引（如果当前没有任何关联的 task_items，表明是首次拉取索引）
    let taskItems = getTaskItems(taskId);
    if (taskItems.length === 0 || task.index_status !== 'complete') {
      emitProgress(taskId, 'running', '正在抓取答主主页内容列表索引...');
      saveTask({ id: taskId, index_status: 'running' });
      
      const scrapedIndexes: ScrapedIndexItem[] = [];
      const topN = task.top_n;

      if (task.item_types === 'answers' || task.item_types === 'all') {
        const answers = await scrapePeopleIndex(page, task.author_id, 'answers', topN);
        scrapedIndexes.push(...answers);
      }

      // 如果有暂停，需要在这里检查
      if (checkIsPaused(taskId)) return;

      if (task.item_types === 'articles' || task.item_types === 'all') {
        const articles = await scrapePeopleIndex(page, task.author_id, 'articles', topN);
        scrapedIndexes.push(...articles);
      }

      if (scrapedIndexes.length === 0) {
        emitProgress(taskId, 'failed', '未从答主主页中发现任何有效的回答或文章。');
        saveTask({ id: taskId, status: 'failed' });
        return;
      }

      // 获取并更新作者名
      const authorName = scrapedIndexes[0].authorName || '未知作者';

      // 用单个事务替换索引，避免半写入后被误认为索引完成。
      replaceTaskIndex(taskId, authorName, scrapedIndexes);
      taskItems = getTaskItems(taskId);
      emitProgress(taskId, 'running', `列表扫描完毕。共发现 ${taskItems.length} 个条目，开始消费正文队列...`);
    } else {
      emitProgress(taskId, 'running', '检测到已存在的内容列表，恢复/断点续爬队列中...');
    }

    // 3. 构建待消费的正文抓取队列并排序
    let pendingItems = taskItems.filter(ti => ti.status === 'pending' || ti.status === 'failed');
    
    // 排序
    if (task.sort_by === 'vote') {
      pendingItems.sort((a, b) => b.voteup_count - a.voteup_count);
    } else {
      // 默认按创建时间降序 (最新的在前面)
      pendingItems.sort((a, b) => b.created_time - a.created_time);
    }

    emitProgress(taskId, 'running', `待抓取正文条目共: ${pendingItems.length} 个`);

    // 4. 消费队列
    const outputBaseDir = path.resolve(process.cwd(), task.output_dir);

    // 全局速率预算：滑动窗口记录最近条目成败，用于自适应冷却与保护性中止
    const recentResults: boolean[] = [];
    let consumedCount = 0;
    const countTrailingFailures = (results: boolean[]): number => {
      let n = 0;
      for (let j = results.length - 1; j >= 0 && !results[j]; j--) n++;
      return n;
    };

    for (let i = 0; i < pendingItems.length; i++) {
      // 循环中首先检查状态是否被暂停
      if (checkIsPaused(taskId)) {
        emitProgress(taskId, 'paused', '任务已被用户手动暂停。');
        return;
      }

      const item = pendingItems[i];
      emitProgress(taskId, 'running', `正在抓取进度 [${i + 1}/${pendingItems.length}]: (${item.item_type === 'answer' ? '回答' : '文章'}) ${item.title}`);

      // 抓取并重试
      let success = false;
      let failureCode = 'UNKNOWN';
      let errorMessage = '';
      let retryCount = 0;
      let captchaAttempts = 0;
      const maxRetries = 3;
      const maxCaptchaAttempts = 1;

      while (retryCount <= maxRetries && !success) {
        if (checkIsPaused(taskId)) {
          emitProgress(taskId, 'paused', '任务在重试前被暂停。');
          return;
        }

        if (retryCount > 0) {
          const delay = Math.pow(2, retryCount) * 1000;
          logger.info(`抓取重试 [${retryCount}/${maxRetries}], 等待延时: ${delay}ms`);
          await sleep(delay);
        }

        try {
          // 每次抓取之间加入随机防爬延迟
          if (retryCount === 0) {
            await randomSleep(2000, 5000);
          }

          let extracted;
          if (item.item_type === 'answer') {
            extracted = await scrapeAnswer(page, item.url);
          } else {
            extracted = await scrapeArticle(page, item.url);
          }

          const relativePath = await writeMarkdownFile(extracted, outputBaseDir);
          
          // 更新数据库 items 属性缓存（例如最新的 voteup_count）
          saveItem({
            id: extracted.id,
            item_type: extracted.type,
            author_id: extracted.authorId,
            author_name: extracted.authorName,
            title: extracted.title,
            answer_id: extracted.answerId || null,
            question_id: extracted.questionId || null,
            article_id: extracted.articleId || null,
            url: extracted.url,
            question_url: extracted.questionUrl || null,
            created_time: extracted.createdTime,
            updated_time: extracted.updatedTime,
            voteup_count: extracted.voteupCount,
            comment_count: extracted.commentCount
          });

          // 保存状态为 success
          saveTaskItem({
            task_id: taskId,
            item_id: item.item_id,
            status: 'success',
            output_path: relativePath,
            failure_code: null,
            error_message: null,
            created_at: item.created_at,
            updated_at: Date.now()
          });

          success = true;
        } catch (e: any) {
          logger.error(`抓取单篇发生异常: ${e.message}`);
          errorMessage = e.message || 'Unknown error';

          // 分析错误类型
          if (errorMessage.includes('LOGIN_REQUIRED')) {
            failureCode = 'LOGIN_REQUIRED';
            retryCount = maxRetries + 1; // 账号未登录，直接中断，不进行重试
          } else if (errorMessage.includes('CAPTCHA_REQUIRED')) {
            failureCode = 'CAPTCHA_REQUIRED';
            if (captchaAttempts >= maxCaptchaAttempts) {
              retryCount = maxRetries + 1;
              break;
            }
            captchaAttempts++;
            try {
              page = await handleCaptchaInteractively(taskId, item.url);
              retryCount = 0;
              continue;
            } catch (err: any) {
              logger.error(`交互式人机验证启动失败: ${err.message}`);
              retryCount = maxRetries + 1; // 交互失败直接中断
            }
          } else if (errorMessage.includes('DOM_NOT_FOUND')) {
            failureCode = 'DOM_NOT_FOUND';
          } else if (errorMessage.includes('CONTENT_EMPTY')) {
            failureCode = 'CONTENT_EMPTY';
          } else if (errorMessage.includes('timeout') || errorMessage.includes('Navigation')) {
            failureCode = 'NETWORK_ERROR';
          } else {
            failureCode = 'UNKNOWN';
          }

          retryCount++;
        }
      }

      if (!success) {
        saveTaskItem({
          task_id: taskId,
          item_id: item.item_id,
          status: 'failed',
          output_path: null,
          failure_code: failureCode,
          error_message: errorMessage,
          created_at: item.created_at,
          updated_at: Date.now()
        });

        // 如果是不可恢复的登录错误，我们建议直接中断整个大任务，不要傻傻等待后面几十个任务报错
        if (failureCode === 'LOGIN_REQUIRED' || failureCode === 'CAPTCHA_REQUIRED') {
          saveTask({ id: taskId, status: 'failed' });
          emitProgress(taskId, 'failed', `遇到登录障碍，任务终止: ${errorMessage}。重新 npm run login 后，可用「重跑失败条目」断点续跑。`);
          return;
        }
      }

      refreshTaskCounts(taskId);

      // —— 全局速率预算与自适应风控保护 ——
      recentResults.push(success);
      if (recentResults.length > 10) recentResults.shift();
      consumedCount++;

      const failuresInWindow = recentResults.filter(ok => !ok).length;
      if (recentResults.length >= 10 && failuresInWindow >= 8) {
        saveTask({ id: taskId, status: 'failed' });
        emitProgress(taskId, 'failed', '⛔ 最近 10 篇失败率过高，疑似触发站点风控，任务已保护性中止。请稍后用「重跑失败条目」恢复。');
        return;
      }

      const consecutiveFailures = countTrailingFailures(recentResults);
      if (consecutiveFailures >= 3) {
        emitProgress(taskId, 'running', `🛡️ 已连续失败 ${consecutiveFailures} 篇，进入风控冷却（60-120 秒）...`);
        await randomSleep(60000, 120000);
      } else if (consumedCount % 50 === 0 && i < pendingItems.length - 1) {
        emitProgress(taskId, 'running', `🛡️ 已连续抓取 ${consumedCount} 篇，休息 30-60 秒模拟人类阅读节奏...`);
        await randomSleep(30000, 60000);
      }
    }

    // 5. 循环结束，检查最终任务结果
    const finalTask = getTask(taskId);
    if (finalTask) {
      const isComplete = finalTask.success_count + finalTask.failed_count === finalTask.total_count;
      const status: Task['status'] = isComplete && finalTask.failed_count === 0
        ? 'success'
        : (isComplete && finalTask.success_count > 0 ? 'partial_success' : 'failed');
      saveTask({ id: taskId, status });

      try {
        const successItems = getTaskItems(taskId).filter(ti => ti.status === 'success');
        const authors = new Map<string, string>();
        for (const item of successItems) {
          if (item.author_id && item.author_name) {
            authors.set(item.author_id, item.author_name);
          }
        }
        for (const [authId, authName] of authors.entries()) {
          generateAuthorIndex(authId, authName, outputBaseDir);
        }
      } catch (err: any) {
        logger.error(`生成导航索引失败: ${err.message}`);
      }

      emitProgress(taskId, status, `任务执行完毕。总数: ${finalTask.total_count}, 成功: ${finalTask.success_count}, 失败: ${finalTask.failed_count}`);
    }

  } catch (e: any) {
    logger.error(`任务执行过程严重崩溃: ${e.message}`);
    saveTask({ id: taskId, status: 'failed' });
    emitProgress(taskId, 'failed', `严重错误导致任务异常中止: ${e.message}`);
  } finally {
    await closeBrowserContext();
  }
}

/**
 * 辅助方法：检查数据库状态是否要求暂停
 */
function checkIsPaused(taskId: string): boolean {
  const task = getTask(taskId);
  return task ? task.status === 'paused' : false;
}

/**
 * 为答主目录生成 Obsidian 双链导航索引 index.md
 */
export function generateAuthorIndex(authorId: string, authorName: string, outputBaseDir: string) {
  const items = getAuthorSuccessItems(authorId);
  if (items.length === 0) return;

  const authorDirName = sanitizeFilename(authorName, authorId).replace(/_[^_]+$/, '');
  const authorPath = path.resolve(outputBaseDir, authorDirName);
  const indexPath = path.join(authorPath, 'index.md');

  const answers = items.filter(i => i.item_type === 'answer');
  const articles = items.filter(i => i.item_type === 'article');

  let md = `# ${authorName} 的内容归档\n\n`;
  md += `> 本归档由 Zhihu Packer 自动生成。  \n`;
  md += `> 共归档回答: **${answers.length}** 篇，文章: **${articles.length}** 篇。  \n\n`;

  md += `## 回答列表\n\n`;
  if (answers.length === 0) {
    md += `暂无已归档的回答。\n\n`;
  } else {
    for (const item of answers) {
      if (!item.output_path) continue;
      const fileName = path.basename(item.output_path);
      const dateStr = new Date(item.created_time * 1000).toISOString().split('T')[0];
      md += `- [[${fileName}|${item.title}]] (发布于: ${dateStr} | 赞同数: ${item.voteup_count})\n`;
    }
    md += `\n`;
  }

  md += `## 文章列表\n\n`;
  if (articles.length === 0) {
    md += `暂无已归档的文章。\n\n`;
  } else {
    for (const item of articles) {
      if (!item.output_path) continue;
      const fileName = path.basename(item.output_path);
      const dateStr = new Date(item.created_time * 1000).toISOString().split('T')[0];
      md += `- [[${fileName}|${item.title}]] (发布于: ${dateStr} | 赞同数: ${item.voteup_count})\n`;
    }
    md += `\n`;
  }

  fs.writeFileSync(indexPath, md, 'utf-8');
  logger.info(`已成功生成/更新答主 ${authorName} 的导航索引: ${indexPath}`);
}
