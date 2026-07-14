import { Page } from 'playwright-core';
import * as fs from 'fs';
import * as path from 'path';
import { logger } from './utils.js';
import { normalizeUrl } from './extractor.js';
import { resolveBrowserCacheDir } from './runtime-paths.js';

export interface ScrapedIndexItem {
  id: string; // answer:12345 或 article:67890
  type: 'answer' | 'article';
  title: string;
  authorId: string;
  authorName: string;
  url: string;
  createdTime: number;
  updatedTime: number;
  voteupCount: number;
  commentCount: number;
  // 回答专属
  questionId?: string;
  questionUrl?: string;
}

export function selectIndexItems(
  items: readonly ScrapedIndexItem[],
  topN: number | null,
  sortBy: 'time' | 'vote'
): ScrapedIndexItem[] {
  const sorted = [...items].sort((a, b) => {
    const primary =
      sortBy === 'vote' ? b.voteupCount - a.voteupCount : b.createdTime - a.createdTime;
    if (primary !== 0) return primary;
    const secondary =
      sortBy === 'vote' ? b.createdTime - a.createdTime : b.voteupCount - a.voteupCount;
    if (secondary !== 0) return secondary;
    return a.id.localeCompare(b.id);
  });
  return topN === null ? sorted : sorted.slice(0, Math.max(0, topN));
}

export function scanLimitForSelection(
  topN: number | null,
  sortBy: 'time' | 'vote'
): number | null {
  return sortBy === 'time' && topN !== null ? Math.max(0, topN) : null;
}

/** Zhihu list API paging payload (snake_case as returned by the site). */
export type ZhihuListPaging = {
  is_end?: boolean;
  next?: string | null;
  totals?: number;
};

export type ListPagingState = {
  isEnd: boolean;
  next: string | null;
  totals: number | null;
  pagesSeen: number;
  lastCursor: string | null;
};

export function emptyPagingState(): ListPagingState {
  return { isEnd: false, next: null, totals: null, pagesSeen: 0, lastCursor: null };
}

/**
 * Merge one authenticated list-API page into the collected index map and update paging.
 * Returns how many *new* items were added. Detects repeated cursors as incomplete pagination.
 */
export function mergeListApiPage(
  itemType: 'answers' | 'articles',
  peopleId: string,
  json: { data?: unknown[]; paging?: ZhihuListPaging } | null | undefined,
  collected: Map<string, ScrapedIndexItem>,
  paging: ListPagingState
): { added: number; repeatedCursor: boolean } {
  if (!json || !Array.isArray(json.data)) {
    return { added: 0, repeatedCursor: false };
  }

  let added = 0;
  for (const raw of json.data) {
    const item = raw as Record<string, any>;
    if (itemType === 'answers') {
      if (item.type !== 'answer') continue;
      const answerId = String(item.id);
      const itemId = `answer:${answerId}`;
      if (collected.has(itemId)) continue;
      const questionId = String(item.question?.id || '');
      collected.set(itemId, {
        id: itemId,
        type: 'answer',
        title: item.question?.title || '未命名问题',
        authorId: item.author?.id || peopleId,
        authorName: item.author?.name || '未知作者',
        url: `https://www.zhihu.com/question/${questionId}/answer/${answerId}`,
        createdTime: item.created_time || 0,
        updatedTime: item.updated_time || 0,
        voteupCount: item.voteup_count || 0,
        commentCount: item.comment_count || 0,
        questionId,
        questionUrl: `https://www.zhihu.com/question/${questionId}`
      });
      added++;
    } else {
      if (item.type !== 'article') continue;
      const articleId = String(item.id);
      const itemId = `article:${articleId}`;
      if (collected.has(itemId)) continue;
      collected.set(itemId, {
        id: itemId,
        type: 'article',
        title: item.title || '未命名文章',
        authorId: item.author?.id || peopleId,
        authorName: item.author?.name || '未知作者',
        url: `https://zhuanlan.zhihu.com/p/${articleId}`,
        createdTime: item.created || item.created_time || 0,
        updatedTime: item.updated || item.updated_time || 0,
        voteupCount: item.voteup_count || item.likes_count || 0,
        commentCount: item.comment_count || 0
      });
      added++;
    }
  }

  const pagingPayload = json.paging || {};
  const next =
    typeof pagingPayload.next === 'string' && pagingPayload.next.trim()
      ? pagingPayload.next.trim()
      : null;
  let repeatedCursor = false;
  if (next && paging.lastCursor && next === paging.lastCursor) {
    repeatedCursor = true;
  }
  if (next) {
    paging.lastCursor = next;
  }
  paging.next = next;
  paging.pagesSeen += 1;
  if (typeof pagingPayload.totals === 'number' && Number.isFinite(pagingPayload.totals)) {
    paging.totals = Math.max(0, Math.floor(pagingPayload.totals));
  }
  if (pagingPayload.is_end === true) {
    paging.isEnd = true;
  } else if (pagingPayload.is_end === false) {
    paging.isEnd = false;
  }

  return { added, repeatedCursor };
}

