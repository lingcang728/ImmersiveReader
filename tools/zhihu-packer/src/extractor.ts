import { Page } from 'playwright-core';
import { lookup } from 'node:dns/promises';
import * as path from 'path';
import * as fs from 'fs';
import { createHash } from 'crypto';
import { isIP } from 'node:net';
import { request } from 'node:https';
import { pipeline } from 'node:stream/promises';
import { Transform } from 'node:stream';
import { logger, sanitizeFilename, evaluateClean } from './utils.js';

export interface ExtractedContent {
  id: string; // answer:12345 或 article:67890
  type: 'answer' | 'article';
  title: string;
  authorId: string;
  authorName: string;
  contentHtml: string;
  contentMarkdown: string;
  createdTime: number; // 秒级时间戳
  updatedTime: number;
  voteupCount: number;
  commentCount: number;
  url: string;
  // 以下回答专属
  answerId?: string;
  questionId?: string;
  questionUrl?: string;
  // 以下文章专属
  articleId?: string;
}

export function isUnavailableZhihuPage(title: string, bodyText: string): boolean {
  return `${title}\n${bodyText}`.includes('没有知识存在的荒原');
}

async function assertZhihuContentAvailable(page: Page): Promise<void> {
  const snapshot = await page.evaluate(() => ({
    title: document.title || '',
    bodyText: document.body?.innerText || ''
  })).catch(() => ({ title: '', bodyText: '' }));

  if (isUnavailableZhihuPage(snapshot.title, snapshot.bodyText)) {
    throw new Error('CONTENT_UNAVAILABLE: 知乎内容已删除或当前账号不可见');
  }
}

