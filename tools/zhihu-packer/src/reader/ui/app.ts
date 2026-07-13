import { VirtualFile, readText } from '../core/scanner.js';
import { ArticleMetadata } from '../core/metadata.js';
import { renderMarkdown } from '../core/markdown-renderer.js';
import { manageImageCache, clearAllImageCache, revokeArticleImages } from '../core/image-resolver.js';
import { saveReadingProgress, getReadingProgress } from '../core/storage.js';
import { buildSearchIndex, searchArticles, SearchIndexItem } from '../core/search.js';
import { buildServedContentUrl } from '../modes/served-mode.js';
import type { ReaderMode } from '../modes/reader-mode.js';

declare const DOMPurify: any;

export class ReaderApp {
  private filesMap: Map<string, VirtualFile>;
  private articles: ArticleMetadata[];
  private sourceId: string;
  private sourceName: string;
  private mode: ReaderMode;
  private servedReadIds = new Set<string>();

  private activeIndex: number = 0;
  private fontSize: number = 20; // 默认 20px
  private isSidebarActive: boolean = false;
  private isSearchActive: boolean = false;
  private searchSelectedIndex: number = 0;
  private searchFiltered: SearchIndexItem[] = [];
  private searchIndex: SearchIndexItem[] = [];
  private searchReturnFocus: HTMLElement | null = null;
  
  // 滚动与转场锁
  private isScrollingLock: boolean = false;
  private scrollLockTimer: number | null = null;
  private scrollingToActiveIndex: number | null = null;
  
  // 键盘防抖队列
  private pendingKeyboardIndex: number | null = null;
  private keyboardNavTimer: number | null = null;
  private scrollRaf: number | null = null;
  private saveProgressTimer: number | null = null;

  // 转场事务序列号
  private currentTransitionId: number = 0;

  // DOM 缓存
  private container!: HTMLElement;
  private menu!: HTMLElement;
  private menuBubble!: HTMLElement;
  private sidebar!: HTMLElement;
  private sidebarOverlay!: HTMLElement;
  private menuTrigger!: HTMLElement;
  private floatingProgress!: HTMLElement;
  private topProgress!: HTMLElement;
  private sidebarSearch!: HTMLInputElement;

  // 搜索 Command Palette DOM
  private searchOverlay!: HTMLElement;
  private paletteInput!: HTMLInputElement;
  private paletteResults!: HTMLElement;

  // Lightbox 图片放大 DOM
  private lightbox!: HTMLElement;
  private lightboxImg!: HTMLImageElement;

  constructor(
    filesMap: Map<string, VirtualFile>,
    articles: ArticleMetadata[],
    sourceId: string,
    sourceName: string,
    mode: ReaderMode = { kind: 'file' }
  ) {
    this.filesMap = filesMap;
    this.articles = articles;
    this.sourceId = sourceId;
    this.sourceName = sourceName;
    this.mode = mode;

    this.initDOMElements();
    this.initSearchIndex();
    this.initUI();
    this.bindEvents();
    
    // 恢复历史进度
    void this.restoreProgress();
  }

  private initDOMElements() {
    this.container = document.getElementById('articles-container')!;
    this.menu = document.getElementById('sidebar-menu')!;
    this.menuBubble = document.getElementById('menu-bubble')!;
    this.sidebar = document.getElementById('sidebar')!;
    this.sidebarOverlay = document.getElementById('sidebar-overlay')!;
    this.menuTrigger = document.getElementById('menu-trigger')!;
    this.floatingProgress = document.getElementById('floating-progress')!;
    this.topProgress = document.getElementById('top-progress')!;
    this.sidebarSearch = document.getElementById('sidebar-search')! as HTMLInputElement;

    this.searchOverlay = document.getElementById('search-overlay')!;
    this.paletteInput = document.getElementById('palette-input')! as HTMLInputElement;
    this.paletteResults = document.getElementById('palette-results')!;

    this.lightbox = document.getElementById('lightbox-overlay')!;
    this.lightboxImg = document.getElementById('lightbox-img')! as HTMLImageElement;

    // 动态调整侧栏头部标题
    const sidebarTitle = this.sidebar.querySelector('.sidebar-title');
    const sidebarSubtitle = this.sidebar.querySelector('.sidebar-subtitle');
    if (sidebarTitle) sidebarTitle.textContent = this.sourceName;
    if (sidebarSubtitle) {
      sidebarSubtitle.textContent = this.mode.kind === 'packed'
        ? '离线归档'
        : this.mode.kind === 'served'
          ? '与沉浸阅读桌面端同步'
          : '本地模式 · 进度仅保存在此浏览器';
    }
    if (this.mode.kind === 'file' && !document.getElementById('local-mode-notice')) {
      const notice = document.createElement('div');
      notice.id = 'local-mode-notice';
      notice.textContent = '本地模式，进度仅保存在此浏览器';
      notice.setAttribute('role', 'status');
      Object.assign(notice.style, {
        position: 'fixed', left: '50%', bottom: '18px', transform: 'translateX(-50%)',
        zIndex: '70', padding: '7px 12px', borderRadius: '999px', fontSize: '12px',
        color: 'var(--text-secondary)', background: 'var(--bg-raised)', border: '1px solid var(--line)',
      });
      document.body.appendChild(notice);
    }
  }

  private initSearchIndex() {
    this.searchIndex = buildSearchIndex(this.articles);
  }

  private initUI() {
    // 1. 清空旧的主体 DOM 与图片缓存
    this.container.innerHTML = '';
    clearAllImageCache();

    // 2. 渲染目录栏
    this.renderSidebarMenu();

    // 3. 一次性生成所有文章的外壳占位符 (防止一次性解析百篇 Markdown 导致卡死)
    this.articles.forEach((art, idx) => {
      const card = document.createElement('article');
      card.className = 'article-card';
      card.id = `article-${idx}`;
      card.setAttribute('data-index', idx.toString());
      
      card.appendChild(this.createArticleHeader(art));
      const body = document.createElement('div');
      body.className = 'article-body';
      const placeholder = document.createElement('div');
      placeholder.className = 'article-loading-placeholder';
      placeholder.textContent = '正在加载正文...';
      body.appendChild(placeholder);
      card.appendChild(body);
      this.container.appendChild(card);
    });

    // 4. 初次触发渲染当前邻域
    this.lazyRenderNeighborhood();
  }

