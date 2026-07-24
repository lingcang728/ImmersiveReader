import type { BookManifest } from '../../../../../packages/contracts/dist/index.js';
import type { ArticleMetadata } from '../core/metadata.js';
import type { ReaderMode, SharedReadingState } from './reader-mode.js';

export interface ServedModeData {
  articles: ArticleMetadata[];
  sourceId: string;
  sourceName: string;
  mode: ReaderMode;
}

export const SERVED_SESSION_HEARTBEAT_MS = 2 * 60 * 1000;

export function buildServedContentUrl(base: string, relativePath: string): string {
  const encoded = relativePath.split('/').map((segment) => encodeURIComponent(segment)).join('/');
  return `${base.replace(/\/$/, '')}/${encoded}`;
}

export function manifestToArticles(manifest: BookManifest): ArticleMetadata[] {
  return manifest.chapters.map((chapter) => ({
    articleId: chapter.id,
    relativePath: chapter.path,
    filename: chapter.path.split('/').at(-1) ?? chapter.path,
    title: chapter.title,
    date: chapter.date ?? '未知日期',
    timestamp: chapter.date ? Date.parse(chapter.date) || 0 : 0,
    author: manifest.title,
    summary: '',
    wordCount: chapter.wordCount,
    upvoteCount: chapter.voteCount || undefined,
    frontMatter: { path: chapter.path },
  }));
}

function sessionBase(pathname: string): string | null {
  const match = pathname.match(/^(\/s\/[^/]+)\/reader\/?$/);
  return match?.[1] ?? null;
}

async function fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
  const response = await fetch(url, { cache: 'no-store', ...init });
  if (!response.ok) {
    throw new Error(`${response.status} ${await response.text()}`);
  }
  return await response.json() as T;
}

export function buildServedHeartbeatUrl(base: string): string {
  return `${base.replace(/\/$/, '')}/heartbeat`;
}

function keepServedSessionAlive(base: string): void {
  const heartbeatUrl = buildServedHeartbeatUrl(base);
  const heartbeat = () => {
    void fetch(heartbeatUrl, { cache: 'no-store' }).catch((error) => {
      console.warn('连读会话保活失败，将在下次周期重试:', error);
    });
  };
  const timer = window.setInterval(heartbeat, SERVED_SESSION_HEARTBEAT_MS);
  const handleVisibilityChange = () => {
    if (document.visibilityState === 'visible') heartbeat();
  };
  document.addEventListener('visibilitychange', handleVisibilityChange);
  window.addEventListener('pagehide', () => {
    window.clearInterval(timer);
    document.removeEventListener('visibilitychange', handleVisibilityChange);
  }, { once: true });
}

export async function loadServedMode(pathname = window.location.pathname): Promise<ServedModeData | null> {
  const base = sessionBase(pathname);
  if (!base) return null;
  const manifest = await fetchJson<BookManifest>(`${base}/manifest`);
  keepServedSessionAlive(base);
  document.title = `${manifest.title} - 沉浸阅读`;
  const mode: ReaderMode = {
    kind: 'served',
    contentBase: `${base}/content`,
    loadProgress: () => fetchJson<SharedReadingState>(`${base}/progress`),
    saveProgress: async (progress) => {
      const response = await fetch(`${base}/progress`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(progress),
      });
      if (!response.ok) {
        throw new Error(`${response.status} ${await response.text()}`);
      }
    },
  };
  return {
    articles: manifestToArticles(manifest),
    sourceId: manifest.bookId,
    sourceName: manifest.title,
    mode,
  };
}
