// QA mock for the Android-first mobile surface (task-7).
//
// Injected by scripts/qa/mobile/verify_mobile.py via Playwright
// addInitScript() so the built SPA can run in desktop Chrome without a Tauri
// shell. It mirrors the real wire shapes produced by apps/desktop/src-tauri
// (serde camelCase payloads) and the command list used by
// apps/desktop/src/lib/mobile/api.ts + lib/epub/ipc.ts.
//
// Never shipped: QA asset only. No real user data — every book/chapter below
// is synthetic fixture text.
(() => {
  "use strict";

  const QUERY = new URLSearchParams(typeof location !== "undefined" ? location.search : "");
  const STATE = QUERY.get("state") || "ready";
  const MODE = QUERY.get("mode") || "normal";

  const iso = (offsetDays = 0) =>
    new Date(Date.now() + offsetDays * 86400000).toISOString();

  const mdBook1 = {
    bookId: "md-shan-yue",
    title: "山月记·节选",
    source: "manual",
    chapterCount: 3,
    readCount: 1,
    progress: 0.34,
    currentChapterTitle: "第一章 山月",
    updatedAt: "2026-10-01T10:00:00Z",
    lastReadAt: iso(-1),
  };
  const mdBook2 = {
    bookId: "md-du-shu-bi-ji",
    title: "读书笔记",
    source: "manual",
    chapterCount: 2,
    readCount: 0,
    progress: 0,
    currentChapterTitle: null,
    updatedAt: "2026-09-28T09:00:00Z",
    lastReadAt: null,
  };
  const epubBook = {
    bookId: "epub-xiao-wang-zi",
    title: "小王子·节选",
    source: "manual",
    chapterCount: 3,
    readCount: 0,
    progress: 0,
    currentChapterTitle: "第一章 帽子",
    updatedAt: "2026-10-02T12:00:00Z",
    lastReadAt: iso(-2),
  };
  const books = [mdBook1, mdBook2, epubBook];

  const mdChapters = [
    { id: "c1", path: "001.md", title: "第一章 山月", voteCount: 0, wordCount: 860 },
    { id: "c2", path: "002.md", title: "第二章 李征", voteCount: 0, wordCount: 920 },
    { id: "c3", path: "003.md", title: "第三章 猛虎", voteCount: 0, wordCount: 780 },
  ];
  const epubChapters = [
    { id: "c0", path: "content/chapter-1.xhtml", title: "第一章 帽子", voteCount: 0, wordCount: 620 },
    { id: "c1", path: "content/chapter-2.xhtml", title: "第二章 沙漠", voteCount: 0, wordCount: 700 },
    { id: "c2", path: "content/chapter-3.xhtml", title: "第三章 玫瑰", voteCount: 0, wordCount: 640 },
  ];

  const mdParagraph = (chapterId, index) =>
    `IR-QA-MD-${chapterId} 第 ${index} 段。山月记是中学生课本里常被引用的短篇；` +
    "这段合成文本只用于移动端排版/滚动/翻章验证，不包含任何真实用户内容。" +
    "文字长度刻意接近真实中文长文，用来观察行高、页宽、阅读进度与滚动性能。";

  function markdownContent(chapterId) {
    const body = Array.from({ length: 26 }, (_, i) => mdParagraph(chapterId, i + 1)).join("\n\n");
    return (
      `# ${chapterId.toUpperCase()} 山月记\n\n` +
      `> QA 合成 fixture：IR-QA-MD-${chapterId}\n\n` +
      `${body}\n\n` +
      "## 代码块\n\n```rust\nfn main() { println!(\"IR-QA-MD-CODE\"); }\n```\n\n" +
      "## 表格\n\n| 角色 | 状态 |\n|---|---|\n| 李征 | 化为虎 |\n| 袁傪 | 故人 |\n"
    );
  }

  function epubXhtml(chapterId) {
    const paragraphs = Array.from(
      { length: 18 },
      (_, i) =>
        `<p>IR-QA-EPUB-${chapterId} 第 ${i + 1} 段。这是一段用于 EPUB 阅读冒烟的合成正文，` +
        "验证纵向滚动、段落排版、内链与目录跳转。</p>",
    ).join("\n");
    return (
      `<?xml version="1.0" encoding="utf-8"?>` +
      `<html xmlns="http://www.w3.org/1999/xhtml"><head><title>${chapterId}</title></head><body>` +
      `<section><h1>IR-QA-EPUB-${chapterId}</h1>` +
      `<p><a href="chapter-2.xhtml#note">IR-QA-EPUB-INTERNAL-LINK</a></p>` +
      paragraphs +
      `<p id="note">IR-QA-EPUB-FOOTNOTE 脚注内容。</p>` +
      `<p><img src="../images/ir-qa-pixel.png" alt="QA 像素图"/></p>` +
      `</section></body></html>`
    );
  }

  // Data URI is not needed by the mock: convertFileSrc returns the input and the
  // image request is intentionally inert (missing asset must not fail the page).
  const bookDetails = {
    "md-shan-yue": () => ({
      manifest: {
        schemaVersion: 1,
        bookId: "md-shan-yue",
        title: "山月记·节选",
        source: "manual",
        generatedAt: "2026-10-01T10:00:00Z",
        updatedAt: "2026-10-01T10:00:00Z",
        chapters: mdChapters,
      },
      progress: {
        schemaVersion: 1,
        current: "c1",
        position: 0.34,
        read: ["c1"],
        updated: iso(-1),
      },
      provenance: null,
      taskRecords: [],
      bookDir: "/storage/emulated/0/ImmersiveReader-QA/Library/md-shan-yue",
    }),
    "md-du-shu-bi-ji": () => ({
      manifest: {
        schemaVersion: 1,
        bookId: "md-du-shu-bi-ji",
        title: "读书笔记",
        source: "manual",
        generatedAt: "2026-09-28T09:00:00Z",
        updatedAt: "2026-09-28T09:00:00Z",
        chapters: [
          { id: "n1", path: "001.md", title: "摘录：松弛感", voteCount: 0, wordCount: 500 },
          { id: "n2", path: "002.md", title: "摘录：长文阅读", voteCount: 0, wordCount: 540 },
        ],
      },
      progress: { schemaVersion: 1, current: "n1", position: 0, read: [], updated: "" },
      provenance: null,
      taskRecords: [],
      bookDir: "/storage/emulated/0/ImmersiveReader-QA/Library/md-du-shu-bi-ji",
    }),
    "epub-xiao-wang-zi": () => ({
      manifest: {
        schemaVersion: 1,
        bookId: "epub-xiao-wang-zi",
        title: "小王子·节选",
        source: "manual",
        generatedAt: "2026-10-02T12:00:00Z",
        updatedAt: "2026-10-02T12:00:00Z",
        chapters: epubChapters,
      },
      progress: { schemaVersion: 1, current: "c0", position: 0, read: [], updated: "" },
      provenance: null,
      taskRecords: [],
      bookDir: "/storage/emulated/0/ImmersiveReader-QA/Library/epub-xiao-wang-zi",
      publication: {
        schemaVersion: 1,
        format: "epub",
        epubVersion: "3.0",
        title: "小王子·节选",
        creator: "Antoine de Saint-Exupéry",
        language: "zh",
        cover: "images/cover.png",
        nav: [
          {
            title: "第一章 帽子",
            chapterId: "c0",
            children: [{ title: "1.1 蟒蛇", chapterId: "c1" }],
          },
          { title: "第三章 玫瑰", chapterId: "c2" },
        ],
        spine: ["c0", "c1", "c2"],
        resources: {
          "content/chapter-1.xhtml": "content/chapter-1.xhtml",
          "content/chapter-2.xhtml": "content/chapter-2.xhtml",
          "content/chapter-3.xhtml": "content/chapter-3.xhtml",
        },
        unsupported: [],
      },
    }),
  };

  const chapterLookup = (bookId) => {
    if (bookId === "epub-xiao-wang-zi") return epubChapters;
    if (bookId === "md-du-shu-bi-ji") return bookDetails["md-du-shu-bi-ji"]().manifest.chapters;
    return mdChapters;
  };
  const chapterIndex = (bookId, chapterId) =>
    chapterLookup(bookId).findIndex((chapter) => chapter.id === chapterId);

  const invoked = [];
  let importPolls = 0;
  let openDialogCalls = 0;

  function importStatus(operationId) {
    importPolls += 1;
    const done = importPolls >= 2;
    return {
      operationId,
      phase: done ? "completed" : "running",
      state: done ? "succeeded" : "running",
      terminal: done,
      percent: done ? 100 : 40,
      label: done ? "导入完成" : "读取文件",
      issues: done
        ? [{ path: "content://qa/one.md", status: "imported", message: "" }]
        : [],
      bookIds: done ? ["md-shan-yue"] : [],
    };
  }

  async function invoke(command, args) {
    invoked.push({ command, args: args ?? null, at: Date.now() });
    switch (command) {
      case "get_platform_capabilities":
        return {
          platform: "android",
          is_mobile: true,
          supported_formats: ["md", "markdown", "txt", "epub"],
          directory_import: false,
          uri_import: true,
          content_uri_import: true,
          reveal_directory: false,
          reader_session: false,
          reader_sessions: false,
          task_acquisition: false,
          secret_store: false,
          volume_key_capture: true,
          volume_key_bridge: true,
          open_with: true,
          pending_open_uris: true,
          export_bundle: false,
          reading_bundle: false,
          markdown_editing: false,
          epub: true,
          isMobile: true,
          supportedFormats: ["md", "markdown", "txt", "epub"],
        };
      case "scan_library": {
        if (STATE === "loading") {
          await new Promise((resolve) => setTimeout(resolve, 1800));
        }
        if (STATE === "empty") return { books: [], issues: [], writable: true };
        if (STATE === "error") {
          return {
            books,
            issues: [{ path: "/storage/emulated/0/QA/Broken/manifest.json", message: "manifest.json 无法解析" }],
            writable: false,
          };
        }
        return { books, issues: [], writable: true };
      }
      case "open_book": {
        const bookId = args?.bookId;
        const build = bookDetails[bookId];
        if (!build) throw new Error(`open_book: unknown bookId ${bookId}`);
        return build();
      }
      case "get_book_chapter_path": {
        const { bookId, chapterId } = args ?? {};
        return `/storage/emulated/0/ImmersiveReader-QA/Library/${bookId}/${chapterId}.md`;
      }
      case "read_markdown_file": {
        const path = String(args?.path ?? "");
        const chapterId = (path.split("/").pop() || "").replace(/\.md$/i, "") || "c1";
        return { content: markdownContent(chapterId), encoding: "utf-8" };
      }
      case "get_readable_chapter": {
        const { bookId, chapterId } = args ?? {};
        const index = chapterIndex(bookId, chapterId);
        const chapters = chapterLookup(bookId);
        const isEpub = bookId === "epub-xiao-wang-zi";
        return {
          format: isEpub ? "xhtml" : "markdown",
          chapterId,
          title: chapters[index]?.title ?? chapterId,
          content: isEpub ? epubXhtml(chapterId) : markdownContent(chapterId),
          resourceDir: isEpub ? "content" : "",
          prevChapterId: index > 0 ? chapters[index - 1].id : undefined,
          nextChapterId: index >= 0 && index + 1 < chapters.length ? chapters[index + 1].id : undefined,
        };
      }
      case "open_reader_locator":
      case "get_reader_locator":
        return null;
      case "save_reader_locator":
      case "save_book_progress":
      case "save_reading_state":
      case "save_recent_files":
      case "set_volume_key_capture":
        return command === "set_volume_key_capture" ? true : null;
      case "load_reading_state":
        return { scroll_position: 0, bookmarks: [], progress: 0 };
      case "load_recent_files":
        return { json: "[]", store_exists: true };
      case "list_bookmarks":
        return { bookmarks: [] };
      case "add_bookmark":
        return { bookmarkId: `qa-bm-${Date.now()}` };
      case "remove_bookmark":
        return null;
      case "search_book":
        return {
          hits: [
            {
              chapterId: chapterLookup(args?.bookId)[0]?.id ?? "c0",
              title: "QA 搜索命中",
              snippet: "IR-QA-SEARCH-HIT 合成搜索片段",
            },
          ],
        };
      case "get_app_settings":
        return {
          schemaVersion: 3,
          libraryRoot: "/storage/emulated/0/ImmersiveReader-QA/Library",
        };
      case "get_storage_locations":
        return {
          channel: "qa",
          settingsPath: "/data/user/0/com.lingcang.immersivereading.qa/files/settings.json",
          dataRoot: "/data/user/0/com.lingcang.immersivereading.qa/files/Data",
          cacheRoot: "/data/user/0/com.lingcang.immersivereading.qa/cache/ImmersiveReader",
          logsRoot: "/data/user/0/com.lingcang.immersivereading.qa/files/Logs",
          runtimeStateRoot: "/data/user/0/com.lingcang.immersivereading.qa/files/RuntimeState",
          backupsRoot: "/data/user/0/com.lingcang.immersivereading.qa/files/Backups",
          libraryRoot: "/storage/emulated/0/ImmersiveReader-QA/Library",
          runtimeRoot: "/data/user/0/com.lingcang.immersivereading.qa/files/runtime",
        };
      case "get_storage_usage":
        return {
          libraryBytes: 12_345_678,
          dataBytes: 1_234_567,
          cacheBytes: 456_789,
          logsBytes: 12_345,
          backupsBytes: 0,
          runtimeStateBytes: 1_024,
        };
      case "clear_safe_cache":
        return { deletedItems: 3, releasedBytes: 456_789, skipped: [] };
      case "list_trash":
        return [];
      case "restore_trash_item":
      case "permanently_delete_trash_item":
        return null;
      case "remove_book":
      case "delete_book":
        return "已移出书架（QA mock）";
      case "take_pending_open_uris": {
        if (MODE === "pending-open") {
          return ["content://qa/ImmersiveReader-QA/sample-notes.md"];
        }
        return [];
      }
      case "begin_import":
        importPolls = 0;
        return { operationId: "qa-import-1" };
      case "get_import_status":
        return importStatus(args?.operationId ?? "qa-import-1");
      case "cancel_import":
        return null;
      case "plugin:dialog|open": {
        openDialogCalls += 1;
        if (MODE === "import" || openDialogCalls > 0) {
          return [
            "content://com.android.providers.downloads.documents/document/qa-notes",
            "content://com.android.providers.downloads.documents/document/qa-epub",
          ];
        }
        return null;
      }
      case "plugin:dialog|save":
        return null;
      case "plugin:opener|open_url":
        return null;
      case "plugin:app|version":
      case "app_version":
        return "1.3.0-qa";
      case "plugin:event|listen":
      case "plugin:event|unlisten":
      case "plugin:event|emit":
        return command === "plugin:event|listen" ? 1 : null;
      case "plugin:updater|check":
        throw new Error("updater is desktop-only (QA mock)");
      // Desktop-only commands the mobile shell must never call. Returning a
      // benign value would hide that regression, so fail loudly instead: the
      // Playwright run then reports it as a console/UI error.
      case "get_acquisition_snapshot":
      case "list_temporary_content":
      case "get_secret_status":
      case "start_reader_session":
      case "load_reader_preferences":
        throw new Error(`desktop-only command called on mobile: ${command}`);
      default:
        // Unknown command: record and return null rather than rejecting so a
        // new optional call cannot hang the UI; verify_mobile.py asserts that
        // none of the *known* critical routes fell through.
        return null;
    }
  }

  let nextCallbackId = 1;
  const callbacks = new Map();

  window.__QA_MOCK__ = {
    books,
    mdChapters,
    epubChapters,
    invoked,
    version: "2026-10-18-mobile-qa-mock",
  };

  window.__TAURI_INTERNALS__ = {
    invoke,
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { windowLabel: "main", label: "main" },
    },
    convertFileSrc: (path) => path,
    transformCallback: (callback) => {
      const id = nextCallbackId++;
      callbacks.set(id, callback);
      window[`_${id}`] = callback;
      return id;
    },
    unregisterCallback: (id) => {
      callbacks.delete(id);
      delete window[`_${id}`];
    },
  };
})();