function escapeHtmlText(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

export function normalizeUrl(urlStr: string): { type: 'answer' | 'article' | 'unknown', id: string, normalizedUrl: string, questionId?: string } {
  try {
    const url = new URL(urlStr);
    
    // 1. 回答：/question/{qId}/answer/{aId}
    const answerMatch = url.pathname.match(/\/question\/(\d+)\/answer\/(\d+)/);
    if (answerMatch) {
      const questionId = answerMatch[1];
      const answerId = answerMatch[2];
      return {
        type: 'answer',
        id: `answer:${answerId}`,
        normalizedUrl: `https://www.zhihu.com/question/${questionId}/answer/${answerId}`,
        questionId
      };
    }

    // 2. 文章：/p/{articleId}
    const articleMatch = url.pathname.match(/\/p\/(\d+)/);
    if (articleMatch && (url.hostname.includes('zhuanlan') || url.hostname.includes('zhihu.com'))) {
      const articleId = articleMatch[1];
      return {
        type: 'article',
        id: `article:${articleId}`,
        normalizedUrl: `https://zhuanlan.zhihu.com/p/${articleId}`
      };
    }
  } catch (e) {
    // ignore
  }

  // 兜底正则
  const answerReg = /\/question\/(\d+)\/answer\/(\d+)/;
  const m1 = urlStr.match(answerReg);
  if (m1) {
    return {
      type: 'answer',
      id: `answer:${m1[2]}`,
      normalizedUrl: `https://www.zhihu.com/question/${m1[1]}/answer/${m1[2]}`,
      questionId: m1[1]
    };
  }

  const articleReg = /\/p\/(\d+)/;
  const m2 = urlStr.match(articleReg);
  if (m2) {
    return {
      type: 'article',
      id: `article:${m2[1]}`,
      normalizedUrl: `https://zhuanlan.zhihu.com/p/${m2[1]}`
    };
  }

  return {
    type: 'unknown',
    id: '',
    normalizedUrl: urlStr
  };
}

/**
 * 在浏览器沙箱内将富文本 HTML 转换为 Markdown。
 * 包含公式转 LaTeX、剔除图片、卡片转链接、剔除无关元素。
 */
export async function convertHtmlToMarkdown(page: Page, html: string): Promise<string> {
  return evaluateClean<string>(page, (htmlContent) => {
    const container = document.createElement('div');
    container.innerHTML = htmlContent;

    // 1. 处理公式 span.ztext-math
    const maths = container.querySelectorAll('span.ztext-math');
    maths.forEach(math => {
      const tex = math.getAttribute('data-tex');
      if (tex) {
        // 区分块公式和行内公式
        const isBlock = math.classList.contains('ztext-math-block') || 
                        (math.parentElement?.tagName === 'P' && math.parentElement.childNodes.length === 1);
        const placeholder = isBlock ? `\n\n$$\n${tex}\n$$\n\n` : ` $${tex}$ `;
        const textNode = document.createTextNode(placeholder);
        math.parentNode?.replaceChild(textNode, math);
      }
    });

    // 2. 图片转为 Markdown 引用（Node 侧随后下载到本地 assets/，见 archiveImagesLocally）
    // 先移除 noscript（其中是 lazy 图片的重复副本，且内容会被当作纯文本泄漏进正文）
    container.querySelectorAll('noscript').forEach(n => n.remove());
    container.querySelectorAll('img').forEach(img => {
      const candidate =
        img.getAttribute('data-original') ||
        img.getAttribute('data-actualsrc') ||
        img.getAttribute('data-src') ||
        img.getAttribute('src') ||
        '';
      const url = candidate.trim();
      if (!url || url.startsWith('data:')) {
        img.remove();
        return;
      }
      const textNode = document.createTextNode(`\n\n![](${url})\n\n`);
      img.parentNode?.replaceChild(textNode, img);
    });

    // 3. 处理知乎卡片
    const linkCards = container.querySelectorAll('a.LinkCard, a.MCNLinkCard, a[data-draft-node="zhihu-link-card"]');
    linkCards.forEach(card => {
      const href = card.getAttribute('href') || '';
      const titleEl = card.querySelector('.LinkCard-title, .MCNLinkCard-title') || card;
      const title = titleEl.textContent?.trim() || href;
      const placeholder = ` [${title}](${href}) `;
      const textNode = document.createTextNode(placeholder);
      card.parentNode?.replaceChild(textNode, card);
    });

    // 4. 处理普通链接
    const links = container.querySelectorAll('a');
    links.forEach(link => {
      // 避免重复处理已经被转为卡片的链接
      if (link.parentNode === null) return; 
      const href = link.getAttribute('href') || '';
      const text = link.textContent?.trim() || href;
      const placeholder = ` [${text}](${href}) `;
      const textNode = document.createTextNode(placeholder);
      link.parentNode?.replaceChild(textNode, link);
    });

    // 5. 递归转换 DOM 树
    const serialize = (node: Node): string => {
      if (node.nodeType === Node.TEXT_NODE) {
        return node.nodeValue || '';
      }
      if (node.nodeType !== Node.ELEMENT_NODE) {
        return '';
      }

      const el = node as Element;
      const tagName = el.tagName.toLowerCase();
      let childrenContent = '';
      el.childNodes.forEach(child => {
        childrenContent += serialize(child);
      });

      switch (tagName) {
        case 'p':
          return `\n\n${childrenContent.trim()}\n\n`;
        case 'strong':
        case 'b':
          return ` **${childrenContent.trim()}** `;
        case 'em':
        case 'i':
          return ` *${childrenContent.trim()}* `;
        case 'h1':
          return `\n\n# ${childrenContent.trim()}\n\n`;
        case 'h2':
          return `\n\n## ${childrenContent.trim()}\n\n`;
        case 'h3':
          return `\n\n### ${childrenContent.trim()}\n\n`;
        case 'h4':
          return `\n\n#### ${childrenContent.trim()}\n\n`;
        case 'blockquote':
          const quoted = childrenContent.trim().split('\n').map(line => `> ${line}`).join('\n');
          return `\n\n${quoted}\n\n`;
        case 'ul':
          return `\n\n${childrenContent}\n\n`;
        case 'ol':
          return `\n\n${childrenContent}\n\n`;
        case 'li':
          const parentElement = el.parentElement;
          const parentTag = parentElement?.tagName.toLowerCase();
          if (parentTag === 'ol' && parentElement) {
            const index = Array.from(parentElement.children).indexOf(el) + 1;
            return `\n${index}. ${childrenContent.trim()}`;
          }
          return `\n- ${childrenContent.trim()}`;
        case 'br':
          return '\n';
        case 'code':
          if (el.parentElement?.tagName.toLowerCase() === 'pre') {
            return childrenContent;
          }
          return ` \`${childrenContent.trim()}\` `;
        case 'pre':
          const codeEl = el.querySelector('code');
          const lang = codeEl?.getAttribute('class')?.match(/language-(\w+)/)?.[1] || '';
          return `\n\n\`\`\`${lang}\n${childrenContent.trim()}\n\`\`\`\n\n`;
        case 'div':
        case 'span':
        case 'section':
        default:
          return childrenContent;
      }
    }

    let markdown = serialize(container);
    markdown = markdown.replace(/\n{3,}/g, '\n\n');
    return markdown.trim();
  }, html);
}

/**
 * 抓取单个回答
 */
export async function scrapeAnswer(page: Page, targetUrl: string): Promise<ExtractedContent> {
  const norm = normalizeUrl(targetUrl);
  if (norm.type !== 'answer') {
    throw new Error(`链接不是知乎回答格式: ${targetUrl}`);
  }
  const answerId = norm.id.split(':')[1];

  logger.info(`正在加载回答页面: ${norm.normalizedUrl}`);

  // 1. 设置 API 拦截
  let apiContent: string | null = null;
  let apiData: any = null;
  
  const handleResponse = async (response: any) => {
    const url = response.url();
    if (url.includes(`/api/v4/answers/${answerId}`)) {
      try {
        const json = await response.json();
        if (json && json.content) {
          apiContent = json.content;
          apiData = json;
        }
      } catch (e) {
        // ignore
      }
    }
  };
  page.on('response', handleResponse);

  try {
    await page.goto(norm.normalizedUrl, { waitUntil: 'domcontentloaded', timeout: 30000 });
    await page.waitForTimeout(1000);
  } catch (e: any) {
    logger.warn(`页面跳转超时或出错，尝试继续解析: ${e.message}`);
  } finally {
    page.off('response', handleResponse);
  }

  // 检测跳转
  const currentUrl = page.url();
  if (currentUrl.includes('signin')) {
    throw new Error('LOGIN_REQUIRED: 未登录或登录态失效，无法访问内容。请在沉浸阅读的知乎获取面板登录。');
  }
  if (currentUrl.includes('unhuman') || currentUrl.includes('captcha')) {
    throw new Error('CAPTCHA_REQUIRED: 触发了知乎防爬人机验证。请在沉浸阅读的知乎获取面板完成人机验证。');
  }
  await assertZhihuContentAvailable(page);

  // 2. 优先通过 API 拦截结果解析
  if (apiContent && apiData) {
    logger.info('成功通过拦截 API 数据解析回答！');
    const authorName = apiData.author?.name || '匿名用户';
    const authorId = apiData.author?.id || 'anonymous';
    const title = apiData.question?.title || '未命名问题';
    const createdTime = apiData.created_time || Math.floor(Date.now() / 1000);
    const updatedTime = apiData.updated_time || createdTime;
    const voteupCount = apiData.voteup_count || 0;
    const commentCount = apiData.comment_count || 0;

    const markdown = await convertHtmlToMarkdown(page, apiContent);

    return {
      id: norm.id,
      type: 'answer',
      title,
      authorId,
      authorName: authorName === '匿名' || authorName === '' ? '匿名用户' : authorName,
      contentHtml: apiContent,
      contentMarkdown: markdown,
      createdTime,
      updatedTime,
      voteupCount,
      commentCount,
      url: norm.normalizedUrl,
      answerId,
      questionId: norm.questionId,
      questionUrl: `https://www.zhihu.com/question/${norm.questionId}`
    };
  }

  // 3. 次优先：通过 js-initialData 兜底
  logger.info('API 拦截未生效，正在尝试从 Hydration JSON (js-initialData) 提取...');
  try {
    const hydrationDataStr = await page.$eval('#js-initialData', el => el.textContent);
    if (hydrationDataStr) {
      const hydration = JSON.parse(hydrationDataStr);
      const answerData = hydration?.initialState?.entities?.answers?.[answerId];
      if (answerData) {
        logger.info('成功从 Hydration JSON 中提取回答数据！');
        const content = answerData.content;
        const authorObj = answerData.author;
        const authorName = authorObj ? (hydration.initialState.entities.users[authorObj.id]?.name || authorObj.name) : '匿名用户';
        const authorId = authorObj?.id || 'anonymous';
        
        const questionObj = answerData.question;
        const questionId = questionObj?.id;
        const title = questionObj ? (hydration.initialState.entities.questions[questionId]?.title || questionObj.title) : '未命名问题';

        const createdTime = answerData.createdTime || Math.floor(Date.now() / 1000);
        const updatedTime = answerData.updatedTime || createdTime;
        const voteupCount = answerData.voteupCount || 0;
        const commentCount = answerData.commentCount || 0;

        const markdown = await convertHtmlToMarkdown(page, content);

        return {
          id: norm.id,
          type: 'answer',
          title,
          authorId,
          authorName: authorName === '匿名' || authorName === '' ? '匿名用户' : authorName,
          contentHtml: content,
          contentMarkdown: markdown,
          createdTime,
          updatedTime,
          voteupCount,
          commentCount,
          url: norm.normalizedUrl,
          answerId,
          questionId: questionId ? String(questionId) : norm.questionId,
          questionUrl: questionId ? `https://www.zhihu.com/question/${questionId}` : `https://www.zhihu.com/question/${norm.questionId}`
        };
      }
    }
  } catch (e: any) {
    logger.warn(`从 Hydration JSON 提取失败: ${e.message}`);
  }

  // 4. DOM 提取降级
  logger.info('Hydration JSON 解析未生效，正在进行 DOM 降级提取...');
  
  const answerSelector = `.AnswerCard[data-answer-id="${answerId}"], [data-zop-usertoken]`;
  const answerEl = await page.$(answerSelector) || await page.$('.AnswerCard') || await page.$('.Post-RichTextContainer') || await page.$('.RichText');
  
  if (!answerEl) {
    const screenshotPath = path.resolve(process.cwd(), 'debug-answer.png');
    try {
      await page.screenshot({ path: screenshotPath });
      logger.error(`已将调试截图保存至: ${screenshotPath}`);
    } catch {
      logger.warn('当前浏览器不支持调试截图，已跳过 debug-answer.png 生成。');
    }
    throw new Error('DOM_NOT_FOUND: 无法在页面中定位回答内容容器');
  }

  // 从 DOM 里尽力提炼信息
  const title = await page.$eval('.QuestionHeader-title', el => el.textContent?.trim()).catch(() => '未命名问题');
  let authorName = '匿名用户';
  try {
    authorName = await page.$eval('[meta[itemprop="name"]]', el => el.getAttribute('content') || '');
  } catch (e) {
    try {
      authorName = await page.$eval('.AuthorInfo-name', el => el.textContent?.trim() || '');
    } catch (err) {
      authorName = '匿名用户';
    }
  }
  
  const contentHtml = await answerEl.innerHTML();
  if (!contentHtml || contentHtml.trim() === '') {
    throw new Error('CONTENT_EMPTY: 提取到的 DOM 内容为空');
  }

  const markdown = await convertHtmlToMarkdown(page, contentHtml);

  // 尝试读时间（知乎通常在回答底部有一个发布/编辑时间）
  const timeText = await page.$eval('.ContentItem-time', el => el.textContent?.trim()).catch(() => '');
  let createdTime = Math.floor(Date.now() / 1000);
  let updatedTime = createdTime;
  if (timeText) {
    // 比如 "发布于 2023-05-12" 或 "编辑于 2023-05-12"
    const m = timeText.match(/(\d{4}-\d{2}-\d{2})/);
    if (m) {
      const seconds = Math.floor(new Date(m[1]).getTime() / 1000);
      createdTime = seconds;
      updatedTime = seconds;
    }
  }

  let voteupCount = 0;
  try {
    voteupCount = await page.$eval('[itemprop="upvoteCount"]', el => Number(el.getAttribute('content')));
  } catch (e) {
    try {
      voteupCount = await page.$eval('.VoteButton--up', el => {
        const txt = el.textContent || '';
        const num = txt.replace(/[^0-9]/g, '');
        return num ? Number(num) : 0;
      });
    } catch (err) {
      voteupCount = 0;
    }
  }

  return {
    id: norm.id,
    type: 'answer',
    title,
    authorId: 'unknown',
    authorName: authorName === '匿名' || authorName === '' ? '匿名用户' : authorName,
    contentHtml,
    contentMarkdown: markdown,
    createdTime,
    updatedTime,
    voteupCount,
    commentCount: 0,
    url: norm.normalizedUrl,
    answerId,
    questionId: norm.questionId,
    questionUrl: `https://www.zhihu.com/question/${norm.questionId}`
  };
}

/**
 * 抓取单个文章
 */
export async function scrapeArticle(page: Page, targetUrl: string): Promise<ExtractedContent> {
  const norm = normalizeUrl(targetUrl);
  if (norm.type !== 'article') {
    throw new Error(`链接不是知乎文章格式: ${targetUrl}`);
  }
  const articleId = norm.id.split(':')[1];

  logger.info(`正在加载文章页面: ${norm.normalizedUrl}`);

  let apiContent: string | null = null;
  let apiData: any = null;

  const handleResponse = async (response: any) => {
    const url = response.url();
    if (url.includes(`/api/v4/articles/${articleId}`) || url.includes(`/api/v4/posts/${articleId}`)) {
      try {
        const json = await response.json();
        if (json && json.content) {
          apiContent = json.content;
          apiData = json;
        }
      } catch (e) {
        // ignore
      }
    }
  };
  page.on('response', handleResponse);

  try {
    await page.goto(norm.normalizedUrl, { waitUntil: 'domcontentloaded', timeout: 30000 });
    await page.waitForTimeout(1000);
  } catch (e: any) {
    logger.warn(`页面跳转超时或出错，尝试继续解析: ${e.message}`);
  } finally {
    page.off('response', handleResponse);
  }

  // 检测跳转
  const currentUrl = page.url();
  if (currentUrl.includes('signin')) {
    throw new Error('LOGIN_REQUIRED: 未登录或登录态失效，无法访问内容。请在沉浸阅读的知乎获取面板登录。');
  }
  if (currentUrl.includes('unhuman') || currentUrl.includes('captcha')) {
    throw new Error('CAPTCHA_REQUIRED: 触发了知乎防爬人机验证。请在沉浸阅读的知乎获取面板完成人机验证。');
  }
  await assertZhihuContentAvailable(page);

  // 1. API 拦截数据
  if (apiContent && apiData) {
    logger.info('成功通过拦截 API 数据解析文章！');
    const authorName = apiData.author?.name || '匿名用户';
    const authorId = apiData.author?.id || 'anonymous';
    const title = apiData.title || '未命名文章';
    const createdTime = apiData.created || apiData.created_time || Math.floor(Date.now() / 1000);
    const updatedTime = apiData.updated || apiData.updated_time || createdTime;
    const voteupCount = apiData.voteup_count || apiData.likes_count || 0;
    const commentCount = apiData.comment_count || 0;

    const markdown = await convertHtmlToMarkdown(page, apiContent);

    return {
      id: norm.id,
      type: 'article',
      title,
      authorId,
      authorName: authorName === '匿名' || authorName === '' ? '匿名用户' : authorName,
      contentHtml: apiContent,
      contentMarkdown: markdown,
      createdTime,
      updatedTime,
      voteupCount,
      commentCount,
      url: norm.normalizedUrl,
      articleId
    };
  }

  // 2. Hydration 提取
  logger.info('API 拦截未生效，正在尝试从 Hydration JSON (js-initialData) 提取...');
  try {
    const hydrationDataStr = await page.$eval('#js-initialData', el => el.textContent);
    if (hydrationDataStr) {
      const hydration = JSON.parse(hydrationDataStr);
      // 文章或者 Post 可能在不同的 entity 下
      const articleData = hydration?.initialState?.entities?.articles?.[articleId] || 
                          hydration?.initialState?.entities?.posts?.[articleId];
      if (articleData) {
        logger.info('成功从 Hydration JSON 中提取文章数据！');
        const content = articleData.content;
        const authorObj = articleData.author;
        const authorName = authorObj ? (hydration.initialState.entities.users[authorObj.id]?.name || authorObj.name) : '匿名用户';
        const authorId = authorObj?.id || 'anonymous';
        const title = articleData.title || '未命名文章';

        const createdTime = articleData.createdTime || articleData.created || Math.floor(Date.now() / 1000);
        const updatedTime = articleData.updatedTime || articleData.updated || createdTime;
        const voteupCount = articleData.voteupCount || articleData.likesCount || 0;
        const commentCount = articleData.commentCount || 0;

        const markdown = await convertHtmlToMarkdown(page, content);

        return {
          id: norm.id,
          type: 'article',
          title,
          authorId,
          authorName: authorName === '匿名' || authorName === '' ? '匿名用户' : authorName,
          contentHtml: content,
          contentMarkdown: markdown,
          createdTime,
          updatedTime,
          voteupCount,
          commentCount,
          url: norm.normalizedUrl,
          articleId
        };
      }
    }
  } catch (e: any) {
    logger.warn(`从 Hydration JSON 提取失败: ${e.message}`);
  }

  // 3. DOM 提取降级
  logger.info('Hydration JSON 解析未生效，正在进行 DOM 降级提取...');
  const articleEl = await page.$('.Post-RichTextContainer') || await page.$('.Post-Content') || await page.$('.RichText');
  if (!articleEl) {
    throw new Error('DOM_NOT_FOUND: 无法在页面中定位文章内容容器');
  }

  let title = '未命名文章';
  try {
    title = await page.$eval('.Post-Title', el => el.textContent?.trim() || '');
  } catch (e) {
    try {
      title = await page.$eval('h1.Post-Title', el => el.textContent?.trim() || '');
    } catch (err) {
      title = '未命名文章';
    }
  }
  
  const authorName = await page.$eval('.AuthorInfo-name', el => el.textContent?.trim()).catch(() => '匿名用户');
  
  const contentHtml = await articleEl.innerHTML();
  if (!contentHtml || contentHtml.trim() === '') {
    throw new Error('CONTENT_EMPTY: 提取到的 DOM 内容为空');
  }

  const markdown = await convertHtmlToMarkdown(page, contentHtml);

  // 尝试读时间
  const timeText = await page.$eval('.Post-Time', el => el.textContent?.trim()).catch(() => '');
  let createdTime = Math.floor(Date.now() / 1000);
  let updatedTime = createdTime;
  if (timeText) {
    const m = timeText.match(/(\d{4}-\d{2}-\d{2})/);
    if (m) {
      const seconds = Math.floor(new Date(m[1]).getTime() / 1000);
      createdTime = seconds;
      updatedTime = seconds;
    }
  }

  return {
    id: norm.id,
    type: 'article',
    title,
    authorId: 'unknown',
    authorName: authorName === '匿名' || authorName === '' ? '匿名用户' : authorName,
    contentHtml,
    contentMarkdown: markdown,
    createdTime,
    updatedTime,
    voteupCount: 0,
    commentCount: 0,
    url: norm.normalizedUrl,
    articleId
  };
}

/**
 * 将抓取到的数据写入 Obsidian 格式的 Markdown 文件
 */
const MARKDOWN_IMAGE_RE = /!\[\]\((https?:\/\/[^\s)]+)\)/g;
const IMAGE_MAX_BYTES = 16 * 1024 * 1024;
const IMAGE_ITEM_MAX_BYTES = 64 * 1024 * 1024;
const IMAGE_MAX_COUNT = 100;
const IMAGE_MAX_REDIRECTS = 3;
const IMAGE_TIMEOUT_MS = 20_000;
const IMAGE_CDN_HOST = /^pic\d+\.zhimg\.com$/i;
const IMAGE_MIME_EXTENSIONS: Record<string, string> = {
  'image/jpeg': 'jpg',
  'image/png': 'png',
  'image/gif': 'gif',
  'image/webp': 'webp',
};

