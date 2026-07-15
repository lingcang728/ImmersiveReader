(() => {
  let nextCallback = 1;
  const callbacks = new Map();
  const books = [
    { bookId: 'zhihu:zombie', title: '你的ZombieMan · 知乎归档', source: 'zhihu', sourceId: 'zombie', chapterCount: 388, readCount: 37, progress: 0.42, currentChapterId: 'a1', currentChapterTitle: '如何评价当代人「永久在线」的生存状态？', lastReadAt: '2026-07-09T23:41:00+08:00' },
    { bookId: 'zhihu:jonathan', title: 'Jonathan Z · 知乎归档', source: 'zhihu', sourceId: 'jonathan', chapterCount: 163, readCount: 1, progress: 0.01, currentChapterId: 'a2', currentChapterTitle: '生理性喜欢会有怎样的表现？', lastReadAt: '2026-07-10T08:13:00+08:00' },
    { bookId: 'zhihu:moriarty', title: '茶花路莫里亚蒂 · 知乎归档', source: 'zhihu', sourceId: 'moriarty', chapterCount: 918, readCount: 0, progress: 0, currentChapterId: null, currentChapterTitle: null, lastReadAt: null }
  ];
  const invoke = async (command, args) => {
    const state = new URLSearchParams(location.search).get('state') || 'ready';
    if (command === 'get_app_settings') return { schemaVersion: 1, libraryRoot: 'C:\\Users\\15pro\\Documents\\沉浸阅读\\Library', companionRoot: 'C:\\Users\\15pro\\Desktop\\MyProject\\ImmersiveReader', temporaryRoots: [] };
    if (command === 'scan_library') {
      if (state === 'loading') await new Promise(resolve => setTimeout(resolve, 1800));
      if (state === 'empty') return { books: [], issues: [], writable: true };
      if (state === 'error') return { books, issues: [{ path: 'C:\\损坏书目\\manifest.json', message: 'manifest.json 无法解析' }], writable: false };
      return { books, issues: [], writable: true };
    }
    if (command === 'list_temporary_content') return [];
    if (command === 'list_trash') return [];
    if (command === 'get_acquisition_snapshot') return { tasks: [], recoverableCacheBytes: 0, generatedAt: new Date().toISOString() };
    if (command === 'open_book') {
      const book = books.find(item => item.bookId === args?.bookId) || books[0];
      return {
        manifest: {
          schemaVersion: 1,
          bookId: book.bookId,
          title: book.title,
          source: book.source,
          sourceId: book.sourceId,
          generatedAt: '2026-07-10T08:00:00Z',
          updatedAt: '2026-07-10T08:13:00Z',
          chapters: [
            { id: 'a1', path: '001.md', title: '第一篇回答', date: '2026-07-10', voteCount: 12, wordCount: 420 },
            { id: 'a2', path: '002.md', title: '第二篇回答', date: '2026-07-10', voteCount: 8, wordCount: 280 }
          ]
        },
        progress: { schemaVersion: 1, current: 'a1', position: 0.42, read: [], updated: '2026-07-10T08:13:00Z' },
        provenance: {
          schemaVersion: 1,
          bookId: book.bookId,
          sourceId: book.sourceId,
          sourceKind: book.source,
          createdByTaskId: 'zhihu-task-1',
          lastSuccessfulTaskId: 'zhihu-task-1',
          revision: 2,
          manifestSha256: '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
          engineVersion: 'zhihu-test-engine',
          updatedAt: '2026-07-10T08:13:00Z'
        },
        taskRecords: [{
          id: 'zhihu-task-1',
          kind: 'zhihu',
          revision: 2,
          lastSequence: 4,
          lifecycleState: 'terminal',
          outcome: 'success',
          requiredAction: 'none',
          progress: { mode: 'determinate', percent: 100, completedUnits: 5, totalUnits: 5, label: 'writing_output' },
          engineStage: 'published',
          engineStatus: 'completed',
          recoverable: false,
          canPause: false,
          canResume: false,
          canRetry: false,
          canCancel: false,
          bookId: book.bookId,
          sourceId: book.sourceId,
          cacheLeaseBytes: 0,
          createdAt: '2026-07-10T08:00:00Z',
          updatedAt: '2026-07-10T08:13:00Z'
        }]
      };
    }
    if (command === 'load_recent_files') return { json: '[]', store_exists: true };
    if (command === 'get_storage_locations') {
      return {
        channel: 'qa',
        settingsPath: 'C:\\qa\\settings.json',
        dataRoot: 'C:\\qa\\Data',
        cacheRoot: 'C:\\qa\\Cache',
        logsRoot: 'C:\\qa\\Logs',
        runtimeStateRoot: 'C:\\qa\\RuntimeState',
        backupsRoot: 'C:\\qa\\Backups',
        libraryRoot: 'C:\\qa\\Library',
        runtimeRoot: 'C:\\qa\\runtime'
      };
    }
    if (command === 'get_secret_status') {
      return { configured: false, maskedHint: null, lastVerifiedAt: null };
    }
    if (command === 'get_storage_usage') {
      return {
        libraryBytes: 0,
        dataBytes: 0,
        cacheBytes: 0,
        logsBytes: 0,
        backupsBytes: 0,
        runtimeStateBytes: 0
      };
    }
    if (command === 'get_publish_recovery_status') return [];
    if (command === 'get_migration_runs') return [];
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
