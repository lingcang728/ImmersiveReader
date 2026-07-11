import { Page } from 'playwright-core';
import * as fs from 'fs';
import * as path from 'path';
import { logger } from './utils.js';
import { normalizeUrl } from './extractor.js';

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
    throw new Error('LOGIN_REQUIRED: 无头浏览器未登录（缺少 z_c0 cookie），知乎已不再向未登录客户端返回主页内容。请先运行 npm run login 完成扫码登录后重试。');
  }
  throw new Error('CAPTCHA_REQUIRED: 触发知乎反爬（zse-ck）拦截，等待后仍未渲染出内容。请运行 npm run login 在有头浏览器中完成人机验证后重试。');
}

export async function scrapePeopleIndex(
  page: Page,
  peopleId: string,
  itemType: 'answers' | 'articles',
  topN: number | null = null
): Promise<ScrapedIndexItem[]> {
  const targetUrl = `https://www.zhihu.com/people/${peopleId}/${itemType}`;
  logger.info(`开始扫描答主 ${peopleId} 的 ${itemType} 列表, 目标 URL: ${targetUrl}`);

  const collected: Map<string, ScrapedIndexItem> = new Map();

  // 1. 监听 API 请求拦截
  const handleResponse = async (response: any) => {
    const url = response.url();
    // 拦截 answers API
    if (itemType === 'answers' && url.includes(`/members/${peopleId}/answers`)) {
      try {
        const json = await response.json();
        if (json && Array.isArray(json.data)) {
          for (const item of json.data) {
            if (item.type !== 'answer') continue;
            const answerId = String(item.id);
            const itemId = `answer:${answerId}`;
            
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
          }
        }
      } catch (e) {
        // ignore
      }
    }

    // 拦截 articles API
    if (itemType === 'articles' && (url.includes(`/members/${peopleId}/articles`) || url.includes(`/members/${peopleId}/posts`))) {
      try {
        const json = await response.json();
        if (json && Array.isArray(json.data)) {
          for (const item of json.data) {
            if (item.type !== 'article') continue;
            const articleId = String(item.id);
            const itemId = `article:${articleId}`;
            
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
          }
        }
      } catch (e) {
        // ignore
      }
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
    throw new Error('LOGIN_REQUIRED: 访问个人主页需要登录态，请在终端运行 npm run login 登录。');
  }
  if (currentUrl.includes('unhuman') || currentUrl.includes('captcha')) {
    throw new Error('CAPTCHA_REQUIRED: 访问主页触发了人机验证，请运行 npm run login 完成人机验证。');
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

  // 2. 模拟向下滚动滚动，触发 API 列表拉取
  let isEnd = false;
  let lastSize = 0;
  let noNewCount = 0;
  const maxScrolls = 200; // 防死循环
  let scrollCount = 0;

  while (!isEnd && scrollCount < maxScrolls) {
    // 滚动到底部
    await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
    await page.waitForTimeout(2000 + Math.random() * 1000); // 随机等待2~3秒

    scrollCount++;

    // 检查是否有“没有更多”提示
    const hasNoMore = await page.evaluate(() => {
      const texts = ['没有更多了', '已加载完毕', '暂无内容', '没有更多内容'];
      // 搜索页面文本
      const bodyText = document.body.textContent || '';
      return texts.some(t => bodyText.includes(t));
    });

    if (hasNoMore) {
      logger.info('触发页面“没有更多”占位符，停止滚动。');
      break;
    }

    const currentSize = collected.size;
    logger.info(`已拉取到索引条目数: ${currentSize}`);

    if (topN && currentSize >= topN) {
      logger.info(`已达到设定的 Top N (${topN}) 数量，停止滚动。`);
      break;
    }

    if (currentSize === lastSize) {
      noNewCount++;
      if (noNewCount >= 8) {
        logger.info('连续 8 次滚动未拉取到新条目，停止滚动。');
        break;
      }
    } else {
      noNewCount = 0;
      lastSize = currentSize;
    }
  }

  page.off('response', handleResponse);

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
    try {
      const debugPath = path.resolve(process.cwd(), `debug-people-${itemType}.html`);
      fs.writeFileSync(debugPath, await page.content(), 'utf-8');
      logger.error(`未发现任何条目，已保存调试页面快照至: ${debugPath}（可据此判断是否为登录墙 / 风控空页）`);
    } catch {
      // ignore
    }
  }

  logger.info(`扫描答主 ${peopleId} 列表结束，共发现 ${result.length} 条有效内容`);
  return result;
}