function imageHostAllowed(hostname: string): boolean {
  return IMAGE_CDN_HOST.test(hostname.toLowerCase());
}

export function isAllowedImageUrl(value: string): boolean {
  try {
    const url = new URL(value);
    return url.protocol === 'https:'
      && url.username === ''
      && url.password === ''
      && url.port === ''
      && imageHostAllowed(url.hostname);
  } catch {
    return false;
  }
}

function ipv4Parts(address: string): number[] | null {
  if (isIP(address) !== 4) return null;
  const parts = address.split('.').map(Number);
  return parts.length === 4 && parts.every((part) => Number.isInteger(part) && part >= 0 && part <= 255)
    ? parts
    : null;
}

export function isBlockedImageAddress(address: string): boolean {
  const ipv4 = ipv4Parts(address);
  if (ipv4) {
    const [a, b] = ipv4;
    return a === 0 || a === 10 || a === 127 || (a === 100 && b >= 64 && b <= 127)
      || (a === 169 && b === 254) || (a === 172 && b >= 16 && b <= 31)
      || (a === 192 && (b === 0 || b === 168)) || (a === 198 && b >= 18 && b <= 19)
      || (a === 203 && b === 0) || a >= 224;
  }
  if (isIP(address) !== 6) return true;
  const normalized = address.toLowerCase();
  return normalized === '::1' || normalized === '::'
    || normalized.startsWith('fc') || normalized.startsWith('fd')
    || normalized.startsWith('fe8') || normalized.startsWith('fe9')
    || normalized.startsWith('fea') || normalized.startsWith('feb')
    || normalized.startsWith('ff') || normalized.startsWith('2001:db8')
    || normalized.startsWith('2001:10') || normalized.startsWith('2001:20');
}

