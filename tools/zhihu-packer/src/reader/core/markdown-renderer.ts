import { resolveLocalImages, resolveRelativePath } from './image-resolver.js';
import { VirtualFile } from './scanner.js';
import { cleanReaderMarkdown } from './markdown-cleaner.js';

// 声明全局第三方库 (在编译打包时它们会被直接内置在 HTML 模板中)
declare const marked: any;
declare const DOMPurify: any;

function bilingualTextRatio(text: string): number {
  const compact = text.replace(/\s+/g, '');
  if (!compact) return 0;
  const cjk = (compact.match(/[\u4e00-\u9fff]/g) ?? []).length;
  const latin = (compact.match(/[A-Za-z]/g) ?? []).length;
  const total = cjk + latin;
  return total === 0 ? 0 : cjk / total;
}

function isMostlyLatin(text: string): boolean {
  const compact = text.replace(/\s+/g, '');
  return compact.length >= 12 && bilingualTextRatio(text) < 0.2 && /[A-Za-z]{3,}/.test(compact);
}

function isMostlyChinese(text: string): boolean {
  const compact = text.replace(/\s+/g, '');
  const cjk = (compact.match(/[\u4e00-\u9fff]/g) ?? []).length;
  return compact.length >= 4 && cjk >= 4 && bilingualTextRatio(text) >= 0.28;
}

function normalizePodcastBilingual(wrapper: HTMLElement) {
  const usedIds = new Set<string>();
  let nextId = 0;
  const createId = () => {
    let id = `podcast-${nextId}`;
    while (usedIds.has(id)) {
      nextId += 1;
      id = `podcast-${nextId}`;
    }
    usedIds.add(id);
    nextId += 1;
    return id;
  };
  const getId = (element: HTMLElement) => element.dataset.bilingualId || '';
  const pairId = (translation: HTMLElement, original: HTMLElement) => {
    const id = getId(translation) || getId(original) || createId();
    usedIds.add(id);
    translation.classList.add('podcast-translation');
    translation.dataset.bilingualId = id;
    original.classList.add('podcast-original');
    original.lang = 'en';
    original.tabIndex = 0;
    original.dataset.bilingualId = id;
  };

  const elements = Array.from(wrapper.children) as HTMLElement[];
  for (let index = 0; index < elements.length - 1; index += 1) {
    const current = elements[index];
    const following = elements[index + 1];
    const currentText = current.textContent?.trim() ?? '';
    const followingText = following.textContent?.trim() ?? '';
    if (current.tagName === 'P' && following.tagName === 'P' && isMostlyLatin(currentText) && isMostlyChinese(followingText)) {
      const original = document.createElement('blockquote');
      original.textContent = currentText;
      following.insertAdjacentElement('afterend', original);
      current.remove();
      pairId(following, original);
      continue;
    }
    if (current.tagName === 'P' && following.tagName === 'BLOCKQUOTE' && isMostlyChinese(currentText) && isMostlyLatin(followingText)) {
      pairId(current, following);
    }
  }

  const directChildren = Array.from(wrapper.children) as HTMLElement[];
  let originals = directChildren.filter((element) => element.matches('blockquote.podcast-original'));
  for (let index = 0; index < directChildren.length; index += 1) {
    const element = directChildren[index];
    if (element.tagName === 'P' && element.classList.contains('podcast-translation')) {
      element.dataset.bilingualId ||= createId();
    }
    if (element.tagName === 'BLOCKQUOTE' && isMostlyLatin(element.textContent ?? '') && !element.classList.contains('podcast-original')) {
      const previous = directChildren[index - 1];
      if (previous?.tagName === 'P' && isMostlyChinese(previous.textContent ?? '')) {
        pairId(previous, element);
      }
    }
    if (element.matches('blockquote.podcast-original')) {
      element.lang = 'en';
      element.tabIndex = 0;
      element.dataset.bilingualId ||= createId();
      originals.push(element);
    }
  }
  originals = Array.from(new Set(originals));
  if (originals.length === 0) return;

  const heading = directChildren.find(
    (element) => element.tagName === 'H2' && element.textContent?.trim() === '英文原文',
  );
  if (heading) heading.classList.add('podcast-originals-heading');
  originals.forEach((original) => original.remove());
  if (!heading) {
    const newHeading = document.createElement('h2');
    newHeading.className = 'podcast-originals-heading';
    newHeading.textContent = '英文原文';
    wrapper.appendChild(newHeading);
  }
  originals.forEach((original) => wrapper.appendChild(original));
}

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
  chapterTitle?: string,
): Promise<HTMLElement> {
  if (typeof marked === 'undefined') {
    throw new Error('系统错误：未找到 Markdown 解析库 Marked。');
  }
  if (typeof DOMPurify === 'undefined') {
    throw new Error('系统错误：未找到 HTML 安全过滤库 DOMPurify。');
  }

  // 1. 将 Markdown 解析为原始 HTML (marked.parse 默认是同步的，但支持 async)
  // 为了安全及一致，我们使用 marked.parse 直接转换
  const cleanMarkdown = cleanReaderMarkdown(markdownText, chapterTitle);
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
      'src', 'href', 'title', 'alt', 'class', 'id', 'align', 'valign', 'width', 'height', 'loading', 'tabindex', 'data-bilingual-id'
    ],
    // 强制过滤 javascript: 伪协议
    ALLOW_UNKNOWN_PROTOCOLS: false,
    ALLOWED_URI_REGEXP: /^(?:(?:https?|mailto|ftp|tel):|[^a-z0-9+.-]+(?:[/?#]|$))/i
  });

  // 3. 构建临时宿主 DOM 容器
  const wrapper = document.createElement('div');
  wrapper.className = 'markdown-body-wrapper';
  wrapper.innerHTML = cleanHtml;
  normalizePodcastBilingual(wrapper);

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
