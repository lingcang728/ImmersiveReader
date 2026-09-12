import { getBrowserContext, closeBrowserContext, syncCookiesToObscuraStorage } from './browser.js';
import { scanLimitForSelection, scrapePeopleIndex, ScrapedIndexItem, selectIndexItems } from './indexer.js';
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
  upsertTaskIndexItems,
  saveIndexCheckpoint,
  readIndexCheckpoint,
  tryStartTask,
  recordPublishedTaskItems,
  Task
} from './db.js';
import { logger, randomSleep, sleep, sanitizeFilename } from './utils.js';
import * as path from 'path';
import * as fs from 'fs';
import { resolveArchiveOutputDir } from './runtime-paths.js';
import {
  isPublishableTaskStatus,
  publishTaskStage,
  resetTaskIncoming,
  resolvePublishedTaskItemPath,
  taskIncomingRoot,
} from './publish.js';

let runQueue: Promise<void> = Promise.resolve();
const queuedTaskIds = new Set<string>();

/**
 * 判断任务当前状态能否进入运行队列。
 * - pending：直接可入队；
 * - paused / failed / partial_success：复位为 pending 后入队（恢复 / 重跑失败条目）；
 * - running / cancelled / success：拒绝（运行中或终态）。
 * 返回 false 时任务不会被调度。
 */
export function markTaskQueueable(taskId: string): boolean {
  const task = getTask(taskId);
  if (!task) {
    logger.warn(`无法入队，任务不存在: ${taskId}`);
    return false;
  }
  if (task.status === 'running') {
    logger.warn(`任务已经在运行中: ${taskId}`);
    return false;
  }
  if (task.status === 'cancelled' || task.status === 'success') {
    logger.warn(`任务已到达终态(${task.status})，不可再入队: ${taskId}`);
    return false;
  }
  if (task.status !== 'pending') {
    // paused/failed/partial_success → pending，这样 tryStartTask 的 pending→running 门槛才放行。
    saveTask({ id: taskId, status: 'pending' });
  }
  return true;
}

export function queueTask(taskId: string): boolean {
  if (queuedTaskIds.has(taskId)) {
    // 已在队列中等待执行：若期间被用户暂停，/start 恢复时把它翻回 pending，
    // 使出队时 tryStartTask 正常放行（幂等恢复，不重复入队）。
    const queued = getTask(taskId);
    if (queued?.status === 'paused') {
      saveTask({ id: taskId, status: 'pending' });
      return true;
    }
    logger.warn(`任务已经在运行队列中: ${taskId}`);
    return false;
  }
  if (!markTaskQueueable(taskId)) {
    return false;
  }

  queuedTaskIds.add(taskId);
  // 尾部必须带 .catch：runTaskInternal 任何逃逸的 rejection（含 try 外的
  // 「任务不存在」与 finally 中的关闭异常）都不能成为未观察的 unhandledRejection，
  // 也不能污染队列导致后续任务永不被执行（P1-7）。
  runQueue = runQueue
    .catch(err => logger.error(`任务队列上一个任务异常: ${err?.message || err}`))
    .then(() => runTaskInternal(taskId))
    .catch(err => logger.error(`任务 ${taskId} 执行异常: ${err?.message || err}`))
    .finally(() => {
      queuedTaskIds.delete(taskId);
    });
  return true;
}

/** 任务终态：不再接受调度/暂停/取消。 */
const TERMINAL_TASK_STATUSES: ReadonlySet<Task['status']> = new Set([
  'success',
  'partial_success',
  'failed',
  'cancelled',
]);

export function cancelTask(taskId: string): boolean {
  const task = getTask(taskId);
  if (!task) {
    return false;
  }
  if (task.status === 'cancelled') {
    return true; // 幂等：已取消的任务再次取消视为成功
  }
  if (TERMINAL_TASK_STATUSES.has(task.status)) {
    return false;
  }
  // cancelled 是真正的终态：运行循环会在下一个检查点停下，排队任务出队时被
  // tryStartTask（pending→running）拒绝，永不再被调度执行（P1-9）。
  saveTask({ id: taskId, status: 'cancelled' });
  return true;
}

