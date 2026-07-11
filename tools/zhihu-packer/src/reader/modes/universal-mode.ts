import { scanFolder, VirtualFile, readText, isMarkdownFile, isSupportedFile } from '../core/scanner.js';
import { extractMetadata, ArticleMetadata, calculateSourceId } from '../core/metadata.js';
import { saveDirectoryHandle, saveSourceFolderName, getDirectoryHandle, verifyHandlePermission, getSourceFolderName } from '../core/storage.js';

type LoadedCallback = (
  filesMap: Map<string, VirtualFile>,
  articles: ArticleMetadata[],
  sourceId: string,
  sourceName: string,
  directoryHandle?: FileSystemDirectoryHandle
) => void;

/**
 * 递归读取 Markdown 头部 (最长 16KB，直到遇到 --- 结束符)
 */
async function readHeaderChunk(vFile: VirtualFile): Promise<string> {
  // 如果是句柄模式，可以通过 slice 仅读取前 16KB 字节，避免大文件内存开销
  const maxHeaderBytes = 16384; // 16KB
  if (vFile.handle) {
    const file = await vFile.handle.getFile();
    if (file.size > maxHeaderBytes) {
      const slice = file.slice(0, maxHeaderBytes);
      return await slice.text();
    }
    return await file.text();
  } else if (vFile.file) {
    if (vFile.file.size > maxHeaderBytes) {
      const slice = vFile.file.slice(0, maxHeaderBytes);
      return await slice.text();
    }
    return await vFile.file.text();
  }
  throw new Error('VirtualFile 缺少实体，无法读取');
}

/**
 * 扫描并加载文件夹所有 Markdown 索引信息
 */
export async function loadFolderData(
  input: FileSystemDirectoryHandle | FileList | File[],
  rootDirName: string
): Promise<{
  filesMap: Map<string, VirtualFile>;
  articles: ArticleMetadata[];
  sourceId: string;
  warning?: string;
}> {
  // 1. 扫描文件树
  const { files, warning } = await scanFolder(input);
  
  // 2. 建立相对路径索引 Map
  const filesMap = new Map<string, VirtualFile>();
  for (const f of files) {
    filesMap.set(f.relativePath, f);
  }

  // 3. 构建 Source ID
  const sourceId = calculateSourceId(rootDirName, files);

  // 4. 解析 YAML Front Matter 和头部元数据
  const articles: ArticleMetadata[] = [];
  
  // 检查是否有知乎特有的 index.md 索引文件
  const indexMdFile = files.find(f => f.name.toLowerCase() === 'index.md');
  
  if (indexMdFile) {
    try {
      const indexContent = await readText(indexMdFile);
      // 精准正则匹配：- [[文件名|标题]] (发布于: 日期 | 赞同数: 点赞数)
      const lineRegex = /-\s+\[\[([^|]+)\|([^\]]+)\]\]\s+\(发布于:\s*([^\s|]+)\s*\|\s*赞同数:\s*(\d+)\)/;
      const lines = indexContent.split('\n');
      
      const parentDir = indexMdFile.relativePath.substring(0, indexMdFile.relativePath.lastIndexOf('/'));

      for (const line of lines) {
        const match = line.match(lineRegex);
        if (match) {
          const relativeFilename = match[1];
          const title = match[2];
          const date = match[3];
          const upvoteCount = parseInt(match[4], 10);
          
          // 拼装出该文章相对于根目录的路径
          const articlePath = parentDir ? `${parentDir}/${relativeFilename}` : relativeFilename;
          const artFile = filesMap.get(articlePath);
          
          if (artFile && isMarkdownFile(artFile.name)) {
            // 获取头部摘要
            const headText = await readHeaderChunk(artFile);
            const meta = extractMetadata(artFile.relativePath, artFile.name, artFile.lastModified, headText);
            
            // 覆盖为 index.md 里面的准确属性
            meta.title = title;
            meta.date = date;
            const parsedTime = Date.parse(date);
            if (!isNaN(parsedTime)) {
              meta.timestamp = parsedTime;
            }
            meta.upvoteCount = upvoteCount;
            
            articles.push(meta);
          }
        }
      }
    } catch (e) {
      console.warn('解析 index.md 失败，降级为常规通用 Markdown 导入:', e);
    }
  }

  // 如果没有 index.md 或者解析出的文章为 0，则遍历读取所有 .md 文件的元数据
  if (articles.length === 0) {
    for (const f of files) {
      // 忽略 index.md 以免渲染导航目录本身，且必须是 Markdown 文件
      if (f.name.toLowerCase() === 'index.md' || !isMarkdownFile(f.name)) continue;
      
      try {
        const headText = await readHeaderChunk(f);
        const meta = extractMetadata(f.relativePath, f.name, f.lastModified, headText);
        articles.push(meta);
      } catch (err) {
        console.error(`解析文件元数据失败 [path=${f.relativePath}]:`, err);
      }
    }

    // 按日期/时间戳进行倒序排列 (从新到老)
    articles.sort((a, b) => b.timestamp - a.timestamp);
  }

  return {
    filesMap,
    articles,
    sourceId,
    warning
  };
}

