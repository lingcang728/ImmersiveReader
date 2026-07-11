import { ArticleMetadata } from './metadata.js';

// 用于检测汉字拼音首字母的临界边界字 (基于中文拼音顺序)
const PINYIN_BOUNDS = [
  { letter: 'a', char: '啊' },
  { letter: 'b', char: '芭' },
  { letter: 'c', char: '擦' },
  { letter: 'd', char: '搭' },
  { letter: 'e', char: '蛾' },
  { letter: 'f', char: '发' },
  { letter: 'g', char: '噶' },
  { letter: 'h', char: '哈' },
  { letter: 'j', char: '击' },
  { letter: 'k', char: '喀' },
  { letter: 'l', char: '垃' },
  { letter: 'm', char: '妈' },
  { letter: 'n', char: '拿' },
  { letter: 'o', char: '哦' },
  { letter: 'p', char: '啪' },
  { letter: 'q', char: '期' },
  { letter: 'r', char: '然' },
  { letter: 's', char: '撒' },
  { letter: 't', char: '塌' },
  { letter: 'w', char: '挖' },
  { letter: 'x', char: '昔' },
  { letter: 'y', char: '压' },
  { letter: 'z', char: '匝' }
];

/**
 * 快速获取单个汉字的拼音首字母
 */
export function getCharPinyinLetter(char: string): string {
  if (/[a-zA-Z0-9]/.test(char)) return char.toLowerCase();
  if (!/^[\u4e00-\u9fa5]$/.test(char)) return '';

  let l = 0, r = PINYIN_BOUNDS.length - 1;
  let ans = '';

  // 二分法在临界字数组中检索
  while (l <= r) {
    const mid = (l + r) >> 1;
    if (char.localeCompare(PINYIN_BOUNDS[mid].char, 'zh-Hans-CN') >= 0) {
      ans = PINYIN_BOUNDS[mid].letter;
      l = mid + 1;
    } else {
      r = mid - 1;
    }
  }
  return ans || char.toLowerCase();
}

/**
 * 获取整段字符串的拼音首字母简拼 (例如 "老丹尼" -> "ldn")
 */
export function getPinyinAbbr(str: string): string {
  let result = '';
  for (let i = 0; i < str.length; i++) {
    const letter = getCharPinyinLetter(str[i]);
    if (letter) {
      result += letter;
    }
  }
  return result;
}

export interface SearchIndexItem {
  articleId: string;
  title: string;
  pinyinAbbr: string; // 标题拼音首字母简拼
  path: string;       // 相对路径
  summary: string;    // 前 150 字摘要
  index: number;      // 原始排序索引
}

/**
 * 为文章列表构建轻量内存搜索索引
 */
export function buildSearchIndex(
  articles: ArticleMetadata[]
): SearchIndexItem[] {
  return articles.map((art, idx) => {
    const pinyinAbbr = getPinyinAbbr(art.title);
    return {
      articleId: art.articleId,
      title: art.title,
      pinyinAbbr,
      path: art.relativePath || art.frontMatter?.path || art.filename || '',
      summary: art.summary,
      index: idx
    };
  });
}

/**
 * 拼音简拼/标题/相对路径/摘要分层检索算法
 */
export function searchArticles(
  query: string,
  searchIndex: SearchIndexItem[]
): SearchIndexItem[] {
  const cleanQuery = query.trim().toLowerCase();
  if (!cleanQuery) return searchIndex;

  const titleMatches: SearchIndexItem[] = [];
  const pinyinMatches: SearchIndexItem[] = [];
  const pathMatches: SearchIndexItem[] = [];
  const summaryMatches: SearchIndexItem[] = [];

  for (const item of searchIndex) {
    const title = item.title.toLowerCase();
    const pinyin = item.pinyinAbbr;
    const path = item.path.toLowerCase();
    const summary = item.summary.toLowerCase();

    if (title.includes(cleanQuery)) {
      titleMatches.push(item);
    } else if (pinyin.includes(cleanQuery)) {
      pinyinMatches.push(item);
    } else if (path.includes(cleanQuery)) {
      pathMatches.push(item);
    } else if (summary.includes(cleanQuery)) {
      summaryMatches.push(item);
    }
  }

  // 按匹配权重分层排序返回 (标题最优先，其次是拼音首字母，再次路径，最后摘要)
  return [...titleMatches, ...pinyinMatches, ...pathMatches, ...summaryMatches];
}
