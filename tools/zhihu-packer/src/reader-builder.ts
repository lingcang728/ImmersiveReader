export type ReaderArticle = {
  readonly articleId: string;
  readonly filename: string;
  readonly relativePath: string;
  readonly title: string;
  readonly date: string;
  readonly upvoteCount: number;
  readonly htmlContent: string;
  readonly pinyinAbbr: string;
  readonly author: string;
  readonly summary: string;
  readonly wordCount: number;
  readonly frontMatter: Readonly<Record<string, string>>;
};

// P3-6：构建期预渲染（前 5 篇 data-rendered="true"）必须与运行时清洗走同一
// 份严格白名单——之前这里只 FORBID 了 style/form/input/button，预渲染正文里
// 保留的 style 属性、srcset、video/audio 等正好绕过了 app.ts 的 ALLOWED_*。
// 两侧白名单必须保持一致（reader/ui/app.ts sanitizeHtml）。
function sanitizeHtmlFragment(html: string): string {
  return DOM_PURIFY.sanitize(html, {
    USE_PROFILES: { html: true },
    ALLOWED_TAGS: [
      "h1", "h2", "h3", "h4", "h5", "h6", "p", "br", "hr", "blockquote",
      "ul", "ol", "li", "dl", "dt", "dd", "table", "thead", "tbody", "tr", "th", "td",
      "pre", "code", "em", "strong", "del", "span", "a", "img", "div", "ins", "sub", "sup",
    ],
    ALLOWED_ATTR: [
      "src", "href", "title", "alt", "class", "id", "align", "valign",
      "width", "height", "loading", "tabindex", "data-bilingual-id",
    ],
    ALLOW_UNKNOWN_PROTOCOLS: false,
    ALLOWED_URI_REGEXP: /^(?:(?:https?|mailto|ftp|tel):|[^a-z0-9+.-]+(?:[/?#]|$))/i,
  });
}

export async function renderMarkdownArticle(
  chapter: Chapter,
  markdown: string,
  author: string,
): Promise<ReaderArticle> {
  const body = markdown
    .replace(/^<h1[\s\S]*?<\/h1>\s*/i, "")
    .replace(/^<div[\s\S]*?<\/div>\s*/i, "")
    .trim();
  let htmlContent = sanitizeHtmlFragment(await marked.parse(body));
  htmlContent = htmlContent.replace(/<img ([\s\S]*?)\/?>/g, (_match, attributes: string) => {
    const cleaned = attributes.replace(/style="[^"]*"/g, "").trim();
    return `<div class="image-wrapper"><img ${cleaned} loading="lazy" /></div>`;
  });
  const plainText = body
    .replace(/<[^>]+>/g, "")
    .replace(/[*_`#>-]/g, "")
    .replace(/\s+/g, " ")
    .trim();
  const pinyinAbbr = pinyin(chapter.title, {
    pattern: "first",
    toneType: "none",
    nonZh: "removed",
  })
    .replace(/\s+/g, "")
    .toLowerCase();
  return {
    articleId: chapter.id,
    filename: chapter.path.split("/").at(-1) ?? chapter.path,
    relativePath: chapter.path,
    title: chapter.title,
    date: chapter.date ?? "未知日期",
    upvoteCount: chapter.voteCount,
    htmlContent,
    pinyinAbbr,
    author,
    summary: plainText.slice(0, 150) + (plainText.length > 150 ? "..." : ""),
    // P3-29：wordCount 规范口径 = Unicode 标量数（与 Rust 端一致），不按 UTF-16 码元。
    wordCount: chapter.wordCount || countWordChars(body),
    frontMatter: {
      path: chapter.path,
      voteup_count: String(chapter.voteCount),
    },
  };
}

export function safeSerializeJson(data: unknown): string {
  return JSON.stringify(data)
    .replace(/<\/script>/gi, "<\\/script>")
    .replace(/</g, "\\u003c")
    .replace(/>/g, "\\u003e")
    .replace(/\u2028/g, "\\u2028")
    .replace(/\u2029/g, "\\u2029");
}

function escapeHtmlText(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function renderArticle(article: ReaderArticle, index: number): string {
  const upvote = article.upvoteCount > 0
    ? `<span class="meta-badge meta-badge-upvote">赞同 ${article.upvoteCount}</span>`
    : "";
  return `
    <article class="article-card ${index === 0 ? "active" : ""}" id="article-${index}" data-index="${index}" data-rendered="true">
      <div class="article-header">
        <h2 class="article-title">${escapeHtmlText(article.title)}</h2>
        <div class="article-divider"></div>
        <div class="article-meta-row">
          <span class="meta-badge">${escapeHtmlText(article.date)}</span>
          <span class="meta-badge">作者：${escapeHtmlText(article.author)}</span>
          ${upvote}
        </div>
      </div>
      <div class="article-body">${article.htmlContent}</div>
    </article>`;
}

export function renderReaderHtml(
  htmlTemplate: string,
  articles: readonly ReaderArticle[],
  pageTitle: string,
): string {
  const preRendered = articles.slice(0, 5).map(renderArticle).join("\n");
  return htmlTemplate
    .replace("<!-- ARTICLES_DOM_PLACEHOLDER -->", preRendered)
    .replace("/* ARTICLES_JSON_PLACEHOLDER */", safeSerializeJson(articles))
    .replace(
      "<title>沉浸式 Markdown 阅读器</title>",
      `<title>${escapeHtmlText(pageTitle)}</title>`,
    );
}
import { JSDOM } from "jsdom";
import DOMPurifyFactory from "dompurify";
import { marked } from "marked";
import { pinyin } from "pinyin-pro";

import type { Chapter } from "../../../packages/contracts/dist/index.js";

import { countWordChars } from "./utils.js";

const DOM_PURIFY = DOMPurifyFactory(new JSDOM("").window);
