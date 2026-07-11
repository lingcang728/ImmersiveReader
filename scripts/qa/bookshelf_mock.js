(() => {
  let nextCallback = 1;
  const callbacks = new Map();
  const books = [
    { bookId: 'zhihu:zombie', title: '你的ZombieMan · 知乎归档', source: 'zhihu', sourceId: 'zombie', chapterCount: 388, readCount: 37, progress: 0.42, currentChapterId: 'a1', currentChapterTitle: '如何评价当代人「永久在线」的生存状态？', lastReadAt: '2026-07-09T23:41:00+08:00' },
    { bookId: 'zhihu:jonathan', title: 'Jonathan Z · 知乎归档', source: 'zhihu', sourceId: 'jonathan', chapterCount: 163, readCount: 1, progress: 0.01, currentChapterId: 'a2', currentChapterTitle: '生理性喜欢会有怎样的表现？', lastReadAt: '2026-07-10T08:13:00+08:00' },
    { bookId: 'zhihu:moriarty', title: '茶花路莫里亚蒂 · 知乎归档', source: 'zhihu', sourceId: 'moriarty', chapterCount: 918, readCount: 0, progress: 0, currentChapterId: null, currentChapterTitle: null, lastReadAt: null }
  ];
  const invoke = async (command) => {
    const state = new URLSearchParams(location.search).get('state') || 'ready';
    if (command === 'get_app_settings') return { schemaVersion: 1, libraryRoot: 'C:\\Users\\15pro\\Documents\\沉浸阅读\\Library', companionRoot: 'C:\\Users\\15pro\\Desktop\\MyProject\\ImmersiveReader', temporaryRoots: [] };
    if (command === 'scan_library') {
      if (state === 'loading') await new Promise(resolve => setTimeout(resolve, 1800));
      if (state === 'empty') return { books: [], issues: [], writable: true };
      if (state === 'error') return { books, issues: [{ path: 'C:\\损坏书目\\manifest.json', message: 'manifest.json 无法解析' }], writable: false };
      return { books, issues: [], writable: true };
    }
    if (command === 'list_temporary_content') return [];
    if (command === 'load_recent_files') return { json: '[]', store_exists: true };
    if (command === 'plugin:event|listen') return nextCallback++;
    return null;
  };
  window.__TAURI_INTERNALS__ = {
    invoke,
    metadata: {
      currentWindow: { label: 'main' },
      currentWebview: { windowLabel: 'main', label: 'main' }
    },
    convertFileSrc: path => path,
    transformCallback: callback => {
      const id = nextCallback++;
      callbacks.set(id, callback);
      window[`_${id}`] = callback;
      return id;
    },
    unregisterCallback: id => {
      callbacks.delete(id);
      delete window[`_${id}`];
    }
  };
})();