async function approvedAddress(url: URL): Promise<string> {
  if (!isAllowedImageUrl(url.toString())) {
    throw new Error('image URL is not an approved HTTPS CDN');
  }
  const addresses = await lookup(url.hostname, { all: true, verbatim: true });
  const approved = addresses.find((entry) => !isBlockedImageAddress(entry.address));
  if (!approved) throw new Error('image host resolves only to a blocked address');
  return approved.address;
}

function requestPinnedImage(url: URL, address: string): Promise<import('node:http').IncomingMessage> {
  return new Promise((resolve, reject) => {
    const client = request(url, {
      headers: {
        Referer: 'https://www.zhihu.com/',
        'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120 Safari/537.36',
      },
      timeout: IMAGE_TIMEOUT_MS,
      servername: url.hostname,
      lookup: (_hostname, _options, callback) => callback(null, address, isIP(address)),
    }, resolve);
    client.once('timeout', () => client.destroy(new Error('image request timed out')));
    client.once('error', reject);
    client.end();
  });
}

function imageMagic(prefix: Buffer): string | null {
  if (prefix.length >= 8 && prefix.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) return 'image/png';
  if (prefix.length >= 3 && prefix.subarray(0, 3).equals(Buffer.from([255, 216, 255]))) return 'image/jpeg';
  if (prefix.length >= 4 && prefix.subarray(0, 4).toString('ascii') === 'GIF8') return 'image/gif';
  if (prefix.length >= 12 && prefix.subarray(0, 4).toString('ascii') === 'RIFF' && prefix.subarray(8, 12).toString('ascii') === 'WEBP') return 'image/webp';
  return null;
}