  /**
   * 渲染目录菜单（如果文章数超过 100 篇，默认对侧边栏进行折叠/按年分组）
   */
  private renderSidebarMenu() {
    this.menu.querySelectorAll('.menu-item').forEach(el => el.remove());
    
    const count = this.articles.length;
    
    // 如果文章数量较多，按年份对侧栏目录进行聚类折叠
    if (count > 100) {
      const groups = new Map<string, { title: string; index: number }[]>();
      this.articles.forEach((art, index) => {
        const year = art.date.substring(0, 4) + ' 年';
        if (!groups.has(year)) groups.set(year, []);
        groups.get(year)!.push({ title: art.title, index });
      });

      let groupIndex = 0;
      for (const [year, items] of groups.entries()) {
        const groupHeader = document.createElement('button');
        groupHeader.type = 'button';
        groupHeader.className = 'menu-group-header';
        const yearSpan = document.createElement('span');
        yearSpan.textContent = year;
        const countSpan = document.createElement('span');
        countSpan.className = 'group-count';
        countSpan.textContent = `(${items.length} 篇)`;
        groupHeader.append(yearSpan, countSpan);
        
        const groupContainer = document.createElement('div');
        groupContainer.className = 'menu-group-container';
        groupContainer.id = `menu-group-${groupIndex}`;
        groupHeader.setAttribute('aria-controls', groupContainer.id);
        groupHeader.setAttribute('aria-expanded', 'true');
        groupIndex += 1;
        
        items.forEach(({ title, index }) => {
          const item = this.createMenuItemDOM(title, this.articles[index], index);
          groupContainer.appendChild(item);
        });

        // 默认第一组展开，其他折叠
        groupHeader.onclick = () => {
          groupContainer.classList.toggle('collapsed');
          groupHeader.classList.toggle('collapsed');
          groupHeader.setAttribute('aria-expanded', String(!groupContainer.classList.contains('collapsed')));
        };

        this.menu.appendChild(groupHeader);
        this.menu.appendChild(groupContainer);
      }
    } else {
      // 数量少，直接展示扁平列表
      this.articles.forEach((art, index) => {
        const item = this.createMenuItemDOM(art.title, art, index);
        this.menu.appendChild(item);
      });
    }
  }

  private createMenuItemDOM(cleanTitle: string, art: ArticleMetadata, index: number): HTMLElement {
    const item = document.createElement('button');
    item.type = 'button';
    item.className = `menu-item ${index === this.activeIndex ? 'active' : ''}`;
    item.id = `menu-item-${index}`;
    item.onclick = (e) => {
      e.stopPropagation();
      this.scrollToArticle(index);
      this.closeSidebar();
    };
    
    const title = document.createElement('div');
    title.textContent = cleanTitle;
    const meta = document.createElement('div');
    meta.className = 'menu-item-meta';
    const date = document.createElement('span');
    date.textContent = art.date;
    const words = document.createElement('span');
    words.textContent = `字数: ${art.wordCount}`;
    meta.append(date, words);
    item.append(title, meta);
    return item;
  }

  private createArticleHeader(art: ArticleMetadata): HTMLElement {
    const header = document.createElement('div');
    header.className = 'article-header';
    const title = document.createElement('h2');
    title.className = 'article-title';
    title.textContent = art.title;
    const divider = document.createElement('div');
    divider.className = 'article-divider';
    const meta = document.createElement('div');
    meta.className = 'article-meta-row';
    const date = document.createElement('span');
    date.className = 'meta-badge';
    date.textContent = art.date;
    const author = document.createElement('span');
    author.className = 'meta-badge';
    author.textContent = `作者：${art.author}`;
    meta.append(date, author);
    if (art.upvoteCount) {
      const upvote = document.createElement('span');
      upvote.className = 'meta-badge meta-badge-upvote';
      upvote.innerHTML = '<svg viewBox="0 0 24 24"><path d="M23,10C23,9.5 22.8,9 22.4,8.6C22,8.2 21.5,8 21,8H15V6C15,4.9 14.1,4 13,4L12.5,4L8,8.5V20H20C20.6,20 21.2,19.6 21.4,19.1L22.9,13.1C23,12.8 23,12.4 23,12V10M1,20H5V8H1V20Z"/></svg>';
      upvote.appendChild(document.createTextNode(` 赞同 ${art.upvoteCount}`));
      meta.appendChild(upvote);
    }
    header.append(title, divider, meta);
    return header;
  }