/**
 * Decide whether index scrolling should stop. Priority:
 * 1) Top N reached  2) API is_end  3) API totals reached
 * 4) DOM no-more text  5) repeated cursor  6) no-new streak  7) max scrolls
 */
export function shouldStopIndexScroll(input: {
  topN: number | null;
  collectedSize: number;
  paging: ListPagingState;
  hasDomNoMore: boolean;
  noNewCount: number;
  scrollCount: number;
  maxScrolls: number;
  repeatedCursor: boolean;
  noNewLimit?: number;
}): { stop: boolean; reason: string | null } {
  const noNewLimit = input.noNewLimit ?? 8;
  if (input.topN !== null && input.collectedSize >= input.topN) {
    return { stop: true, reason: `top_n:${input.topN}` };
  }
  if (input.paging.isEnd) {
    return { stop: true, reason: 'api_is_end' };
  }
  if (
    input.paging.totals !== null &&
    input.paging.totals > 0 &&
    input.collectedSize >= input.paging.totals
  ) {
    return { stop: true, reason: `api_totals:${input.paging.totals}` };
  }
  if (input.hasDomNoMore) {
    return { stop: true, reason: 'dom_no_more' };
  }
  if (input.repeatedCursor) {
    return { stop: true, reason: 'repeated_cursor' };
  }
  if (input.noNewCount >= noNewLimit) {
    return { stop: true, reason: `no_new:${noNewLimit}` };
  }
  if (input.scrollCount >= input.maxScrolls) {
    return { stop: true, reason: `max_scrolls:${input.maxScrolls}` };
  }
  return { stop: false, reason: null };
}

/**
 * 从页面 SSR 注入的 #js-initialData (Hydration JSON) 中解析首屏的回答/文章索引。
 *
 * 这是最稳健的数据来源：即便在无头环境下 React 没有把 .ContentItem 卡片渲染出来、
 * 或者滚动分页的 XHR 没有被触发，首屏那一页的数据依然完整地内嵌在该 <script> 标签里。
 * extractor.ts 抓取单篇时已经依赖这一兜底，但 indexer 之前漏掉了，导致首屏内容明明存在
 * 却被判定为「0 条」。
 */
