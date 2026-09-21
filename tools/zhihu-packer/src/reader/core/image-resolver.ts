import { VirtualFile, readBlob } from './scanner.js';

/**
 * 根据基础 Markdown 路径与引用的相对路径，计算目标图片在根目录下的绝对相对路径。
 * `..` 越出根目录时返回 null——调用方不得再退化到文件名模糊匹配，否则可能错绑同名文件。
 */
export function resolveRelativePath(basePath: string, relativePath: string): string | null {
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
      if (baseParts.length === 0) return null;
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

// P2-3：归档自旧版本的内容可能仍含 http(s) 远程图——原样放行会让阅读时向
// 第三方主机外发请求（IP/Referer 追踪面）。替换为本地占位 SVG。
const REMOTE_IMAGE_BLOCKED_SVG = `data:image/svg+xml;utf8,<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120" viewBox="0 0 200 120"><rect width="100%" height="100%" fill="%231E1E24"/><text x="50%" y="50%" dominant-baseline="middle" text-anchor="middle" font-family="sans-serif" font-size="12" fill="%238E919A">远程图片已屏蔽</text></svg>`;

// 缓存每个文件的 Blob URL，防重复生成
// Key: relativePath, Value: Blob URL
const fileUrlCache = new Map<string, string>();

/**
 * 为容器中所有的相对路径图片进行 Blob URL 渲染，并缓存生成的 URL。
 * DOM 正文不卸载，因此 Blob URL 常驻至 clearAllImageCache——按文章回收会导致
 * 滚回旧文章时 <img> 指向已吊销地址而永久裂图。
 */
export async function resolveLocalImages(
  container: HTMLElement,
  markdownRelativePath: string,
  rootFilesMap: Map<string, VirtualFile>
): Promise<void> {
  const images = container.querySelectorAll('img');
  if (images.length === 0) return;

  for (const img of Array.from(images)) {
    const rawSrc = img.getAttribute('src');
    if (!rawSrc) continue;

    // 网络图片一律屏蔽为占位（P2-3），已转换的 Blob/Data URL 直接跳过。
    // `//host/…` 是协议相对地址——按页面协议补全后同样是远程请求，一并屏蔽。
    if (rawSrc.startsWith('http://') || rawSrc.startsWith('https://') || rawSrc.startsWith('//')) {
      img.src = REMOTE_IMAGE_BLOCKED_SVG;
      img.alt = img.alt || '远程图片已屏蔽';
      continue;
    }
    if (rawSrc.startsWith('data:') || rawSrc.startsWith('blob:')) {
      continue;
    }

    try {
      // 1. 解码 URL 编码的路径 (例如 %20 -> 空格) 并统一斜杠
      let decodedSrc: string;
      try {
        decodedSrc = decodeURIComponent(rawSrc).replace(/\\/g, '/');
      } catch {
        decodedSrc = rawSrc.replace(/\\/g, '/');
      }

      // 2. 依次尝试检索
      let matchedFile: VirtualFile | undefined;

      // 路径 1: 相对于当前 md 文件的相对路径 (null = .. 越出根目录，不做文件名回退)
      const relPath = resolveRelativePath(markdownRelativePath, decodedSrc);
      // 路径 2: 相对于导入的根目录的路径 (直接就是 src 本身，去除可能的前导 ./ )
      const rootRelPath = decodedSrc.replace(/^\.\//, '');

      // 2.1 先精准匹配这两种路径
      matchedFile = (relPath ? rootFilesMap.get(relPath) : undefined) || rootFilesMap.get(rootRelPath);

      // 2.2 如果没找到，尝试大小写不敏感精准匹配
      if (!matchedFile && relPath !== null) {
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
      // 安全保障：如果只命中一个则通过；如果命中多个，拒绝自动选择并警告冲突。
      // relPath === null 表示 .. 越出根目录——此时文件名回退可能错绑别处的同名图片，直接判缺失。
      if (!matchedFile && relPath !== null) {
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
      } else {
        // 4. 图片缺失显示优雅占位
        const displayPath = relPath ?? decodedSrc;
        const escapedPath = displayPath.length > 25 ? '...' + displayPath.slice(-25) : displayPath;
        img.src = IMAGE_NOT_FOUND_SVG.replace('PATH_PLACEHOLDER', escapedPath);
        img.classList.add('img-missing');
      }
    } catch (err) {
      console.error(`解析图片路径出错 [src=${rawSrc}]:`, err);
      img.src = IMAGE_NOT_FOUND_SVG.replace('PATH_PLACEHOLDER', '加载失败');
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
  console.log('[ImageResolver] 已释放全部本地图片 Blob URL 缓存');
}
