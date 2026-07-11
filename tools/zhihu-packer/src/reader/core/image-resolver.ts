import { VirtualFile, readBlob } from './scanner.js';

// 用于存储每篇文章所生成的 Blob URL 缓存
// Key: articleId, Value: 该文章下所有生成的 Blob URL 列表
const activeBlobCache = new Map<string, string[]>();

/**
 * 根据基础 Markdown 路径与引用的相对路径，计算目标图片在根目录下的绝对相对路径
 */
export function resolveRelativePath(basePath: string, relativePath: string): string {
  // 统一替换反斜杠
  const cleanRel = relativePath.replace(/\\/g, '/');
  
  // 提取当前 Markdown 文件所在的目录部分
  const baseParts = basePath.split('/');
  baseParts.pop(); // 移除文件名本身，保留父目录

  const relParts = cleanRel.split('/');
  for (const part of relParts) {
    if (part === '.' || part === '') {
      continue;
    } else if (part === '..') {
      baseParts.pop(); // 回退一级目录
    } else {
      baseParts.push(part);
    }
  }

  return baseParts.join('/');
}

/**
 * 优雅的“图片未找到”占位 SVG (Data URL 格式)
 */
const IMAGE_NOT_FOUND_SVG = `data:image/svg+xml;utf8,<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120" viewBox="0 0 200 120"><rect width="100%" height="100%" fill="%231E1E24"/><text x="50%" y="45%" dominant-baseline="middle" text-anchor="middle" font-family="sans-serif" font-size="12" fill="%238E919A">图片未找到</text><text x="50%" y="65%" dominant-baseline="middle" text-anchor="middle" font-family="monospace" font-size="9" fill="%235E616A">PATH_PLACEHOLDER</text></svg>`;

/**
 * 为容器中所有的相对路径图片进行 Blob URL 渲染，并缓存生成的 URL 供后续回收
 */
// 缓存每个文件的 Blob URL，防重复生成
// Key: relativePath, Value: Blob URL
const fileUrlCache = new Map<string, string>();

/**
 * 为容器中所有的相对路径图片进行 Blob URL 渲染，并缓存生成的 URL 供后续回收
 */