/**
 * 初始化 Universal Mode UI 绑定
 */
export function initUniversalMode(
  onLoaded: LoadedCallback,
  lastActiveSourceId: string | null
): void {
  const landingSection = document.getElementById('landing-section');
  const appContainer = document.getElementById('app-container');
  const selectFolderBtn = document.getElementById('select-folder-btn') as HTMLButtonElement | null;
  const dropZone = document.getElementById('drop-zone');
  const restoreBtn = document.getElementById('restore-last-btn') as HTMLButtonElement | null;
  const fallbackInput = document.getElementById('fallback-folder-input') as HTMLInputElement;

  if (!landingSection || !appContainer) return;

  // 展示导入页，收起主页面
  landingSection.classList.remove('hidden');
  appContainer.classList.add('hidden');

  // A. 处理一键恢复上次阅读按钮
  const canRestore = localStorage.getItem('last_active_source_can_restore') === 'true';
  if (lastActiveSourceId && canRestore) {
    const lastFolderName = getSourceFolderName(lastActiveSourceId);
    if (restoreBtn) {
      restoreBtn.textContent = `尝试恢复上次阅读 [${lastFolderName}]`;
      restoreBtn.classList.remove('hidden');
      
      // 绑定恢复点击事件 (必须有用户交互手势)
      restoreBtn.onclick = async () => {
        restoreBtn.disabled = true;
        restoreBtn.textContent = '正在获取权限...';
        
        try {
          const handle = await getDirectoryHandle(lastActiveSourceId);
          if (handle) {
            // 弹出浏览器权限授权
            const hasPermission = await verifyHandlePermission(handle, true);
            if (hasPermission) {
              restoreBtn.textContent = '正在扫描目录...';
              const { filesMap, articles, sourceId, warning } = await loadFolderData(handle, handle.name);
              
              if (warning) alert(warning);
              
              // 成功加载
              landingSection.classList.add('hidden');
              appContainer.classList.remove('hidden');
              onLoaded(filesMap, articles, sourceId, handle.name, handle);
              return;
            }
          }
          throw new Error('未获取到授权或句柄已失效');
        } catch (err) {
          console.error('一键恢复上次阅读失败:', err);
          restoreBtn.classList.add('btn-error');
          restoreBtn.textContent = '尝试恢复上次阅读失败，请重新选择文件夹';
          setTimeout(() => {
            restoreBtn.classList.remove('btn-error');
            restoreBtn.textContent = `尝试恢复上次阅读 [${lastFolderName}]`;
            restoreBtn.disabled = false;
          }, 3000);
        }
      };
    }
  }

  // B. 绑定常规文件夹选择 (showDirectoryPicker)
  if (selectFolderBtn) {
    selectFolderBtn.onclick = async () => {
      if (typeof window.showDirectoryPicker === 'function') {
        try {
          const handle = await window.showDirectoryPicker({ mode: 'read' });
          selectFolderBtn.textContent = '正在读取文件...';
          
          const { filesMap, articles, sourceId, warning } = await loadFolderData(handle, handle.name);
          
          if (warning) alert(warning);

          // 保存文件夹句柄及别名
          await saveDirectoryHandle(sourceId, handle);
          saveSourceFolderName(sourceId, handle.name);
          localStorage.setItem('last_active_source_can_restore', 'true');

          landingSection.classList.add('hidden');
          appContainer.classList.remove('hidden');
          onLoaded(filesMap, articles, sourceId, handle.name, handle);
        } catch (err: any) {
          if (err.name !== 'AbortError') {
            console.error('选择目录失败:', err);
            alert('读取目录失败，请确保授予权限或尝试拖放文件夹导入。');
          }
          selectFolderBtn.textContent = '选择 Markdown 文件夹';
        }
      } else {
        // Fallback 到 input file
        if (fallbackInput) {
          fallbackInput.click();
        }
      }
    };
  }

  // C. 绑定 Fallback Input WebkitDirectory 变更
  if (fallbackInput) {
    fallbackInput.addEventListener('change', async () => {
      const files = fallbackInput.files;
      if (!files || files.length === 0) return;

      if (selectFolderBtn) selectFolderBtn.textContent = '正在读取文件...';
      try {
        // 提取根目录名字
        const sampleFile = files[0];
        const rootName = sampleFile.webkitRelativePath.split('/')[0] || '本地文档';

        const { filesMap, articles, sourceId, warning } = await loadFolderData(files, rootName);

        if (warning) alert(warning);

        // input 模式无法保存句柄到 IndexedDB，但可以记住 Source ID 进度
        saveSourceFolderName(sourceId, rootName);
        localStorage.setItem('last_active_source_id', sourceId);
        localStorage.setItem('last_active_source_can_restore', 'false');

        landingSection.classList.add('hidden');
        appContainer.classList.remove('hidden');
        onLoaded(filesMap, articles, sourceId, rootName);
      } catch (err) {
        console.error('读取 FileList 失败:', err);
        alert('读取目录文件失败，请重试。');
      } finally {
        if (selectFolderBtn) selectFolderBtn.textContent = '选择 Markdown 文件夹';
      }
    });
  }

  // D. 绑定拖放事件
  if (dropZone) {
    dropZone.addEventListener('dragover', (e) => {
      e.preventDefault();
      dropZone.classList.add('dragover');
    });

    dropZone.addEventListener('dragleave', () => {
      dropZone.classList.remove('dragover');
    });

    dropZone.addEventListener('drop', async (e) => {
      e.preventDefault();
      dropZone.classList.remove('dragover');
      
      const items = e.dataTransfer?.items;
      if (!items) return;

      const filesList: File[] = [];
      let rootName = '拖放导入';

      // 递归读取 DataTransferItem
      const traverseEntry = async (entry: any): Promise<void> => {
        if (entry.isFile) {
          if (isSupportedFile(entry.name)) {
            const file = await new Promise<File>((resolve, reject) => {
              entry.file(resolve, reject);
            });
            // 模拟 webkitRelativePath 便于 scanner 正确计算路径层级
            Object.defineProperty(file, 'webkitRelativePath', {
              value: entry.fullPath.replace(/^\//, ''),
              writable: false
            });
            filesList.push(file);
          }
        } else if (entry.isDirectory) {
          const dirReader = entry.createReader();
          const readEntries = async (): Promise<any[]> => {
            return new Promise((resolve, reject) => {
              dirReader.readEntries(resolve, reject);
            });
          };
          
          let entries = await readEntries();
          // DataTransferReader 可能分批次返回文件，循环读空
          while (entries.length > 0) {
            for (const subEntry of entries) {
              await traverseEntry(subEntry);
            }
            entries = await readEntries();
          }
        }
      };

      try {
        if (selectFolderBtn) selectFolderBtn.textContent = '正在解析拖入文件...';
        
        for (let i = 0; i < items.length; i++) {
          const entry = items[i].webkitGetAsEntry();
          if (entry) {
            if (i === 0) rootName = entry.name;
            await traverseEntry(entry);
          }
        }

        if (filesList.length === 0) {
          alert('未在拖入的目录中检测到任何 Markdown 文件 (.md)。');
          return;
        }

        const { filesMap, articles, sourceId, warning } = await loadFolderData(filesList, rootName);

        if (warning) alert(warning);

        saveSourceFolderName(sourceId, rootName);
        localStorage.setItem('last_active_source_id', sourceId);
        localStorage.setItem('last_active_source_can_restore', 'false');

        landingSection.classList.add('hidden');
        appContainer.classList.remove('hidden');
        onLoaded(filesMap, articles, sourceId, rootName);
      } catch (err) {
        console.error('解析拖入文件失败:', err);
        alert('解析拖入的文件夹失败，请确保拖入的是个文件夹。');
      } finally {
        if (selectFolderBtn) selectFolderBtn.textContent = '选择 Markdown 文件夹';
      }
    });
  }
}