async function collectIndexFromInitialData(
  page: Page,
  peopleId: string,
  itemType: 'answers' | 'articles',
  collected: Map<string, ScrapedIndexItem>
): Promise<number> {
  let raw: string | null = null;
  try {
    raw = await page.$eval('#js-initialData', el => el.textContent);
  } catch {
    return 0; // 页面里没有该 script
  }
  if (!raw) return 0;

  let hydration: any;
  try {
    hydration = JSON.parse(raw);
  } catch {
    return 0;
  }

  const entities = hydration?.initialState?.entities;
  if (!entities) return 0;
  const users = entities.users || {};
  const questions = entities.questions || {};

  // 解析作者：兼容 users 既可能以 id、也可能以 urlToken 为键的两种结构。
  const resolveAuthor = (authorObj: any): { id: string; name: string; token: string | null } => {
    const u = (authorObj && (users[authorObj.id] || users[authorObj.urlToken])) || {};
    return {
      id: authorObj?.id || u.id || peopleId,
      name: u.name || authorObj?.name || '未知作者',
      token: u.urlToken || u.url_token || authorObj?.urlToken || authorObj?.url_token || null
    };
  };

  let added = 0;

  if (itemType === 'answers') {
    const answers = entities.answers || {};
    for (const key of Object.keys(answers)) {
      const a = answers[key];
      if (!a || !a.id) continue;

      const author = resolveAuthor(a.author);
      // 仅当能「明确判定」这是别的作者时才跳过，避免字段差异误伤导致结果为空。
      if (author.token && author.token !== peopleId) continue;

      const answerId = String(a.id);
      const questionId = String(a.question?.id || a.questionId || '');
      if (!questionId) continue; // 没有问题 ID 无法拼出有效的回答 URL

      const itemId = `answer:${answerId}`;
      if (collected.has(itemId)) continue;

      const title = a.question?.title || questions[questionId]?.title || '未命名问题';

      collected.set(itemId, {
        id: itemId,
        type: 'answer',
        title,
        authorId: author.id,
        authorName: author.name,
        url: `https://www.zhihu.com/question/${questionId}/answer/${answerId}`,
        createdTime: a.createdTime || a.created_time || 0,
        updatedTime: a.updatedTime || a.updated_time || 0,
        voteupCount: a.voteupCount || a.voteup_count || 0,
        commentCount: a.commentCount || a.comment_count || 0,
        questionId,
        questionUrl: `https://www.zhihu.com/question/${questionId}`
      });
      added++;
    }
  } else {
    const articles = entities.articles || entities.posts || {};
    for (const key of Object.keys(articles)) {
      const a = articles[key];
      if (!a || !a.id) continue;

      const author = resolveAuthor(a.author);
      if (author.token && author.token !== peopleId) continue;

      const articleId = String(a.id);
      const itemId = `article:${articleId}`;
      if (collected.has(itemId)) continue;

      collected.set(itemId, {
        id: itemId,
        type: 'article',
        title: a.title || '未命名文章',
        authorId: author.id,
        authorName: author.name,
        url: `https://zhuanlan.zhihu.com/p/${articleId}`,
        createdTime: a.createdTime || a.created || a.created_time || 0,
        updatedTime: a.updatedTime || a.updated || a.updated_time || 0,
        voteupCount: a.voteupCount || a.voteup_count || a.likesCount || 0,
        commentCount: a.commentCount || a.comment_count || 0
      });
      added++;
    }
  }

  return added;
}

/**
 * 等待页面通过知乎反爬（zse-ck）质询并真正渲染出内容。
 *
 * 知乎对未受信任的客户端会先返回一个极小的 zse-ck 质询页（仅含 <meta id="zh-zse-ck">
 * 和一段 challenge JS），它不会跳转到 signin/unhuman，所以 URL 层面的判断抓不到，
 * 表现就是「页面在，但一条内容都没有」。这里给 challenge JS 留出执行时间并在必要时
 * reload 一次；若最终仍拿不到内容，则依据是否登录给出明确、可执行的报错，
 * 而不是误报「没有发现任何回答或文章」。
 */
async function waitForRealContent(page: Page): Promise<void> {
  const deadline = Date.now() + 20000;
  let reloaded = false;

  while (Date.now() < deadline) {
    const state = await page.evaluate(() => ({
      hasInitial: !!document.querySelector('#js-initialData'),
      cards: document.querySelectorAll('.ContentItem').length,
      challenge: !!document.querySelector('#zh-zse-ck') || !!document.querySelector('script[src*="zse-ck"]')
    }));

    if (state.hasInitial || state.cards > 0) {
      return; // 真实内容已就绪
    }

    if (state.challenge && !reloaded) {
      // 质询页：先让 challenge JS 执行拿到 __zse_ck，再 reload 一次以获取真实页面
      reloaded = true;
      await page.waitForTimeout(2500);
      await page.reload({ waitUntil: 'domcontentloaded' }).catch(() => {});
      await page.waitForTimeout(1500);
      continue;
    }

    await page.waitForTimeout(1500);
  }

  // 超时仍无内容：判定根因，给出可执行建议。
  const cookies = await page.context().cookies();
  const loggedIn = cookies.some(c => c.name === 'z_c0');
  if (!loggedIn) {
    throw new Error('LOGIN_REQUIRED: 无头浏览器未登录（缺少 z_c0 cookie），知乎已不再向未登录客户端返回主页内容。请在沉浸阅读的知乎获取面板登录后重试。');
  }
  throw new Error('CAPTCHA_REQUIRED: 触发知乎反爬（zse-ck）拦截，等待后仍未渲染出内容。请在沉浸阅读的知乎获取面板完成人机验证后重试。');
}

