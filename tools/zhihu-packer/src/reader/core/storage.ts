const DB_NAME = 'markdown-reader-db';
const STORE_NAME = 'handles-store';
const DB_VERSION = 1;

export interface ReadingProgress {
  sourceId: string;
  articleId: string;
  scrollTop: number;
  headingAnchor: string;
  timestamp: number;
  schemaVersion: number;
}

/**
 * 初始化并打开原生 IndexedDB
 */
function openDB(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, DB_VERSION);
    request.onerror = () => reject(request.error);
    request.onsuccess = () => resolve(request.result);
    request.onupgradeneeded = (e) => {
      const db = request.result;
      if (!db.objectStoreNames.contains(STORE_NAME)) {
        db.createObjectStore(STORE_NAME);
      }
    };
  });
}

/**
 * 将 FileSystemDirectoryHandle 保存到 IndexedDB
 */
export async function saveDirectoryHandle(
  sourceId: string,
  handle: FileSystemDirectoryHandle
): Promise<void> {
  const db = await openDB();
  return new Promise((resolve, reject) => {
    const transaction = db.transaction(STORE_NAME, 'readwrite');
    const store = transaction.objectStore(STORE_NAME);
    const request = store.put(handle, sourceId);

    request.onerror = () => reject(request.error);
    transaction.onerror = () => reject(transaction.error);
    transaction.onabort = () => reject(transaction.error || new Error('IndexedDB transaction aborted'));
    transaction.oncomplete = () => {
      localStorage.setItem('last_active_source_id', sourceId);
      resolve();
    };
  });
}

/**
 * 从 IndexedDB 中读取指定的 FileSystemDirectoryHandle
 */
export async function getDirectoryHandle(
  sourceId: string
): Promise<FileSystemDirectoryHandle | null> {
  try {
    const db = await openDB();
    return new Promise((resolve, reject) => {
      const transaction = db.transaction(STORE_NAME, 'readonly');
      const store = transaction.objectStore(STORE_NAME);
      const request = store.get(sourceId);
      request.onsuccess = () => resolve(request.result || null);
      request.onerror = () => reject(request.error);
    });
  } catch (err) {
    console.error('从 IndexedDB 读取文件夹句柄失败:', err);
    return null;
  }
}

/**
 * 获取最后一次活动的 Source ID
 */
export function getLastActiveSourceId(): string | null {
  return localStorage.getItem('last_active_source_id');
}

/**
 * 保存文件夹别名 (比如显示上次恢复的文件夹名称)
 */
export function saveSourceFolderName(sourceId: string, name: string): void {
  localStorage.setItem(`source_name_${sourceId}`, name);
}

export function getSourceFolderName(sourceId: string): string {
  return localStorage.getItem(`source_name_${sourceId}`) || '未命名文件夹';
}

/**
 * 校验并授权 FileSystemDirectoryHandle 的读取权限
 * @returns 是否成功获得授权
 */
export async function verifyHandlePermission(
  handle: FileSystemDirectoryHandle,
  withPrompt: boolean = false
): Promise<boolean> {
  try {
    const opts = { mode: 'read' as const };
    
    // 1. 先查询当前权限
    if (handle.queryPermission && (await handle.queryPermission(opts)) === 'granted') {
      return true;
    }
    
    // 2. 如果需要弹窗，且当前未授权，则申请权限 (必须由用户点击事件触发)
    if (withPrompt && handle.requestPermission) {
      if ((await handle.requestPermission(opts)) === 'granted') {
        return true;
      }
    }
    
    return false;
  } catch (e) {
    console.error('授权校验失败，句柄可能已失效:', e);
    return false;
  }
}

/**
 * 保存阅读进度
 */
export function saveReadingProgress(
  sourceId: string,
  progress: Omit<ReadingProgress, 'timestamp' | 'schemaVersion'>
): void {
  const fullProgress: ReadingProgress = {
    ...progress,
    timestamp: Date.now(),
    schemaVersion: 1
  };
  localStorage.setItem(`progress_${sourceId}`, JSON.stringify(fullProgress));
}

/**
 * 获取阅读进度
 */
export function getReadingProgress(sourceId: string): ReadingProgress | null {
  const raw = localStorage.getItem(`progress_${sourceId}`);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as ReadingProgress;
    if (parsed && parsed.schemaVersion === 1) {
      return parsed;
    }
  } catch (e) {
    console.warn(`解析进度失败 (key=progress_${sourceId})`, e);
  }
  return null;
}

/**
 * 清除指定 Source 的进度
 */
export function clearReadingProgress(sourceId: string): void {
  localStorage.removeItem(`progress_${sourceId}`);
}