async function downloadImageToTemp(url: string, tempPath: string): Promise<{ bytes: number; mime: string }> {
  let current = new URL(url);
  for (let redirects = 0; redirects <= IMAGE_MAX_REDIRECTS; redirects += 1) {
    const address = await approvedAddress(current);
    const response = await requestPinnedImage(current, address);
    if (response.statusCode && response.statusCode >= 300 && response.statusCode < 400) {
      const location = response.headers.location;
      response.resume();
      if (!location) throw new Error('image redirect has no location');
      if (redirects === IMAGE_MAX_REDIRECTS) throw new Error('too many image redirects');
      current = new URL(location, current);
      continue;
    }
    if (response.statusCode !== 200) {
      response.resume();
      throw new Error(`HTTP ${response.statusCode ?? 0}`);
    }
    const mime = String(response.headers['content-type'] || '').split(';', 1)[0].trim().toLowerCase();
    if (!(mime in IMAGE_MIME_EXTENSIONS)) {
      response.resume();
      throw new Error('unsupported image content type');
    }
    const contentLength = Number(response.headers['content-length']);
    if (Number.isFinite(contentLength) && contentLength > IMAGE_MAX_BYTES) {
      response.resume();
      throw new Error('image exceeds size limit');
    }
    let bytes = 0;
    let prefix = Buffer.alloc(0);
    const limiter = new Transform({
      transform(chunk: Buffer, _encoding, callback) {
        bytes += chunk.length;
        if (bytes > IMAGE_MAX_BYTES) {
          callback(new Error('image exceeds size limit'));
          return;
        }
        if (prefix.length < 16) prefix = Buffer.concat([prefix, chunk]).subarray(0, 16);
        callback(null, chunk);
      },
    });
    await pipeline(response, limiter, fs.createWriteStream(tempPath, { flags: 'wx' }));
    if (bytes === 0 || imageMagic(prefix) !== mime) throw new Error('image MIME does not match file signature');
    return { bytes, mime };
  }
  throw new Error('image redirect resolution failed');
}