  private async renderArticleBody(index: number): Promise<void> {
    const card = document.getElementById(`article-${index}`)!;
    if (!card || card.getAttribute('data-rendered') === 'true') return;

    const art = this.articles[index];
    const bodyContainer = card.querySelector('.article-body')!;
    
    try {
      if (this.mode.kind === 'packed' && art.htmlContent) {
        // Packed Mode 直接使用 JSON 中的 htmlContent，但包装并在 DOM 层面注入稳定 Heading ID
        const wrapper = document.createElement('div');
        wrapper.className = 'markdown-body-wrapper';
        wrapper.innerHTML = this.sanitizeHtml(art.htmlContent);

        // 注入稳定性 heading ID
        const headings = wrapper.querySelectorAll('h1, h2, h3, h4, h5, h6');
        headings.forEach((heading, idx) => {
          const text = heading.textContent || '';
          let hash = 0;
          for (let i = 0; i < text.length; i++) {
            hash = (hash << 5) - hash + text.charCodeAt(i);
            hash |= 0;
          }
          const hashStr = Math.abs(hash).toString(36);
          heading.id = `${art.articleId}-${idx}-${hashStr}`;
        });

        // 等待所有异步图片加载完成 (对 Data URL/网络图超时判定)
        const imgs = wrapper.querySelectorAll('img');
        const imgPromises = Array.from(imgs).map(img => {
          return new Promise<void>(resolve => {
            if (img.complete) {
              resolve();
            } else {
              img.addEventListener('load', () => resolve(), { once: true });
              img.addEventListener('error', () => resolve(), { once: true });
              setTimeout(resolve, 500); // 较低网络超时，保证首屏流畅性
            }
          });
        });
        await Promise.all(imgPromises);

        bodyContainer.innerHTML = '';
        bodyContainer.appendChild(wrapper);
        card.setAttribute('data-rendered', 'true');
        this.bindImageLightboxes(card);
        return;
      } else if (this.mode.kind === 'served') {
        const response = await fetch(buildServedContentUrl(this.mode.contentBase, art.relativePath), {
          cache: 'no-store',
        });
        if (!response.ok) throw new Error(`${response.status} ${await response.text()}`);
        const rawMarkdown = await response.text();
        const renderedDom = await renderMarkdown(
          rawMarkdown,
          art.articleId,
          art.relativePath,
          this.filesMap,
          this.mode.contentBase,
        );
        bodyContainer.innerHTML = '';
        bodyContainer.appendChild(renderedDom);
        card.setAttribute('data-rendered', 'true');
        this.bindImageLightboxes(card);
        return;
      } else {
        // Universal Mode 下动态从文件句柄读取 Markdown 全文并转换
        const vFile = this.filesMap.get(art.relativePath);
        if (!vFile) throw new Error('找不到关联的本地文件');
        
        const rawMarkdown = await readText(vFile);

        // 渲染并安全净化，且已注入稳定 Heading ID 与等待图片加载完成
        const renderedDom = await renderMarkdown(
          rawMarkdown,
          art.articleId,
          art.relativePath,
          this.filesMap
        );
        bodyContainer.innerHTML = '';
        bodyContainer.appendChild(renderedDom);
        card.setAttribute('data-rendered', 'true');
        
        this.bindImageLightboxes(card);
        return;
      }
    } catch (err) {
      console.error(`渲染文章正文失败 [index=${index}, title=${art.title}]:`, err);
      bodyContainer.textContent = '';
      const error = document.createElement('div');
      error.className = 'article-error-placeholder';
      error.textContent = '正文加载失败：文件丢失或读取权限被拒绝。';
      bodyContainer.appendChild(error);
    }
  }