export async function scrapePeopleIndex(
  page: Page,
  peopleId: string,
  itemType: 'answers' | 'articles',
  topN: number | null = null
): Promise<ScrapedIndexItem[]> {
  const indexPath = itemType === 'articles' ? 'posts' : itemType;
  const targetUrl = `https://www.zhihu.com/people/${peopleId}/${indexPath}`;
  logger.info(`开始扫描答主 ${peopleId} 的 ${itemType} 列表, 目标 URL: ${targetUrl}`);

  const collected: Map<string, ScrapedIndexItem> = new Map();
  const pagingState = emptyPagingState();
  let repeatedCursor = false;

  // 1. 监听 API 请求拦截 — 以 paging.is_end / next / totals 为完成主依据
  const handleResponse = async (response: any) => {
    const url = response.url();
    const isAnswers =
      itemType === 'answers' && url.includes(`/members/${peopleId}/answers`);
    const isArticles =
      itemType === 'articles' &&
      (url.includes(`/members/${peopleId}/articles`) ||
        url.includes(`/members/${peopleId}/posts`));
    if (!isAnswers && !isArticles) return;
    try {
      const json = await response.json();
      const merged = mergeListApiPage(itemType, peopleId, json, collected, pagingState);
      if (merged.repeatedCursor) {
        repeatedCursor = true;
      }
      if (merged.added > 0 || pagingState.isEnd) {
        logger.info(
          `列表 API 页 #${pagingState.pagesSeen}: +${merged.added} 条，累计 ${collected.size}` +
            (pagingState.totals !== null ? ` / API total ${pagingState.totals}` : '') +
            (pagingState.isEnd ? '，is_end=true' : '')
        );
      }
    } catch {
      // ignore non-JSON or aborted responses
    }
  };

  page.on('response', handleResponse);

  try {
    await page.goto(targetUrl, { waitUntil: 'domcontentloaded', timeout: 30000 });
    await page.waitForTimeout(1500);
  } catch (e: any) {
    logger.warn(`主页加载超时或出错: ${e.message}`);
  }

  const currentUrl = page.url();
  if (currentUrl.includes('signin')) {
    throw new Error('LOGIN_REQUIRED: 访问个人主页需要登录态，请在沉浸阅读的知乎获取面板登录。');
  }
  if (currentUrl.includes('unhuman') || currentUrl.includes('captcha')) {
    throw new Error('CAPTCHA_REQUIRED: 访问主页触发了人机验证，请在沉浸阅读的知乎获取面板完成人机验证。');
  }

  // 1.4 等待知乎反爬（zse-ck）质询通过并确认渲染出真实内容；
  //     若超时仍为空，会根据是否登录抛出 LOGIN_REQUIRED / CAPTCHA_REQUIRED，避免误报「没有内容」。
  await waitForRealContent(page);

  // 1.5 首屏 Hydration 解析：优先从 SSR 注入的 #js-initialData 中解析首页索引。
  // 这一步比依赖 React 渲染出的 DOM 卡片、或滚动分页 XHR 更可靠，是修复「明明有内容却 0 条」的关键。
  const seeded = await collectIndexFromInitialData(page, peopleId, itemType, collected);
  if (seeded > 0) {
    logger.info(`已从首屏 js-initialData 解析到 ${seeded} 条索引。`);
  } else {
    logger.info('首屏 js-initialData 未解析到索引，将依赖滚动分页拦截与 DOM 兜底。');
  }

  // 2. 滚动触发列表分页。完成依据优先 API paging.is_end / totals；
  //    DOM「没有更多」与连续无新增仅作异常兜底，不再作为唯一完成信号。
  let lastSize = collected.size;
  let noNewCount = 0;
  const maxScrolls = 200;
  let scrollCount = 0;
  let stopReason: string | null = null;

  // 首屏 API 可能已在 goto 时拦截到 is_end / totals
  {
    const early = shouldStopIndexScroll({
      topN,
      collectedSize: collected.size,
      paging: pagingState,
      hasDomNoMore: false,
      noNewCount: 0,
      scrollCount: 0,
      maxScrolls,
      repeatedCursor
    });
    if (early.stop) {
      stopReason = early.reason;
      logger.info(`索引在首屏即满足结束条件: ${stopReason}`);
    }
  }

  while (!stopReason && scrollCount < maxScrolls) {
    await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
    await page.waitForTimeout(2000 + Math.random() * 1000);
    scrollCount++;

    const hasNoMore = await page.evaluate(() => {
      const texts = ['没有更多了', '已加载完毕', '暂无内容', '没有更多内容'];
      const bodyText = document.body.textContent || '';
      return texts.some(t => bodyText.includes(t));
    });

    const currentSize = collected.size;
    logger.info(
      `滚动 #${scrollCount}: 已发现 ${currentSize}` +
        (pagingState.totals !== null ? ` / API ${pagingState.totals}` : '') +
        (pagingState.isEnd ? ' (is_end)' : '')
    );

    if (currentSize === lastSize) {
      noNewCount++;
    } else {
      noNewCount = 0;
      lastSize = currentSize;
    }

    const decision = shouldStopIndexScroll({
      topN,
      collectedSize: currentSize,
      paging: pagingState,
      hasDomNoMore: hasNoMore,
      noNewCount,
      scrollCount,
      maxScrolls,
      repeatedCursor
    });
    if (decision.stop) {
      stopReason = decision.reason;
      logger.info(`停止滚动: ${stopReason}`);
      break;
    }
  }

  if (!stopReason && scrollCount >= maxScrolls) {
    stopReason = `max_scrolls:${maxScrolls}`;
  }

  page.off('response', handleResponse);

  // Incomplete pagination: saw API pages, never got is_end, and stopped only on weak signals.
  const weakStop =
    stopReason?.startsWith('no_new:') ||
    stopReason?.startsWith('max_scrolls:') ||
    stopReason === 'repeated_cursor';
  if (
    pagingState.pagesSeen > 0 &&
    !pagingState.isEnd &&
    topN === null &&
    weakStop &&
    (pagingState.totals === null || collected.size < pagingState.totals)
  ) {
    throw new Error(
      `PAGINATION_INCOMPLETE: 列表分页未明确结束（stop=${stopReason}, discovered=${collected.size}` +
        (pagingState.totals !== null ? `, apiTotals=${pagingState.totals}` : '') +
        ', pages=' +
        pagingState.pagesSeen +
        '）。请重试索引，勿将截断结果当作成功。'
    );
  }
  if (pagingState.totals !== null && collected.size < pagingState.totals && topN === null) {
    logger.warn(
      `索引数量 ${collected.size} 少于 API totals ${pagingState.totals}（stop=${stopReason}）。` +
        '主页展示数可能含删除/折叠/仅本人可见内容，将以 API 可见集合为准。'
    );
  }

  // 3. DOM 兜底/补充：把页面上已渲染（含滚动加载）的卡片再解析一遍。
  //    使用 Map 去重，因此即便与首屏 Hydration / API 拦截的结果重叠也不会重复计入。
  const beforeDomSize = collected.size;
  {
    try {
      const domItems = await page.evaluate((type) => {
        const results: any[] = [];
        const cards = document.querySelectorAll('.ContentItem');
        
        cards.forEach(card => {
          const zopStr = card.getAttribute('data-zop') || '{}';
          let zop: any = {};
          try { zop = JSON.parse(zopStr); } catch (e) {}

          const metaUrl = card.querySelector('meta[itemprop="url"]')?.getAttribute('content') || '';
          const metaCreated = card.querySelector('meta[itemprop="dateCreated"]')?.getAttribute('content') || '';
          const metaModified = card.querySelector('meta[itemprop="dateModified"]')?.getAttribute('content') || '';
          const metaLikes = card.querySelector('meta[itemprop="upvoteCount"]')?.getAttribute('content') || '0';
          const metaComments = card.querySelector('meta[itemprop="commentCount"]')?.getAttribute('content') || '0';

          if (!metaUrl) return;

          results.push({
            title: zop.title || card.querySelector('.ContentItem-title')?.textContent?.trim() || '未命名',
            authorName: zop.authorName || '未知作者',
            url: metaUrl,
            createdTime: metaCreated ? Math.floor(new Date(metaCreated).getTime() / 1000) : 0,
            updatedTime: metaModified ? Math.floor(new Date(metaModified).getTime() / 1000) : 0,
            voteupCount: parseInt(metaLikes, 10) || 0,
            commentCount: parseInt(metaComments, 10) || 0
          });
        });
        return results;
      }, itemType);

      for (const raw of domItems) {
        const norm = normalizeUrl(raw.url);
        if (norm.type === 'unknown') continue;

        const itemId = norm.id;
        const rawId = itemId.split(':')[1];
        if (norm.type === 'answer') {
          collected.set(itemId, {
            id: itemId,
            type: 'answer',
            title: raw.title,
            authorId: peopleId,
            authorName: raw.authorName,
            url: norm.normalizedUrl,
            createdTime: raw.createdTime,
            updatedTime: raw.updatedTime,
            voteupCount: raw.voteupCount,
            commentCount: raw.commentCount,
            questionId: norm.questionId,
            questionUrl: `https://www.zhihu.com/question/${norm.questionId}`
          });
        } else if (norm.type === 'article') {
          collected.set(itemId, {
            id: itemId,
            type: 'article',
            title: raw.title,
            authorId: peopleId,
            authorName: raw.authorName,
            url: norm.normalizedUrl,
            createdTime: raw.createdTime,
            updatedTime: raw.updatedTime,
            voteupCount: raw.voteupCount,
            commentCount: raw.commentCount
          });
        }
      }
    } catch (e: any) {
      logger.error(`从 DOM 解析索引时出错: ${e.message}`);
    }
  }
  const domAdded = collected.size - beforeDomSize;
  if (domAdded > 0) {
    logger.info(`从 DOM 中补充解析到 ${domAdded} 条索引。`);
  }

  // 4. 返回最终结果并转换成数组
  let result = Array.from(collected.values());
  if (topN) {
    result = result.slice(0, topN);
  }

  // 若三种来源（首屏 Hydration / API 拦截 / DOM）都没拿到任何条目，
  // 保存当前页面 HTML 快照以便排查（通常意味着登录态失效的登录墙，或风控返回的空页）。
  if (result.length === 0) {
    const accessState = await page.evaluate((type) => {
      const bodyText = document.body.textContent || '';
      const emptyTexts = type === 'answers'
        ? ['还没有回答', '暂无回答']
        : ['还没有文章', '暂无文章'];
      return {
        hasLoginWall: !!document.querySelector('.signFlowModal, .SignFlow, input[name="username"], button.SignFlow-submitButton')
          || bodyText.includes('立即登录/注册')
          || bodyText.includes('登录知乎，您可以享受'),
        hasChallenge: !!document.querySelector('#zh-zse-ck, script[src*="zse-ck"]')
          || window.location.href.includes('unhuman')
          || window.location.href.includes('captcha'),
        hasExplicitEmpty: emptyTexts.some(text => bodyText.includes(text))
      };
    }, itemType);
    if (accessState.hasLoginWall) {
      throw new Error('LOGIN_REQUIRED: 当前知乎会话显示登录页面，登录态缺失或已过期。请在沉浸阅读的知乎获取面板重新登录后重试。');
    }
    const cookies = await page.context().cookies();
    const loggedIn = cookies.some(cookie => cookie.name === 'z_c0');
    if (!loggedIn) {
      throw new Error('LOGIN_REQUIRED: 无头浏览器未登录（缺少 z_c0 cookie），知乎未返回任何可归档内容。请在沉浸阅读的知乎获取面板登录后重试。');
    }
    if (accessState.hasExplicitEmpty) {
      logger.info(`知乎明确返回 ${itemType} 空状态。`);
      return [];
    }
    try {
      const debugRoot = resolveBrowserCacheDir({ cwd: process.cwd(), environment: process.env });
      fs.mkdirSync(debugRoot, { recursive: true });
      const debugPath = path.join(debugRoot, `debug-people-${itemType}.html`);
      fs.writeFileSync(debugPath, await page.content(), 'utf-8');
      logger.error(`未发现任何条目，已保存调试页面快照至: ${debugPath}（可据此判断是否为登录墙 / 风控空页）`);
    } catch {
      // ignore
    }
    throw new Error(accessState.hasChallenge
      ? 'CAPTCHA_REQUIRED: 知乎返回了人机验证页面，请在沉浸阅读的知乎获取面板完成验证后重试。'
      : 'CAPTCHA_REQUIRED: 登录态存在，但知乎未返回可验证的内容索引；请在沉浸阅读的知乎获取面板用有头浏览器确认页面后重试。');
  }

  logger.info(`扫描答主 ${peopleId} 列表结束，共发现 ${result.length} 条有效内容`);
  return result;
}
