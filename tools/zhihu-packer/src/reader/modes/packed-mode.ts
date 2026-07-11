import { ArticleMetadata } from '../core/metadata.js';

/**
 * 校验页面中是否预注入了打包数据
 */
export function getPackedArticles(): ArticleMetadata[] | null {
  const jsonScript = document.getElementById('articles-json');
  if (!jsonScript) return null;

  const rawJson = jsonScript.textContent?.trim();
  if (!rawJson || rawJson.includes('ARTICLES_JSON_PLACEHOLDER')) {
    // 占位符尚未被替换，说明是空白通用阅读器
    return null;
  }

  try {
    const articles = JSON.parse(rawJson) as any[];
    if (Array.isArray(articles) && articles.length > 0) {
      // 兼容老版本的转换数据格式，补全 articleId
      return articles.map((art, idx) => ({
        articleId: art.articleId || `packed_${idx}`,
        title: art.title || '无标题',
        date: art.date || '未知日期',
        timestamp: art.timestamp || Date.parse(art.date) || Date.now(),
        author: art.author || '老丹尼',
        summary: art.summary || (art.htmlContent ? art.htmlContent.replace(/<[^>]+>/g, '').slice(0, 150) : ''),
        wordCount: art.wordCount || (art.htmlContent ? art.htmlContent.length : 0),
        upvoteCount: art.upvoteCount,
        frontMatter: art.frontMatter || {},
        // 预存的 htmlContent
        htmlContent: art.htmlContent,
        relativePath: art.relativePath || '',
        filename: art.filename || ''
      } as any));
    }
  } catch (err) {
    console.error('解析打包注入 JSON 数据失败，将优雅进入通用导入模式:', err);
  }
  return null;
}