export async function resolveLocalImages(
  articleId: string,
  container: HTMLElement,
  markdownRelativePath: string,
  rootFilesMap: Map<string, VirtualFile>
): Promise<void> {
  const images = container.querySelectorAll('img');
  if (images.length === 0) return;

  const generatedUrls: string[] = [];

  for (const img of Array.from(images)) {
    const rawSrc = img.getAttribute('src');
    if (!rawSrc) continue;

    // 网络图片及已转换的 Blob/Data URL 直接跳过
    if (
      rawSrc.startsWith('http://') ||
      rawSrc.startsWith('https://') ||
      rawSrc.startsWith('data:') ||
      rawSrc.startsWith('blob:')
    ) {
      continue;
    }

    try {
      // 1. 解码 URL 编码的路径 (例如 %20 -> 空格) 并统一斜杠
      const decodedSrc = decodeURIComponent(rawSrc).replace(/\\/g, '/');

      // 2. 依次尝试检索
      let matchedFile: VirtualFile | undefined;

      // 路径 1: 相对于当前 md 文件的相对路径
      const relPath = resolveRelativePath(markdownRelativePath, decodedSrc);
      // 路径 2: 相对于导入的根目录的路径 (直接就是 src 本身，去除可能的前导 ./ )
      const rootRelPath = decodedSrc.replace(/^\.\//, '');

      // 2.1 先精准匹配这两种路径
      matchedFile = rootFilesMap.get(relPath) || rootFilesMap.get(rootRelPath);

      // 2.2 如果没找到，尝试大小写不敏感精准匹配
      if (!matchedFile) {
        const relPathLower = relPath.toLowerCase();
        const rootRelPathLower = rootRelPath.toLowerCase();
        
        matchedFile = rootFilesMap.get(relPathLower) || rootFilesMap.get(rootRelPathLower);
        
        if (!matchedFile) {
          // 尝试在 Map 的 keys 里匹配小写
          for (const [key, val] of rootFilesMap.entries()) {
            const keyLower = key.toLowerCase();
            if (keyLower === relPathLower || keyLower === rootRelPathLower) {
              matchedFile = val;
              break;
            }
          }
        }
      }

      // 2.3 如果依然没找到，尝试文件名大小写不敏感检索 (防止因回退层级错误导致找不到)
      // 安全保障：如果只命中一个则通过；如果命中多个，拒绝自动选择并警告冲突
      if (!matchedFile) {
        const filenameLower = decodedSrc.split('/').pop()?.toLowerCase();
        if (filenameLower) {
          const matches: VirtualFile[] = [];
          for (const [key, val] of rootFilesMap.entries()) {
            const keyFilename = key.split('/').pop() || '';
            if (keyFilename.toLowerCase() === filenameLower) {
              matches.push(val);
            }
          }
          if (matches.length === 1) {
            matchedFile = matches[0];
          } else if (matches.length > 1) {
            console.warn(
              `[ImageResolver] 图片匹配冲突：检测到多个名为 "${filenameLower}" 的图片候选，已拒绝自动匹配以防图片显示错误。冲突文件路径：\n` +
              matches.map(m => ` - ${m.relativePath}`).join('\n')
            );
          }
        }
      }

      if (matchedFile) {
        // 3. 将本地文件读取为 Blob 并创建 URL (优先走缓存，防止内存重复膨胀)
        let blobUrl = fileUrlCache.get(matchedFile.relativePath);
        if (!blobUrl) {
          const blob = await readBlob(matchedFile);
          blobUrl = URL.createObjectURL(blob);
          fileUrlCache.set(matchedFile.relativePath, blobUrl);
        }
        
        img.src = blobUrl;
        generatedUrls.push(blobUrl);
      } else {
        // 4. 图片缺失显示优雅占位
        const escapedPath = relPath.length > 25 ? '...' + relPath.slice(-25) : relPath;
        img.src = IMAGE_NOT_FOUND_SVG.replace('PATH_PLACEHOLDER', escapedPath);
        img.classList.add('img-missing');
      }
    } catch (err) {
      console.error(`解析图片路径出错 [src=${rawSrc}]:`, err);
      img.src = IMAGE_NOT_FOUND_SVG.replace('PATH_PLACEHOLDER', '加载失败');
    }
  }

  // 写入文章级别缓存，用于记录当前渲染生成了哪些 Blob
  if (generatedUrls.length > 0) {
    activeBlobCache.set(articleId, generatedUrls);
  }
}

/**
 * 回收释放单篇文章所占用的所有图片 Blob URL，防内存泄漏
 * (已禁用：为了保证跳转与滚动定位稳定性，DOM 正文不再卸载，因此不注销 Blob 防止出现裂图)
 */
export function revokeArticleImages(articleId: string): void {
  const urls = activeBlobCache.get(articleId);
  if (!urls) return;
  for (const url of urls) {
    try {
      URL.revokeObjectURL(url);
      // 同时也从 fileUrlCache 中清除该引用
      for (const [path, cachedUrl] of fileUrlCache.entries()) {
        if (cachedUrl === url) {
          fileUrlCache.delete(path);
          break;
        }
      }
    } catch (e) {
      console.warn(`注销 Blob URL 失败: ${url}`, e);
    }
  }
  activeBlobCache.delete(articleId);
}

/**
 * 根据邻域管理缓存（已禁用：防止滚动时图片闪烁与裂图）
 */
export function manageImageCache(
  activeArticleId: string,
  neighborIds: string[]
): void {
  const keep = new Set([activeArticleId, ...neighborIds]);
  for (const articleId of Array.from(activeBlobCache.keys())) {
    if (!keep.has(articleId)) {
      revokeArticleImages(articleId);
    }
  }
}

/**
 * 释放全部图片 Blob 缓存 (退出文件夹或重新载入时使用)
 */
export function clearAllImageCache(): void {
  // 释放所有在 fileUrlCache 中缓存的 Object URL
  for (const url of fileUrlCache.values()) {
    try {
      URL.revokeObjectURL(url);
    } catch (e) {
      console.warn(`注销 Blob URL 失败: ${url}`, e);
    }
  }
  fileUrlCache.clear();
  activeBlobCache.clear();
  console.log('[ImageResolver] 已释放全部本地图片 Blob URL 缓存');
}