  private sanitizeHtml(html: string): string {
    if (typeof DOMPurify === 'undefined') return '';
    return DOMPurify.sanitize(html, {
      USE_PROFILES: { html: true },
      ALLOWED_TAGS: [
        'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'p', 'br', 'hr', 'blockquote',
        'ul', 'ol', 'li', 'dl', 'dt', 'dd', 'table', 'thead', 'tbody', 'tr', 'th', 'td',
        'pre', 'code', 'em', 'strong', 'del', 'span', 'a', 'img', 'div', 'ins', 'sub', 'sup'
      ],
      ALLOWED_ATTR: ['src', 'href', 'title', 'alt', 'class', 'id', 'align', 'valign', 'width', 'height', 'loading'],
      ALLOW_UNKNOWN_PROTOCOLS: false,
      ALLOWED_URI_REGEXP: /^(?:(?:https?|mailto|ftp|tel):|[^a-z0-9+.-]+(?:[/?#]|$))/i
    });
  }

  private unloadArticleBody(index: number) {
    const card = document.getElementById(`article-${index}`);
    if (!card || card.getAttribute('data-rendered') !== 'true') return;
    const body = card.querySelector('.article-body');
    if (!body) return;
    revokeArticleImages(this.articles[index].articleId);
    body.textContent = '';
    const placeholder = document.createElement('div');
    placeholder.className = 'article-loading-placeholder';
    placeholder.textContent = '正文已暂存，滚动回来时自动加载...';
    body.appendChild(placeholder);
    card.setAttribute('data-rendered', 'false');
  }

  private getNeighborIds(): string[] {
    const neighbors: string[] = [];
    const activeIndex = this.getSafeActiveIndex();
    if (activeIndex > 0) neighbors.push(this.articles[activeIndex - 1].articleId);
    if (activeIndex < this.articles.length - 1) neighbors.push(this.articles[activeIndex + 1].articleId);
    return neighbors;
  }

  private getSafeActiveIndex(): number {
    if (this.activeIndex >= 0 && this.activeIndex < this.articles.length) {
      return this.activeIndex;
    }

    const activeCard = document.querySelector('.article-card.active') as HTMLElement | null;
    const domIndex = Number(activeCard?.dataset.index);
    if (Number.isInteger(domIndex) && domIndex >= 0 && domIndex < this.articles.length) {
      this.activeIndex = domIndex;
      return domIndex;
    }

    this.activeIndex = 0;
    return 0;
  }

  private clearReaderMotionClasses() {
    document.body.classList.remove(
      'reader-motion-exit',
      'reader-motion-enter',
      'reader-motion-enter-active',
      'reader-motion-prev',
      'reader-motion-next'
    );
  }

  /**
   * 核心按需懒渲染调度：仅渲染当前 activeIndex 及其相邻的 [idx-1, idx+1] 文章
   * (不执行卸载/折叠逻辑)
   */
  private lazyRenderNeighborhood() {
    if (this.articles.length === 0) return;

    const active = this.getSafeActiveIndex();
    const total = this.articles.length;

    // 渲染邻域内的文章
    const renderPromises: Promise<void>[] = [];
    for (let i = active - 1; i <= active + 1; i++) {
      if (i >= 0 && i < total) {
        renderPromises.push(this.renderArticleBody(i));
      }
    }

    // 异步完成后管理图片缓存生命周期 (已渲染的文章正文保留在 DOM 中，不执行卸载)
    Promise.all(renderPromises).then(() => {
      const activeId = this.articles[active].articleId;
      manageImageCache(activeId, this.getNeighborIds());
    });
  }

  /**
   * 确保文章渲染。
   * accurate=false 时，只渲染 targetIndex 前后一篇，用于左右键相邻切换。
   * accurate=true 时，渲染 0 到 targetIndex 之间所有未渲染文章，用于跨篇/目录跳转/恢复进度等。
   * 只处理未渲染文章（data-rendered !== "true"），防 DOM 重复重建及图片重新绑定闪烁。
   */
  private async ensureRenderedForJump(targetIndex: number, accurate: boolean): Promise<void> {
    const unrenderedIndices: number[] = [];
    if (accurate) {
      for (let i = 0; i <= targetIndex; i++) {
        const card = document.getElementById(`article-${i}`);
        if (card && card.getAttribute('data-rendered') !== 'true') {
          unrenderedIndices.push(i);
        }
      }
    } else {
      for (let i = Math.max(0, targetIndex - 1); i <= Math.min(this.articles.length - 1, targetIndex + 1); i++) {
        const card = document.getElementById(`article-${i}`);
        if (card && card.getAttribute('data-rendered') !== 'true') {
          unrenderedIndices.push(i);
        }
      }
    }

    if (unrenderedIndices.length === 0) return;

    const showOverlay = unrenderedIndices.length > 3;
    let loadingIndicator: HTMLElement | null = null;
    if (showOverlay) {
      loadingIndicator = document.createElement('div');
      loadingIndicator.className = 'jump-loading-indicator';
      loadingIndicator.innerHTML = `
        <div class="jump-loading-content">
          <div class="spinner"></div>
          <div class="jump-loading-text">正在准备文档排版并精确计算定位...</div>
          <div class="jump-loading-progress">0 / ${unrenderedIndices.length}</div>
        </div>
      `;
      document.body.appendChild(loadingIndicator);
    }

    const batchSize = 3;
    for (let i = 0; i < unrenderedIndices.length; i += batchSize) {
      const batch = unrenderedIndices.slice(i, i + batchSize);
      await Promise.all(batch.map(idx => this.renderArticleBody(idx)));
      
      if (showOverlay && loadingIndicator) {
        const progressEl = loadingIndicator.querySelector('.jump-loading-progress');
        if (progressEl) {
          progressEl.textContent = `${Math.min(i + batchSize, unrenderedIndices.length)} / ${unrenderedIndices.length}`;
        }
      }
      
      // Yield 到主线程以保持 UI 响应
      await new Promise(resolve => setTimeout(resolve, 0));
    }

    if (loadingIndicator && loadingIndicator.parentNode) {
      loadingIndicator.parentNode.removeChild(loadingIndicator);
    }
  }

  private bindImageLightboxes(card: HTMLElement) {
    card.querySelectorAll('.article-body img').forEach(img => {
      const imageEl = img as HTMLImageElement;
      if (imageEl.dataset.bound) return;
      imageEl.dataset.bound = "true";
      imageEl.addEventListener('click', (e) => {
        e.stopPropagation();
        this.lightboxImg.src = imageEl.src;
        this.lightbox.classList.add('active');
      });
    });
  }

  /**
   * 监听并聚焦当前的活动文章，高亮侧边栏，更新进度
   */
  private updateActiveArticle(index: number, saveProgress: boolean = true, force: boolean = false) {
    if (index < 0 || index >= this.articles.length) return;
    
    if (!force && index === this.activeIndex) return;

    const prevIndex = this.activeIndex;
    if (this.mode.kind === 'served' && index > prevIndex && this.articles[prevIndex]) {
      this.servedReadIds.add(this.articles[prevIndex].articleId);
    }

    const prevActive = document.querySelectorAll('.article-card.active');
    prevActive.forEach(el => el.classList.remove('active'));

    const currentActive = document.getElementById(`article-${index}`);
    if (currentActive) {
      currentActive.classList.add('active');

      // 对目标文章卡片执行进入动画 (仅在实际发生卡片切换时)
      if (prevIndex !== index) {
        // 1. 播放动画前，先取消当前文章的所有活跃动画，防止冲突
        currentActive.getAnimations().forEach(a => a.cancel());

        // 2. 确定进入方向：下一篇从下方进入 (translateY(28px))，上一篇从上方进入 (translateY(-28px))
        const translateYVal = index > prevIndex ? 28 : -28;

        // 3. 执行 Web Animations API，最终 transform 对齐 CSS 中 .article-card.active 状态的 scale(1.01)
        currentActive.animate([
          {
            opacity: 0.58,
            transform: `translateY(${translateYVal}px) scale(0.985)`,
            filter: 'blur(6px)'
          },
          {
            opacity: 1,
            transform: 'translateY(0) scale(1.01)',
            filter: 'blur(0)'
          }
        ], {
          duration: 520,
          easing: 'cubic-bezier(0.22, 1, 0.36, 1)',
          fill: 'both'
        });
      }
    }

    const prevMenuItem = document.querySelectorAll('.menu-item.active');
    prevMenuItem.forEach(el => el.classList.remove('active'));
    
    const currentMenuItem = document.getElementById(`menu-item-${index}`);
    if (currentMenuItem) {
      currentMenuItem.classList.add('active');
      currentMenuItem.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
      this.moveMenuBubble(currentMenuItem);
    }

    this.activeIndex = index;
    
    // 更新邻域渲染
    this.lazyRenderNeighborhood();

    this.updateProgressText();
    
    if (saveProgress) {
      this.scheduleSaveCurrentProgress();
    }
  }

  private moveMenuBubble(targetEl: HTMLElement) {
    if (!targetEl || this.isSidebarActive === false) return;
    this.menuBubble.style.display = 'block';
    this.menuBubble.style.top = `${targetEl.offsetTop}px`;
    this.menuBubble.style.height = `${targetEl.offsetHeight}px`;
  }

  private getKeyboardBaseIndex(): number {
    if (this.pendingKeyboardIndex !== null) {
      return this.pendingKeyboardIndex;
    }
    if (this.scrollingToActiveIndex !== null) {
      return this.scrollingToActiveIndex;
    }
    return this.getSafeActiveIndex();
  }

  private handleKeyboardNav(delta: number) {
    const baseIndex = this.getKeyboardBaseIndex();

    if (this.pendingKeyboardIndex === null) {
      this.pendingKeyboardIndex = baseIndex;
    }
    
    const nextIndex = this.pendingKeyboardIndex + delta;
    if (nextIndex >= 0 && nextIndex < this.articles.length) {
      this.pendingKeyboardIndex = nextIndex;
    }
    
    if (this.keyboardNavTimer) {
      clearTimeout(this.keyboardNavTimer);
    }
    
    this.keyboardNavTimer = window.setTimeout(() => {
      const finalTarget = this.pendingKeyboardIndex;
      this.pendingKeyboardIndex = null;
      this.keyboardNavTimer = null;
      
      if (finalTarget !== null && finalTarget !== this.getSafeActiveIndex()) {
        const isAdjacent = Math.abs(finalTarget - this.getSafeActiveIndex()) === 1;
        // 如果是相邻文章切换，传入 accurate=false 只预加载前后一篇以提高速度；跨多篇则传入 accurate=true 以确保定位百分百精确
        this.scrollToArticle(finalTarget, undefined, undefined, true, !isAdjacent);
      }
    }, 75); // 75ms 防抖，合并连续快速按键
  }

  private async scrollToArticle(
    index: number,
    targetScrollTop?: number,
    headingAnchorId?: string,
    useTransition: boolean = false,
    accurate: boolean = true
  ) {
    if (index < 0 || index >= this.articles.length) return;

    // 增加转场事务序列号
    this.currentTransitionId++;
    const myTransitionId = this.currentTransitionId;

    // 强行打断任何正在播放的过渡，重置全局 body 转场类
    this.clearReaderMotionClasses();

    this.scrollingToActiveIndex = index;

    this.isScrollingLock = true;
    if (this.scrollLockTimer) clearTimeout(this.scrollLockTimer);

    try {
      // 1. 确保目标文章及其上方的所有文章都完成真实正文渲染，保证 DOM 高度真实
      await this.ensureRenderedForJump(index, accurate);

      // 并发检查：如果在渲染等待期间有新事务插队，直接退出
      if (this.currentTransitionId !== myTransitionId) {
        return;
      }

      const target = document.getElementById(`article-${index}`);
      if (!target) {
        return;
      }

      let scrollTargetElement: HTMLElement = target;
      if (headingAnchorId) {
        const anchorEl = target.querySelector(`#${headingAnchorId}`) as HTMLElement;
        if (anchorEl) scrollTargetElement = anchorEl;
      }

      if (scrollTargetElement === target) {
        // 按照优先级查找定位元素：.article-title > .article-header > .article-card
        const titleEl = target.querySelector('.article-title') as HTMLElement;
        const headerEl = target.querySelector('.article-header') as HTMLElement;
        if (titleEl) {
          scrollTargetElement = titleEl;
        } else if (headerEl) {
          scrollTargetElement = headerEl;
        }
      }

      const getElementDocumentTop = (el: HTMLElement): number => {
        let top = 0;
        let current: HTMLElement | null = el;
        while (current) {
          top += current.offsetTop;
          current = current.offsetParent as HTMLElement | null;
        }
        return top;
      };

      const calculateOffset = (): number => {
        if (typeof targetScrollTop === 'number' && !headingAnchorId) {
          // 仅在进度恢复且无标题锚点时使用相对 offsetTop 兜底
          return target.offsetTop + Math.min(targetScrollTop, target.offsetHeight);
        } else {
          const elementTop = getElementDocumentTop(scrollTargetElement);
          return Math.max(0, elementTop - 80); // 扣除 80px 顶部安全偏移量
        }
      };

      // 瞬间真实定位
      window.scrollTo({
        top: calculateOffset(),
        behavior: 'auto'
      });

      // 二次确认并发安全性
      if (this.currentTransitionId !== myTransitionId) {
        return;
      }

      // 强制触发高亮，确保 .active 挂载，并播放进入动画
      this.updateActiveArticle(index, true, true);

      // 图片加载后的二次定位校准
      const imgs = target.querySelectorAll('img');
      if (imgs.length > 0) {
        let calibrated = false;
        const runCalibration = () => {
          if (calibrated) return;
          calibrated = true;
          if (this.currentTransitionId !== myTransitionId) return; // 确认事务未过期
          
          // 双重 rAF 确保浏览器已完成这一轮 DOM 重排和首屏渲染，让 offset 的计算完美精准
          requestAnimationFrame(() => {
            requestAnimationFrame(() => {
              if (this.currentTransitionId !== myTransitionId) return;
              window.scrollTo({
                top: calculateOffset(),
                behavior: 'auto'
              });
            });
          });
        };

        let loadedCount = 0;
        const handleImgLoad = () => {
          loadedCount++;
          if (loadedCount === imgs.length) {
            runCalibration();
          }
        };

        imgs.forEach(img => {
          const imgEl = img as HTMLImageElement;
          if (imgEl.complete) {
            handleImgLoad();
          } else {
            imgEl.addEventListener('load', handleImgLoad, { once: true });
            imgEl.addEventListener('error', handleImgLoad, { once: true });
          }
        });

        // 350ms 兜底超时
        setTimeout(runCalibration, 350);
      }

    } finally {
      // 任意早退或异常都必须释放本事务留下的滚动锁与清理。
      if (this.currentTransitionId === myTransitionId) {
        this.scrollingToActiveIndex = null;
        this.clearReaderMotionClasses();
        this.scrollLockTimer = window.setTimeout(() => {
          if (this.currentTransitionId === myTransitionId) {
            this.isScrollingLock = false;
          }
        }, 150);
      }
    }
  }

  private updateProgressText() {
    const total = this.articles.length;
    const current = this.getSafeActiveIndex() + 1;
    this.floatingProgress.textContent = `${current} / ${total}`;
  }

  /**
   * 节流的滚动处理函数，用于计算视口高亮文章
   */
  /**
   * Post a private, versioned activity message to the parent ImmersiveReader shell.
   * Throttled by the existing scroll RAF so high-frequency scroll does not spam.
   */
  private notifyParentReadingActivity() {
    if (window.parent === window) return;
    try {
      window.parent.postMessage(
        {
          source: 'immersive-reader-flow',
          version: 1,
          type: 'reading-activity',
        },
        '*'
      );
    } catch {
      // Cross-origin edge cases — ignore.
    }
  }

  private handleScrollThrottled() {
    this.updateProgressBar();

    if (this.isScrollingLock) return;

    // 查找当前视口中线所占主体最长的文章
    const viewportMiddle = window.scrollY + window.innerHeight * 0.4;
    const cards = Array.from(document.querySelectorAll('.article-card'));
    
    let closestIndex = this.getSafeActiveIndex();
    let minDistance = Infinity;

    for (const card of cards) {
      const index = parseInt(card.getAttribute('data-index') || '0', 10);
      const top = (card as HTMLElement).offsetTop;
      const bottom = top + (card as HTMLElement).offsetHeight;

      if (viewportMiddle >= top && viewportMiddle <= bottom) {
        closestIndex = index;
        break;
      }

      const dist = Math.min(Math.abs(viewportMiddle - top), Math.abs(viewportMiddle - bottom));
      if (dist < minDistance) {
        minDistance = dist;
        closestIndex = index;
      }
    }

    if (closestIndex !== this.getSafeActiveIndex()) {
      this.updateActiveArticle(closestIndex);
    } else {
      // 仅当 activeIndex 没变但滚动高度变了，也触发进度保存
      this.scheduleSaveCurrentProgress();
    }
  }

  private scheduleSaveCurrentProgress() {
    if (this.saveProgressTimer) {
      clearTimeout(this.saveProgressTimer);
    }
    this.saveProgressTimer = window.setTimeout(() => {
      this.saveProgressTimer = null;
      this.saveCurrentProgress();
    }, 500);
  }

  private updateProgressBar() {
    const docHeight = document.documentElement.scrollHeight - window.innerHeight;
    const scrolled = docHeight > 0 ? (window.scrollY / docHeight) * 100 : 0;
    this.topProgress.style.width = `${scrolled}%`;
  }

  private saveCurrentProgress() {
    const activeIndex = this.getSafeActiveIndex();
    const card = document.getElementById(`article-${activeIndex}`);
    if (!card) return;

    // 计算当前卡片正文之下的相对 scrollTop
    const currentCardScrollTop = Math.max(0, window.scrollY - (card as HTMLElement).offsetTop);
    
    // 获取当前视口内最近的标题 Heading
    let headingAnchor = '';
    const headings = card.querySelectorAll('h1, h2, h3, h4, h5, h6');
    let bestHeading: HTMLElement | null = null;
    
    for (const h of Array.from(headings)) {
      const rect = h.getBoundingClientRect();
      const dist = rect.top - 85;
      if (dist <= 0) {
        // 在视口顶部安全线之上，记录最后一个（即最靠近视口顶部的那个）
        bestHeading = h as HTMLElement;
      } else if (dist < 100 && !bestHeading) {
        // 如果上面没有，退而求其次找视口内最近的下方标题
        bestHeading = h as HTMLElement;
      }
    }
    if (bestHeading) {
      headingAnchor = bestHeading.id || '';
    }

    if (this.mode.kind === 'served') {
      const available = Math.max(1, (card as HTMLElement).offsetHeight - window.innerHeight);
      const position = Math.max(0, Math.min(1, currentCardScrollTop / available));
      if (activeIndex === this.articles.length - 1 && position >= 0.95) {
        this.servedReadIds.add(this.articles[activeIndex].articleId);
      }
      void this.mode.saveProgress({
        schemaVersion: 1,
        current: this.articles[activeIndex].articleId,
        position,
        read: [...this.servedReadIds],
        updated: new Date().toISOString(),
      }).catch((error) => console.error('同步阅读进度失败:', error));
      return;
    }

    saveReadingProgress(this.sourceId, {
      sourceId: this.sourceId,
      articleId: this.articles[activeIndex].articleId,
      scrollTop: currentCardScrollTop,
      headingAnchor
    });
  }

  private async restoreProgress() {
    if (this.mode.kind === 'served') {
      try {
        const progress = await this.mode.loadProgress();
        this.servedReadIds = new Set(progress.read);
        let targetIndex = this.articles.findIndex(art => art.articleId === progress.current);
        if (targetIndex < 0) targetIndex = this.articles.findIndex(art => !this.servedReadIds.has(art.articleId));
        if (targetIndex < 0) targetIndex = 0;
        this.updateActiveArticle(targetIndex, false, true);
        await this.ensureRenderedForJump(targetIndex, false);
        const card = document.getElementById(`article-${targetIndex}`);
        if (card) {
          const available = Math.max(0, (card as HTMLElement).offsetHeight - window.innerHeight);
          window.scrollTo({ top: (card as HTMLElement).offsetTop + available * progress.position });
        }
      } catch (error) {
        console.error('读取共享进度失败:', error);
        this.updateActiveArticle(0, false, true);
      }
      return;
    }
    const progress = getReadingProgress(this.sourceId);
    if (!progress) {
      // 无进度，高亮第 0 篇
      this.updateActiveArticle(0, false, true);
      return;
    }

    // 优先匹配 articleId
    let targetIndex = this.articles.findIndex(art => art.articleId === progress.articleId);
    if (targetIndex === -1) {
      targetIndex = 0;
    }

    // 先建立稳定的 activeIndex，再异步做高精度定位。
    // 旧逻辑会临时置为 -1，恢复跳转期间按左右键会从错误索引计算目标文章。
    this.updateActiveArticle(targetIndex, false, true);

    // 异步高精度定位，优先基于 headingAnchor 恢复
    setTimeout(() => {
      this.scrollToArticle(targetIndex, progress.scrollTop, progress.headingAnchor);
    }, 100);
  }

  private adjustFontSize(delta: number) {
    this.fontSize = Math.max(15, Math.min(32, this.fontSize + delta));
    document.documentElement.style.setProperty('--p-font-size', `${this.fontSize}px`);
  }

  private openSidebar() {
    this.isSidebarActive = true;
    this.sidebar.classList.add('active');
    this.sidebarOverlay.classList.add('active');
    this.sidebar.removeAttribute('inert');
    this.sidebar.setAttribute('aria-hidden', 'false');
    this.menuTrigger.setAttribute('aria-expanded', 'true');
    setTimeout(() => {
      const activeItem = document.getElementById(`menu-item-${this.getSafeActiveIndex()}`);
      if (activeItem) this.moveMenuBubble(activeItem);
      this.sidebarSearch.focus();
    }, 100);
  }

  private closeSidebar() {
    const restoreTriggerFocus = this.sidebar.contains(document.activeElement);
    this.isSidebarActive = false;
    this.sidebar.classList.remove('active');
    this.sidebarOverlay.classList.remove('active');
    this.sidebar.setAttribute('inert', '');
    this.sidebar.setAttribute('aria-hidden', 'true');
    this.menuTrigger.setAttribute('aria-expanded', 'false');
    if (restoreTriggerFocus) this.menuTrigger.focus();
  }

  private toggleSidebar() {
    if (this.isSidebarActive) this.closeSidebar();
    else this.openSidebar();
  }

  private openSearch() {
    const activeElement = document.activeElement;
    this.searchReturnFocus = activeElement instanceof HTMLElement && activeElement !== document.body
      ? activeElement
      : this.menuTrigger;
    this.closeSidebar();
    this.isSearchActive = true;
    this.searchOverlay.removeAttribute('inert');
    this.searchOverlay.classList.add('active');
    this.searchOverlay.setAttribute('aria-hidden', 'false');
    this.paletteInput.value = '';
    this.paletteInput.focus();
    this.renderSearchResults('');
  }

  private closeSearch() {
    this.isSearchActive = false;
    this.searchOverlay.classList.remove('active');
    this.searchOverlay.setAttribute('inert', '');
    this.searchOverlay.setAttribute('aria-hidden', 'true');
    this.paletteInput.blur();
    this.searchReturnFocus?.focus();
    this.searchReturnFocus = null;
  }

  private renderSearchResults(query: string) {
    this.paletteResults.innerHTML = '';
    this.searchFiltered = searchArticles(query, this.searchIndex).slice(0, 15);
    this.searchSelectedIndex = 0;

    if (this.searchFiltered.length === 0) {
      const noResult = document.createElement('div');
      noResult.className = 'palette-no-results';
      noResult.textContent = '没有找到匹配的文章';
      this.paletteResults.appendChild(noResult);
      return;
    }

    this.searchFiltered.forEach((item, idx) => {
      const el = document.createElement('button');
      el.type = 'button';
      el.className = `palette-item ${idx === 0 ? 'selected' : ''}`;
      el.onclick = () => {
        this.scrollToArticle(item.index);
        this.closeSearch();
      };

      const art = this.articles[item.index];
      const title = document.createElement('span');
      title.className = 'palette-item-title';
      title.textContent = art.title;
      const meta = document.createElement('div');
      meta.className = 'palette-item-meta';
      const date = document.createElement('span');
      date.textContent = art.date;
      const words = document.createElement('span');
      words.textContent = `字数 ${art.wordCount}`;
      meta.append(date, words);
      el.append(title, meta);
      this.paletteResults.appendChild(el);
    });
  }

  private shouldIgnoreGlobalShortcut(e: KeyboardEvent): boolean {
    // 快捷键过滤增强
    if (e.defaultPrevented) return true;
    if (e.ctrlKey || e.metaKey || e.altKey) return true;

    const target = e.target as HTMLElement;
    if (!target) return false;

    // 1. 如果正在输入，忽略所有全局快捷键 (Escape 退出焦点由于单独提出来提前捕获，不被此处忽略)
    const activeElement = document.activeElement;
    if (activeElement) {
      const activeTagName = activeElement.tagName.toLowerCase();
      const isEditing = 
        activeTagName === 'input' || 
        activeTagName === 'textarea' || 
        activeTagName === 'select' || 
        activeElement.getAttribute('contenteditable') === 'true';
      
      if (isEditing) {
        return true;
      }
    }

    // 2. 如果 e.target.closest(...)，对于 ArrowLeft 和 ArrowRight 限制触发
    if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
      if (target && typeof target.closest === 'function' && target.closest('pre, code, table, a, button, input, textarea, select')) {
        return true;
      }
    }

    return false;
  }

  private bindEvents() {
    // 遮罩点击
    this.sidebarOverlay.onclick = () => this.closeSidebar();
    this.menuTrigger.onclick = () => this.toggleSidebar();

    window.addEventListener('scroll', () => {
      if (this.scrollRaf === null) {
        this.scrollRaf = window.requestAnimationFrame(() => {
          this.scrollRaf = null;
          this.handleScrollThrottled();
          // Notify parent desktop shell (cross-origin iframe) of reading activity.
          // Parent owns the top-edge chrome hotzone; we only report activity.
          this.notifyParentReadingActivity();
        });
      }
    }, { passive: true });

    // 搜索输入
    this.paletteInput.addEventListener('input', (e) => {
      this.renderSearchResults((e.target as HTMLInputElement).value);
    });

    // 侧栏实时搜索过滤
    this.sidebarSearch.addEventListener('input', (e) => {
      const q = (e.target as HTMLInputElement).value.toLowerCase();
      const menuItems = this.menu.querySelectorAll('.menu-item');
      menuItems.forEach((item) => {
        const text = item.textContent?.toLowerCase() || '';
        if (text.includes(q)) {
          (item as HTMLElement).style.display = 'block';
        } else {
          (item as HTMLElement).style.display = 'none';
        }
      });
    });

    // 监听内链跳转事件
    document.addEventListener('local-navigate', (e: any) => {
      const rawHref = e.detail?.rawHref;
      if (!rawHref) return;
      
      // 统一解析出的目标路径，处理斜杠及相对路径回退
      const decodedHref = decodeURIComponent(rawHref).replace(/\\/g, '/');
      const activeIndex = this.getSafeActiveIndex();
      const currentPath = this.articles[activeIndex].relativePath || this.articles[activeIndex].frontMatter.path || '';
      
      const resolvedPath = (decodedHref.startsWith('.') || !decodedHref.includes('/')) ?
        resolveRelativePath(currentPath, decodedHref) : decodedHref;
        
      const targetFilename = decodedHref.split('/').pop() || '';
      
      // 在当前内存中的 articles 数组中按照优先级查找匹配项
      // 优先级: 1. relativePath, 2. frontMatter.path, 3. filename
      let targetIdx = this.articles.findIndex(art => 
        (art.relativePath && art.relativePath.replace(/\\/g, '/') === resolvedPath) ||
        (art.frontMatter?.path && art.frontMatter.path.replace(/\\/g, '/') === resolvedPath)
      );
      
      if (targetIdx === -1) {
        // 尝试只匹配文件名
        targetIdx = this.articles.findIndex(art => 
          (art.filename && art.filename === targetFilename) ||
          (art.relativePath && art.relativePath.split('/').pop() === targetFilename)
        );
      }
      
      if (targetIdx !== -1) {
        this.scrollToArticle(targetIdx);
      } else {
        // 如果未在 articles 中找到，且有 filesMap (即在 Universal 模式下)，尝试使用 filesMap 匹配
        let matchedFile = this.filesMap.get(resolvedPath);
        if (!matchedFile) {
          matchedFile = this.filesMap.get(decodedHref);
        }
        if (matchedFile) {
          const idx = this.articles.findIndex(art => art.relativePath === matchedFile!.relativePath);
          if (idx !== -1) {
            this.scrollToArticle(idx);
          }
        }
      }
    });

    // 退出放大灯箱
    this.lightbox.onclick = () => this.lightbox.classList.remove('active');

    // 键盘快捷键监听
    window.addEventListener('keydown', (e) => {
      // 1. 放大灯箱 ESC 退出
      if (this.lightbox.classList.contains('active')) {
        if (e.key === 'Escape') {
          this.lightbox.classList.remove('active');
          e.preventDefault();
        }
        return;
      }

      // 2. 搜索 Command Palette 活跃时专属快捷键 (保留其原生的上下及回车、Esc 逻辑)
      if (this.isSearchActive) {
        if (e.key === 'Escape') {
          this.closeSearch();
          e.preventDefault();
        } else if (e.key === 'Tab') {
          const focusable = Array.from(this.searchOverlay.querySelectorAll<HTMLElement>('input, button'))
            .filter((element) => !element.hasAttribute('disabled'));
          const currentIndex = focusable.indexOf(document.activeElement as HTMLElement);
          const nextIndex = e.shiftKey
            ? (currentIndex <= 0 ? focusable.length - 1 : currentIndex - 1)
            : (currentIndex >= focusable.length - 1 ? 0 : currentIndex + 1);
          focusable[nextIndex]?.focus();
          e.preventDefault();
        } else if (e.key === 'ArrowDown') {
          e.preventDefault();
          const items = this.paletteResults.querySelectorAll('.palette-item');
          if (items.length > 0) {
            items[this.searchSelectedIndex].classList.remove('selected');
            this.searchSelectedIndex = (this.searchSelectedIndex + 1) % items.length;
            items[this.searchSelectedIndex].classList.add('selected');
            (items[this.searchSelectedIndex] as HTMLElement).scrollIntoView({ block: 'nearest' });
          }
        } else if (e.key === 'ArrowUp') {
          e.preventDefault();
          const items = this.paletteResults.querySelectorAll('.palette-item');
          if (items.length > 0) {
            items[this.searchSelectedIndex].classList.remove('selected');
            this.searchSelectedIndex = (this.searchSelectedIndex - 1 + items.length) % items.length;
            items[this.searchSelectedIndex].classList.add('selected');
            (items[this.searchSelectedIndex] as HTMLElement).scrollIntoView({ block: 'nearest' });
          }
        } else if (e.key === 'Enter') {
          e.preventDefault();
          if (this.searchFiltered.length > 0) {
            const item = this.searchFiltered[this.searchSelectedIndex];
            this.scrollToArticle(item.index);
            this.closeSearch();
          }
        }
        return;
      }

      // 3. 输入框/文本域焦点状态下，按 Escape 退焦并关闭侧栏目录 (在全局过滤之前，避免 Esc 被吞)
      if (e.key === 'Escape') {
        const act = document.activeElement;
        if (act && (act.tagName === 'INPUT' || act.tagName === 'TEXTAREA')) {
          this.closeSidebar();
          e.preventDefault();
          return;
        }
      }

      // 4. 判定是否应当过滤该次全局快捷键交互 (输入框过滤、元素内左右键过滤、组合键/默认拦截过滤)
      if (this.shouldIgnoreGlobalShortcut(e)) {
        return;
      }

      // 5. 正常的全局阅读控制快捷键
      switch (e.key) {
        case 'ArrowLeft':
        case 'k':
        case 'K':
          e.preventDefault();
          this.handleKeyboardNav(-1);
          break;

        case 'ArrowRight':
        case 'j':
        case 'J':
          e.preventDefault();
          this.handleKeyboardNav(1);
          break;

        case '=':
        case '+':
          e.preventDefault();
          this.adjustFontSize(1);
          break;

        case '-':
        case '_':
          e.preventDefault();
          this.adjustFontSize(-1);
          break;

        case 'm':
        case 'M':
          e.preventDefault();
          this.toggleSidebar();
          break;

        case '/':
        case 's':
        case 'S':
          e.preventDefault();
          this.openSearch();
          break;

        case 'Escape':
          this.closeSidebar();
          break;
      }
    });
  }
}

// 辅助本地相对路径解析，直接从 image-resolver.ts 提取以解决循环依赖
function resolveRelativePath(basePath: string, relativePath: string): string {
  const cleanRel = relativePath.replace(/\\/g, '/');
  const baseParts = basePath.split('/');
  baseParts.pop();

  const relParts = cleanRel.split('/');
  for (const part of relParts) {
    if (part === '.' || part === '') {
      continue;
    } else if (part === '..') {
      baseParts.pop();
    } else {
      baseParts.push(part);
    }
  }
  return baseParts.join('/');
}
