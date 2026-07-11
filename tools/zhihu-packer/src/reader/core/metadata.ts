import { VirtualFile } from './scanner.js';

export interface ArticleMetadata {
  articleId: string;    // 由相对路径计算出来的稳定唯一 ID
  relativePath: string; // 相对路径
  filename?: string;
  title: string;
  date: string;         // YYYY-MM-DD 格式
  timestamp: number;    // 用于排序的毫秒戳
  author: string;
  summary: string;      // 简短摘要 (前 150 字)
  wordCount: number;
  upvoteCount?: number; // 知乎专享赞同数
  htmlContent?: string; // Packed Mode 预渲染 HTML
  frontMatter: Record<string, string>;
}

/**
 * 快速计算 FNV-1a Hash，生成稳定的 8 位十六进制文章 ID
 */
export function calculateHash(str: string): string {
  let hash = 2166136261;
  for (let i = 0; i < str.length; i++) {
    hash ^= str.charCodeAt(i);
    hash += (hash << 1) + (hash << 4) + (hash << 7) + (hash << 8) + (hash << 24);
  }
  return 'art_' + (hash >>> 0).toString(16).padStart(8, '0');
}

/**
 * 计算整个目录的 Source ID (基于目录名、文件数和若干文件路径+大小)
 */
export function calculateSourceId(
  rootDirName: string,
  files: VirtualFile[]
): string {
  const stableFiles = [...files].sort((a, b) => a.relativePath.localeCompare(b.relativePath));
  let inputStr = `${rootDirName}_${stableFiles.length}`;
  // 选取最多 5 个文件做特征值混合
  const step = Math.max(1, Math.floor(stableFiles.length / 5));
  for (let i = 0; i < stableFiles.length; i += step) {
    if (stableFiles[i]) {
      inputStr += `_${stableFiles[i].relativePath}_${stableFiles[i].lastModified}`;
    }
  }
  return 'src_' + calculateHash(inputStr).replace('art_', '');
}

/**
 * 简易安全解析 YAML Front Matter (键值对匹配)
 */
function parseYaml(yamlStr: string): Record<string, string> {
  const result: Record<string, string> = {};
  const lines = yamlStr.split('\n');
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith('#')) continue;
    const colonIdx = trimmed.indexOf(':');
    if (colonIdx > 0) {
      const key = trimmed.slice(0, colonIdx).trim();
      let value = trimmed.slice(colonIdx + 1).trim();
      // 去除两端包裹的引号
      if ((value.startsWith('"') && value.endsWith('"')) || 
          (value.startsWith("'") && value.endsWith("'"))) {
        value = value.slice(1, -1);
      }
      result[key] = value;
    }
  }
  return result;
}

/**
 * 从文件名中提取 YYYY-MM-DD 格式的日期
 */
function extractDateFromFilename(filename: string): string | null {
  const match = filename.match(/(\d{4})[-_](\d{2})[-_](\d{2})/);
  if (match) {
    return `${match[1]}-${match[2]}-${match[3]}`;
  }
  return null;
}

/**
 * 格式化 Date 对象为 YYYY-MM-DD
 */
function formatDate(timestamp: number): string {
  const d = new Date(timestamp);
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const r = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${r}`;
}

/**
 * 纯前端轻量提取单个 Markdown 的元数据 (只处理传入的头部文本，如 16KB)
 */
export function extractMetadata(
  relativePath: string,
  fileName: string,
  mtime: number,
  headText: string
): ArticleMetadata {
  let title = '';
  let dateStr = '';
  let timestamp = mtime;
  let author = '本地文档';
  let frontMatter: Record<string, string> = {};
  
  // 1. 尝试匹配 YAML Front Matter (必须以 --- 开头)
  const normalizedText = headText.replace(/\r\n/g, '\n');
  let bodyText = normalizedText;
  
  if (normalizedText.startsWith('---\n')) {
    const nextSeparator = normalizedText.indexOf('\n---\n', 4);
    if (nextSeparator > 0) {
      const yamlContent = normalizedText.slice(4, nextSeparator);
      frontMatter = parseYaml(yamlContent);
      bodyText = normalizedText.slice(nextSeparator + 5);
    }
  }

  // 2. 确定标题 (优先 Front Matter -> 其次正文第一个一、二级标题 -> 最终文件名)
  if (frontMatter.title) {
    title = frontMatter.title;
  } else {
    // 匹配第一个 # 标题 或 ## 标题
    const titleMatch = bodyText.match(/^#\s+(.+)$|^##\s+(.+)$/m);
    if (titleMatch) {
      title = (titleMatch[1] || titleMatch[2]).trim();
    } else {
      // 剥离扩展名作为标题
      title = fileName.replace(/\.md$/i, '');
    }
  }

  // 3. 确定日期与时间戳 (优先 Front Matter -> 其次文件名匹配 -> 最后修改时间)
  const fnDate = extractDateFromFilename(fileName);
  if (frontMatter.date) {
    dateStr = frontMatter.date;
    const parsedTime = Date.parse(dateStr);
    if (!isNaN(parsedTime)) {
      timestamp = parsedTime;
    }
  } else if (fnDate) {
    dateStr = fnDate;
    const parsedTime = Date.parse(dateStr);
    if (!isNaN(parsedTime)) {
      timestamp = parsedTime;
    }
  } else {
    dateStr = formatDate(mtime);
    timestamp = mtime;
  }

  // 4. 确定作者
  if (frontMatter.author) {
    author = frontMatter.author;
  }

  // 5. 提取字数与简单摘要 (前 150 字)
  // 过滤掉 markdown 的各种特殊标记符号，提取纯文本做摘要
  const cleanSummaryText = bodyText
    .replace(/^#+\s+.+$/gm, '') // 去除标题行
    .replace(/```[\s\S]*?```/g, '') // 去除代码块
    .replace(/\[([^\]]+)\]\([^)]+\)/g, '$1') // 链接保留文本
    .replace(/[*_`#>-]/g, '') // 过滤各种符号
    .replace(/\s+/g, ' ') // 合并空格
    .trim();

  const summary = cleanSummaryText.slice(0, 150) + (cleanSummaryText.length > 150 ? '...' : '');
  const wordCount = bodyText.replace(/\s+/g, '').length;

  const articleId = calculateHash(relativePath);

  return {
    articleId,
    relativePath,
    filename: fileName,
    title,
    date: dateStr,
    timestamp,
    author,
    summary,
    wordCount,
    upvoteCount: frontMatter.voteup_count ? parseInt(frontMatter.voteup_count, 10) : undefined,
    frontMatter
  };
}
