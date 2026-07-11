import { resolveLocalImages, resolveRelativePath } from './image-resolver.js';
import { VirtualFile } from './scanner.js';
import { cleanReaderMarkdown } from './markdown-cleaner.js';

// 声明全局第三方库 (在编译打包时它们会被直接内置在 HTML 模板中)
declare const marked: any;
declare const DOMPurify: any;

/**
 * 安全渲染 Markdown 为 DOM 节点，管道化处理：
 * Raw Markdown -> marked.parse -> DOMPurify.sanitize -> Local Image Resolver -> DOM
 */
export async function renderMarkdown(
  markdownText: string,
  articleId: string,
  relativePath: string,
  rootFilesMap: Map<string, VirtualFile>,
  servedContentBase?: string,
): Promise<HTMLElement> {
  if (typeof marked === 'undefined') {
    throw new Error('系统错误：未找到 Markdown 解析库 Marked。');
  }
  if (typeof DOMPurify === 'undefined') {
    throw new Error('系统错误：未找到 HTML 安全过滤库 DOMPurify。');
  }

  // 1. 将 Markdown 解析为原始 HTML (marked.parse 默认是同步的，但支持 async)
  // 为了安全及一致，我们使用 marked.parse 直接转换
  const cleanMarkdown = cleanReaderMarkdown(markdownText);
  const rawHtml = marked.parse(cleanMarkdown);

  // 2. 使用 DOMPurify 进行严格的 XSS 净化
  // 仅保留基础排版、表格、列表、超链接、图片等安全的 HTML 标签，阻断恶意 JS 执行
  const cleanHtml = DOMPurify.sanitize(rawHtml, {
    USE_PROFILES: { html: true },
    ALLOWED_TAGS: [
      'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'p', 'br', 'hr', 'blockquote',
      'ul', 'ol', 'li', 'dl', 'dt', 'dd', 'table', 'thead', 'tbody', 'tr', 'th', 'td',
      'pre', 'code', 'em', 'strong', 'del', 'span', 'a', 'img', 'div', 'ins', 'sub', 'sup'
    ],
    ALLOWED_ATTR: [
      'src', 'href', 'title', 'alt', 'class', 'id', 'align', 'valign', 'width', 'height', 'loading'
    ],
    // 强制过滤 javascript: 伪协议
    ALLOW_UNKNOWN_PROTOCOLS: false,
    ALLOWED_URI_REGEXP: /^(?:(?:https?|mailto|ftp|tel):|[^a-z0-9+.-]+(?:[/?#]|$))/i
  });

  // 3. 构建临时宿主 DOM 容器
  const wrapper = document.createElement('div');
  wrapper.className = 'markdown-body-wrapper';
  wrapper.innerHTML = cleanHtml;

  // 3.5. 注入稳定性 heading ID (在 DOMPurify 之后对临时 DOM 节点遍历注入)
  // ID 规则：articleId + '-' + headingIndex + '-' + headingTextHash
  const headings = wrapper.querySelectorAll('h1, h2, h3, h4, h5, h6');
  headings.forEach((heading, idx) => {
    const text = heading.textContent || '';
    let hash = 0;
    for (let i = 0; i < text.length; i++) {
      hash = (hash << 5) - hash + text.charCodeAt(i);
      hash |= 0;
    }
    const hashStr = Math.abs(hash).toString(36);
    heading.id = `${articleId}-${idx}-${hashStr}`;
  });

  // 4. 对此 DOM 中的相对路径本地图片进行转换，异步解析成本地 Blob URL
  if (servedContentBase) {
    for (const image of Array.from(wrapper.querySelectorAll('img'))) {
      const rawSource = image.getAttribute('src');
      if (!rawSource || /^(?:https?:|data:|blob:)/i.test(rawSource)) continue;
      const decoded = decodeURIComponent(rawSource).replace(/\\/g, '/');
      const resolved = decoded.startsWith('/')
        ? decoded.replace(/^\/+/, '')
        : resolveRelativePath(relativePath, decoded);
      const encoded = resolved.split('/').map((segment) => encodeURIComponent(segment)).join('/');
      image.src = `${servedContentBase.replace(/\/$/, '')}/${encoded}`;
    }
  } else {
    await resolveLocalImages(articleId, wrapper, relativePath, rootFilesMap);
  }

  // 5. 对正文内的相对路径 markdown 链接跳转进行拦截与解析
  // 例如 [跳转](./sub/doc.md) 转化为点击后由阅读器直接切换文章
  const localLinks = wrapper.querySelectorAll('a');
  for (const a of Array.from(localLinks)) {
    const href = a.getAttribute('href');
    if (href && !href.startsWith('http://') && !href.startsWith('https://') && !href.startsWith('#') && !href.startsWith('mailto:')) {
      a.addEventListener('click', (e) => {
        e.preventDefault();
        // 派发自定义跳转事件，由 app.ts 接收并跳转
        const customEvent = new CustomEvent('local-navigate', {
          bubbles: true,
          detail: { rawHref: href }
        });
        a.dispatchEvent(customEvent);
      });
    }
  }

  // 6. 等待当前文章下的所有图片加载完成，以计算精确高度，防止跳转回跳
  const imgs = wrapper.querySelectorAll('img');
  const imgPromises = Array.from(imgs).map(img => {
    return new Promise<void>(resolve => {
      if (img.complete) {
        resolve();
      } else {
        img.addEventListener('load', () => resolve(), { once: true });
        img.addEventListener('error', () => resolve(), { once: true });
        // 1秒超时保护，防止图片死链导致页面渲染挂起
        setTimeout(resolve, 1000);
      }
    });
  });
  await Promise.all(imgPromises);

  return wrapper;
}