export function pauseTask(taskId: string): boolean {
  const task = getTask(taskId);
  if (!task) {
    return false;
  }
  if (task.status === 'paused') {
    return true; // 幂等
  }
  if (task.status === 'running' || task.status === 'pending') {
    saveTask({ id: taskId, status: 'paused' });
    return true;
  }
  return false;
}

function emitProgress(taskId: string, status: string, message: string) {
  logger.info(`[Task ${taskId}] Status: ${status} | ${message}`);
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
    // 暂停/取消要打断验证码等待，否则用户取消后仍要空等 5 分钟（P1-9）。
    const stoppedDuringCaptcha = checkTaskStopped(taskId);
    if (stoppedDuringCaptcha) {
      emitProgress(taskId, stoppedDuringCaptcha, '人机验证等待期间任务被暂停或取消。');
      break;
    }
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
    // tryStartTask 只放行 pending→running；paused/cancelled/终态任务走到这里说明
    // 是「入队后被暂停或取消」的排队任务——保持其状态原样返回，绝不翻回 running。
    const current = getTask(taskId);
    logger.warn(`任务跳过执行（当前状态: ${current?.status ?? 'missing'}）: ${taskId}`);
    return;
  }

  emitProgress(taskId, 'running', '任务开始启动...');

  let context;
  try {
    context = await getBrowserContext(true); // 无头模式
    let page = await context.newPage();
    const outputBaseDir = path.resolve(process.cwd(), task.output_dir);

    // 2. 检查并拉取索引（如果当前没有任何关联的 task_items，表明是首次拉取索引）
    let taskItems = getTaskItems(taskId);
    if (taskItems.length === 0 || task.index_status !== 'complete') {
      if (taskItems.length === 0) {
        resetTaskIncoming(outputBaseDir, taskId);
      } else {
        fs.mkdirSync(taskIncomingRoot(outputBaseDir, taskId), { recursive: true });
      }
      emitProgress(taskId, 'running', '正在抓取答主主页内容列表索引...');
      saveTask({ id: taskId, index_status: 'running' });
      
      const scrapedIndexes: ScrapedIndexItem[] = [];
      const scanLimit = scanLimitForSelection(task.top_n, task.sort_by);
      let lastAuthorName = task.author_name || '未知作者';
      const priorCheckpoint = readIndexCheckpoint(taskId);

      const persistIndexProgress = (
        contentType: 'answers' | 'articles' | 'all',
        items: ScrapedIndexItem[],
        statusMessage: string,
        paging?: { next: string | null; isEnd: boolean; pagesSeen: number; totals: number | null }
      ) => {
        const authorName = items[0]?.authorName || lastAuthorName;
        lastAuthorName = authorName;
        // Incremental upsert so crash mid-index keeps discovered rows.
        upsertTaskIndexItems(
          taskId,
          authorName,
          items.map((item) => ({
            id: item.id,
            type: item.type,
            authorId: item.authorId,
            authorName: item.authorName,
            title: item.title,
            url: item.url,
            createdTime: item.createdTime,
            updatedTime: item.updatedTime,
            voteupCount: item.voteupCount,
            commentCount: item.commentCount,
            questionId: item.questionId,
            questionUrl: item.questionUrl
          })),
          {
            contentType,
            next: paging?.next ?? priorCheckpoint?.next ?? null,
            isEnd: paging?.isEnd ?? false,
            pagesSeen: paging?.pagesSeen ?? priorCheckpoint?.pagesSeen ?? 0,
            discovered: items.length,
            totals: paging?.totals ?? priorCheckpoint?.totals ?? null,
            stage: 'index',
            statusMessage,
            lastHeartbeatAt: Date.now()
          }
        );
        emitProgress(taskId, 'running', statusMessage);
      };

      if (task.item_types === 'answers' || task.item_types === 'all') {
        const answers = await scrapePeopleIndex(
          page,
          task.author_id,
          'answers',
          scanLimit,
          (progress) => {
            scrapedIndexes.length = 0;
            // Keep articles if already scraped in this run — answers phase first so only answers here.
            scrapedIndexes.push(...progress.items);
            persistIndexProgress('answers', scrapedIndexes, progress.statusMessage, progress.paging);
          }
        );
        // Replace answers slice with final set
        const withoutAnswers = scrapedIndexes.filter((i) => i.type !== 'answer');
        scrapedIndexes.length = 0;
        scrapedIndexes.push(...withoutAnswers, ...answers);
        persistIndexProgress('answers', scrapedIndexes, `回答索引完成：${answers.length} 条`);
      }

      // 如果有暂停/取消，需要在这里检查
      const stoppedAfterAnswers = checkTaskStopped(taskId);
      if (stoppedAfterAnswers) {
        emitProgress(taskId, stoppedAfterAnswers, stoppedAfterAnswers === 'paused' ? '任务已被用户手动暂停。' : '任务已被用户取消。');
        return;
      }

      if (task.item_types === 'articles' || task.item_types === 'all') {
        const articles = await scrapePeopleIndex(
          page,
          task.author_id,
          'articles',
          scanLimit,
          (progress) => {
            const answersOnly = scrapedIndexes.filter((i) => i.type === 'answer');
            const merged = [...answersOnly, ...progress.items];
            persistIndexProgress('articles', merged, progress.statusMessage, progress.paging);
          }
        );
        const answersOnly = scrapedIndexes.filter((i) => i.type === 'answer');
        scrapedIndexes.length = 0;
        scrapedIndexes.push(...answersOnly, ...articles);
        persistIndexProgress('articles', scrapedIndexes, `文章索引完成：${articles.length} 条`);
      }

      if (scrapedIndexes.length === 0) {
        if (saveTaskStatusGuarded(taskId, 'failed')) {
          emitProgress(taskId, 'failed', '未从答主主页中发现任何有效的回答或文章。');
        }
        return;
      }

      // 获取并更新作者名
      const authorName = scrapedIndexes[0].authorName || lastAuthorName || '未知作者';
      const selectedIndexes = selectIndexItems(scrapedIndexes, task.top_n, task.sort_by);

      // Final replace keeps selection/topN consistent and marks index complete.
      replaceTaskIndex(taskId, authorName, selectedIndexes);
      saveIndexCheckpoint(taskId, {
        contentType: task.item_types === 'all' ? 'all' : task.item_types,
        isEnd: true,
        discovered: selectedIndexes.length,
        stage: 'content',
        statusMessage: `列表扫描完毕。共发现 ${selectedIndexes.length} 个条目`,
        lastHeartbeatAt: Date.now()
      });
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
    const incomingRoot = taskIncomingRoot(outputBaseDir, taskId);

    // P1-10：归档目录名归一到任务 peopleId。抓取到的 authorId 可能混有
    // 'unknown'/'anonymous'/真实 ID，若直接参与目录命名会产生多个作者目录，
    // publishTaskStage 要求恰好一个作者目录 → 单条抖动整任务 failed。
    // 目录名与 DB 身份统一用 task.author_id；authorName 仅作展示（任务级规范名）。
    let archiveAuthorName = getTask(taskId)?.author_name || task.author_name || '';

    // 全局速率预算：滑动窗口记录最近条目成败，用于自适应冷却与保护性中止
    const recentResults: boolean[] = [];
    let consumedCount = 0;
    const countTrailingFailures = (results: boolean[]): number => {
      let n = 0;
      for (let j = results.length - 1; j >= 0 && !results[j]; j--) n++;
      return n;
    };

    for (let i = 0; i < pendingItems.length; i++) {
      // 循环中首先检查状态是否被暂停/取消
      const stopped = checkTaskStopped(taskId);
      if (stopped) {
        emitProgress(taskId, stopped, stopped === 'paused' ? '任务已被用户手动暂停。' : '任务已被用户取消。');
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
        const stoppedBeforeRetry = checkTaskStopped(taskId);
        if (stoppedBeforeRetry) {
          emitProgress(taskId, stoppedBeforeRetry, stoppedBeforeRetry === 'paused' ? '任务在重试前被暂停。' : '任务在重试前被取消。');
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
            extracted = await scrapeAnswer(page, item.url, task.author_id);
          } else {
            extracted = await scrapeArticle(page, item.url, task.author_id);
          }

          // P1-10：归一归档身份到任务 peopleId。目录名/DB author_id 一律用任务目标，
          // 单条抓取回退（'unknown'/'anonymous'）或 API 返回的真实 ID 都不再分裂作者目录；
          // authorName 取任务级规范名（索引阶段已持久化），抓取值仅作展示兜底。
          if (!archiveAuthorName) {
            archiveAuthorName = extracted.authorName || '未知作者';
            saveTask({ id: taskId, author_name: archiveAuthorName });
          }
          const normalizedExtracted: typeof extracted = {
            ...extracted,
            authorId: task.author_id,
            authorName: archiveAuthorName,
          };

          const relativePath = await writeMarkdownFile(normalizedExtracted, incomingRoot, {
            id: task.author_id,
            name: archiveAuthorName,
          });
          
          // 更新数据库 items 属性缓存（例如最新的 voteup_count）
          saveItem({
            id: normalizedExtracted.id,
            item_type: normalizedExtracted.type,
            author_id: normalizedExtracted.authorId,
            author_name: normalizedExtracted.authorName,
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
          }, { recordArchive: false });

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
          } else if (errorMessage.includes('CONTENT_UNAVAILABLE')) {
            failureCode = 'CONTENT_UNAVAILABLE';
            retryCount = maxRetries + 1; // 内容已删除或不可见，重试不会恢复
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
          // 用户暂停/取消优先：不要把 cancelled/paused 覆盖成 failed（P1-9）。
          if (saveTaskStatusGuarded(taskId, 'failed')) {
            emitProgress(taskId, 'failed', `遇到登录障碍，任务终止: ${errorMessage}。请在沉浸阅读的知乎获取面板重新登录后再重试。`);
          }
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
        if (saveTaskStatusGuarded(taskId, 'failed')) {
          emitProgress(taskId, 'failed', '⛔ 最近 10 篇失败率过高，疑似触发站点风控，任务已保护性中止。请稍后用「重跑失败条目」恢复。');
        }
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
    // 发布前再检查一次：循环结束后用户仍可能暂停/取消，终态写入不得覆盖（P1-9）。
    const stoppedBeforeFinish = checkTaskStopped(taskId);
    if (stoppedBeforeFinish) {
      emitProgress(taskId, stoppedBeforeFinish, stoppedBeforeFinish === 'paused' ? '任务已暂停，终止于收尾前。' : '任务已取消，终止于收尾前。');
      return;
    }
    const finalTask = getTask(taskId);
    if (finalTask) {
      const isComplete = finalTask.success_count + finalTask.failed_count === finalTask.total_count;
      const status: Task['status'] = isComplete && finalTask.failed_count === 0
        ? 'success'
        : (isComplete && finalTask.success_count > 0 ? 'partial_success' : 'failed');

      if (isPublishableTaskStatus(status, finalTask.success_count)) {
        try {
          emitProgress(taskId, 'running', status === 'success'
            ? '正文抓取完成，正在发布到书架...'
            : `正文部分完成，正在把已成功的 ${finalTask.success_count} 篇发布到书架...`);
          const successfulItems = getTaskItems(taskId).filter(ti => ti.status === 'success');
          const authorName = successfulItems.find(item => item.author_name)?.author_name
            || task.author_name
            || task.author_id;
          const publishResult = publishTaskStage(outputBaseDir, taskId, task.author_id, {
            authorName,
            items: successfulItems,
          });
          const publishedItems = successfulItems
            .map(item => {
              let outputPath = resolvePublishedTaskItemPath(outputBaseDir, taskId, item.output_path);
              // P1-10：旧版本产生的多作者目录会被归并到规范目录，条目记录的旧目录名
              // 可能不再存在——章节文件一律平铺在作者目录根部，回退按文件名定位。
              if ((!outputPath || !fs.existsSync(outputPath)) && item.output_path) {
                const flat = path.join(publishResult.finalRoot, path.basename(item.output_path));
                if (fs.existsSync(flat)) {
                  outputPath = flat;
                }
              }
              if (!outputPath || !fs.existsSync(outputPath)) {
                throw new Error(`ZHIHU_PUBLISH_FAILED: published file missing for ${item.item_id}`);
              }
              return { ...item, output_path: outputPath, updated_at: Date.now() };
            });
          recordPublishedTaskItems(publishedItems);
          // 归档身份已归一到任务 peopleId：一本书一个作者目录，index.md 写入
          // 发布事务返回的真实目录，而不是按条目 author_id 重新推导（P1-10）。
          generateAuthorIndex(
            task.author_id,
            authorName,
            outputBaseDir,
            publishedItems,
            publishResult.authorDirectory,
          );
        } catch (err: any) {
          if (saveTaskStatusGuarded(taskId, 'failed')) {
            emitProgress(taskId, 'failed', `发布归档失败，旧成功版本保持不变: ${err.message}`);
          }
          return;
        }
      }

      // Only expose a terminal status after its publish transaction is durable. The desktop
      // poller treats terminal as final and refreshes the shelf immediately.
      // 若收尾期间用户暂停/取消，则保持该状态（P1-9：取消是终态，不得覆盖）。
      if (checkTaskStopped(taskId)) {
        emitProgress(taskId, checkTaskStopped(taskId) || 'cancelled', '任务在收尾期间被暂停或取消，保持用户选择的状态。');
      } else {
        saveTask({ id: taskId, status });
        emitProgress(taskId, status, `任务执行完毕。总数: ${finalTask.total_count}, 成功: ${finalTask.success_count}, 失败: ${finalTask.failed_count}`);
      }
    }

  } catch (e: any) {
    logger.error(`任务执行过程严重崩溃: ${e.message}`);
    if (saveTaskStatusGuarded(taskId, 'failed')) {
      emitProgress(taskId, 'failed', `严重错误导致任务异常中止: ${e.message}`);
    }
  } finally {
    await closeBrowserContext();
  }
}

/**
 * 辅助方法：检查数据库状态是否要求停止（暂停或取消）。
 * 任务行被删除同样视为停止信号，避免对幽灵任务继续抓取。
 */
function checkTaskStopped(taskId: string): 'paused' | 'cancelled' | null {
  const task = getTask(taskId);
  if (!task) return 'cancelled';
  return task.status === 'paused' || task.status === 'cancelled' ? task.status : null;
}

/**
 * 工作器驱动的状态写入：用户暂停/取消优先，返回 false 表示未写入。
 * 防止循环外层的失败/完成写入把 cancelled/paused 覆盖回 failed（P1-9）。
 */
function saveTaskStatusGuarded(taskId: string, status: Task['status']): boolean {
  if (checkTaskStopped(taskId)) return false;
  saveTask({ id: taskId, status });
  return true;
}

/**
 * 为答主目录生成 Obsidian 双链导航索引 index.md
 * @param authorDirectory 可选：发布事务实际落地的作者目录名。传入后 index.md
 *   直接写进该目录，而不是按 authorName+authorId 重新推导（P1-10：推导名可能与
 *   归一化后的真实目录不一致）。
 */
export function generateAuthorIndex(
  authorId: string,
  authorName: string,
  outputBaseDir: string,
  publishedItems?: readonly (ReturnType<typeof getTaskItems>[number])[],
  authorDirectory?: string,
) {
  const items = publishedItems || getAuthorSuccessItems(authorId);
  if (items.length === 0) return;

  const authorDirName = authorDirectory || sanitizeFilename(authorName, authorId);
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