function imageFileNameFor(url: string, mime?: string): string {
  const hash = createHash('md5').update(url).digest('hex').slice(0, 12);
  const match = url.match(/\.(jpg|jpeg|png|gif|webp)(?:[?#]|$)/i);
  const ext = mime && IMAGE_MIME_EXTENSIONS[mime]
    ? IMAGE_MIME_EXTENSIONS[mime]
    : match ? match[1].toLowerCase() : 'jpg';
  return `${hash}.${ext}`;
}

/**
 * 把 Markdown 中的远程图片下载到答主目录 assets/ 下，并改写为相对路径引用。
 * 单张失败只保留远程链接，不影响整篇归档。
 */
export async function archiveImagesLocally(markdown: string, authorPath: string): Promise<string> {
  // 去掉 lazy 图与 noscript 副本可能造成的相邻重复引用
  let result = markdown.replace(/(!\[\]\(([^)]+)\))(?:\s*!\[\]\(\2\))+/g, '$1');

  const urls = Array.from(new Set(Array.from(result.matchAll(MARKDOWN_IMAGE_RE), m => m[1])));
  if (urls.length === 0) return result;

  const assetsDir = path.join(authorPath, 'assets');
  fs.mkdirSync(assetsDir, { recursive: true });

  let downloaded = 0;
  let totalBytes = 0;
  for (const [index, url] of urls.entries()) {
    if (index >= IMAGE_MAX_COUNT || !isAllowedImageUrl(url)) {
      logger.warn(`图片下载被安全策略拒绝: ${url}`);
      continue;
    }
    const existingPrefix = createHash('md5').update(url).digest('hex').slice(0, 12);
    const existing = fs.readdirSync(assetsDir).find((name) => name.startsWith(`${existingPrefix}.`));
    const fileName = existing || imageFileNameFor(url);
    const filePath = path.join(assetsDir, fileName);
    try {
      if (!fs.existsSync(filePath)) {
        const tempPath = path.join(assetsDir, `.${existingPrefix}.tmp-${process.pid}-${Date.now()}`);
        try {
          const downloadedImage = await downloadImageToTemp(url, tempPath);
          if (totalBytes + downloadedImage.bytes > IMAGE_ITEM_MAX_BYTES) {
            throw new Error('item image byte limit exceeded');
          }
          totalBytes += downloadedImage.bytes;
          const finalName = imageFileNameFor(url, downloadedImage.mime);
          const finalPath = path.join(assetsDir, finalName);
          if (!fs.existsSync(finalPath)) fs.renameSync(tempPath, finalPath);
          else fs.rmSync(tempPath, { force: true });
          result = result.split(`![](${url})`).join(`![](assets/${finalName})`);
          downloaded++;
        } finally {
          if (fs.existsSync(tempPath)) fs.rmSync(tempPath, { force: true });
        }
      } else {
        result = result.split(`![](${url})`).join(`![](assets/${fileName})`);
        downloaded++;
      }
    } catch (e: unknown) {
      const message = e instanceof Error ? e.message : String(e);
      logger.warn(`图片下载失败，保留远程链接: ${url} (${message})`);
    }
  }
  if (downloaded > 0) {
    logger.info(`已本地化 ${downloaded}/${urls.length} 张图片到 ${assetsDir}`);
  }
  return result;
}

export async function writeMarkdownFile(extracted: ExtractedContent, outputBaseDir: string): Promise<string> {
  // 1. 创建答主目录
  const authorDirName = sanitizeFilename(extracted.authorName, extracted.authorId || 'anonymous');
  const authorPath = path.resolve(outputBaseDir, authorDirName);
  if (!fs.existsSync(authorPath)) {
    fs.mkdirSync(authorPath, { recursive: true });
  }

  // 2. 格式化日期 YYYY-MM-DD
  const dateObj = new Date(extracted.createdTime * 1000);
  const yyyy = dateObj.getFullYear();
  const mm = String(dateObj.getMonth() + 1).padStart(2, '0');
  const dd = String(dateObj.getDate()).padStart(2, '0');
  const dateStr = `${yyyy}-${mm}-${dd}`;

  // 3. 构造文件名: YYYY-MM-DD-问题/文章标题_ID.md
  const rawId = extracted.id.split(':')[1];
  const mainTitle = sanitizeFilename(extracted.title, rawId);
  let fileName = `${dateStr}-${mainTitle}.md`;
  let filePath = path.join(authorPath, fileName);

  // Windows MAX_PATH(260) 保护：全路径超长时只压缩标题部分，保住日期与 ID
  const MAX_FULL_PATH = 240;
  if (filePath.length > MAX_FULL_PATH) {
    const fixedLen = authorPath.length + 1 /* \ */ + dateStr.length + 1 /* - */ + rawId.length + 1 /* _ */ + 3 /* .md */;
    const titleBudget = Math.max(8, MAX_FULL_PATH - fixedLen);
    const shortTitle = sanitizeFilename(extracted.title, rawId, titleBudget);
    fileName = `${dateStr}-${shortTitle}.md`;
    filePath = path.join(authorPath, fileName);
    logger.warn(`标题过长触发路径压缩: ${extracted.title} -> ${fileName}`);
  }

  // 5. 构造正文头部排版 (H1 问题/文章标题 居中, 作者与日期在同一行左右分布)
  const safeTitle = escapeHtmlText(extracted.title);
  const safeAuthorName = escapeHtmlText(extracted.authorName);
  const headerLayout = `<h1 style="text-align: center; margin-bottom: 20px;">${safeTitle}</h1>

<div style="display: flex; justify-content: space-between; align-items: center; border-bottom: 1px solid #e0e0e0; padding-bottom: 8px; margin-bottom: 20px;"><span style="font-weight: bold; color: #333;">作者：${safeAuthorName}</span><span style="color: #666;">日期：${dateStr}</span></div>

`;

  // 6. 图片本地化：下载远程图片到答主目录 assets/，Markdown 改为相对路径引用
  const localizedMarkdown = await archiveImagesLocally(extracted.contentMarkdown, authorPath);

  // 7. 写入文件 (完全剔除 Frontmatter, 直接以 H1 标题开始)
  const fullContent = headerLayout + localizedMarkdown;
  fs.writeFileSync(filePath, fullContent, 'utf-8');
  logger.info(`内容已成功归一化并写入文件: ${filePath}`);
  return filePath;
}
