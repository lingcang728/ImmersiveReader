export interface VirtualFile {
  relativePath: string; // 相对路径，例如 "folder/doc.md" (统一使用正斜杠 /)
  name: string;         // 文件名，例如 "doc.md"
  lastModified: number; // 毫秒时间戳
  handle?: FileSystemFileHandle; // showDirectoryPicker 句柄
  file?: File;          // input webkitdirectory 或 drag-drop 模式下的原生 File 对象
}

// 默认忽略的敏感/非文档目录
const EXCLUDED_DIRS = new Set([
  '.git',
  'node_modules',
  'dist',
  'build',
  '.venv',
  '__pycache__',
  '.cache'
]);

const MAX_FILES = 2000;
const MAX_DEPTH = 5;

export function isMarkdownFile(name: string): boolean {
  return name.toLowerCase().endsWith('.md');
}

export function isSupportedFile(name: string): boolean {
  if (isMarkdownFile(name)) return true;
  return /\.(png|jpe?g|gif|webp|bmp|avif)$/i.test(name);
}

/**
 * 格式化文件路径，统一为正斜杠，并去除首部斜杠
 */
function normalizePath(path: string): string {
  return path.replace(/\\/g, '/').replace(/^\//, '');
}

/**
 * 递归扫描 FileSystemDirectoryHandle (Chrome/Edge API)
 */
async function scanDirectoryHandle(
  dirHandle: FileSystemDirectoryHandle,
  currentPath: string,
  depth: number,
  fileList: VirtualFile[]
): Promise<void> {
  if (depth > MAX_DEPTH) return;
  if (fileList.length >= MAX_FILES) return;

  for await (const entry of dirHandle.values()) {
    if (fileList.length >= MAX_FILES) break;

    if (entry.kind === 'directory') {
      if (EXCLUDED_DIRS.has(entry.name)) continue;
      const subDirPath = currentPath ? `${currentPath}/${entry.name}` : entry.name;
      await scanDirectoryHandle(entry as FileSystemDirectoryHandle, subDirPath, depth + 1, fileList);
    } else if (entry.kind === 'file') {
      if (isSupportedFile(entry.name)) {
        const file = await (entry as FileSystemFileHandle).getFile();
        fileList.push({
          relativePath: normalizePath(currentPath ? `${currentPath}/${entry.name}` : entry.name),
          name: entry.name,
          lastModified: file.lastModified,
          handle: entry as FileSystemFileHandle
        });
      }
    }
  }
}

/**
 * 扁平扫描原生的 FileList/File 数组 (Firefox/Safari/Drag-Drop 兼容回退)
 */
async function scanNativeFiles(
  files: File[] | FileList,
  fileList: VirtualFile[]
): Promise<void> {
  const arr = files instanceof FileList ? Array.from(files) : files;
  let count = 0;

  for (const f of arr) {
    if (count >= MAX_FILES) break;

    // 提取并规范化 webkitRelativePath
    let relPath = normalizePath(f.webkitRelativePath || f.name);
    
    // 如果是拖拽得到的，有些可能没有 webkitRelativePath。若有，则按其分层过滤
    const pathParts = relPath.split('/');
    
    // 检查是否包含任何需要忽略的目录
    const hasExcluded = pathParts.some(part => EXCLUDED_DIRS.has(part));
    if (hasExcluded) continue;

    // 检查递归深度 (从根目录起计算的文件夹层数)
    // 比如 "folder/sub1/sub2/sub3/sub4/doc.md" 深度为 5
    if (pathParts.length - 1 > MAX_DEPTH) continue;

    if (isSupportedFile(f.name)) {
      fileList.push({
        relativePath: relPath,
        name: f.name,
        lastModified: f.lastModified,
        file: f
      });
      count++;
    }
  }
}

/**
 * 通用读取文本接口
 */
export async function readText(vFile: VirtualFile): Promise<string> {
  if (vFile.file) {
    return await vFile.file.text();
  } else if (vFile.handle) {
    const file = await vFile.handle.getFile();
    return await file.text();
  }
  throw new Error(`无法读取文件 ${vFile.relativePath}：缺少句柄或文件实体`);
}

/**
 * 通用读取 Blob 接口
 */
export async function readBlob(vFile: VirtualFile): Promise<Blob> {
  if (vFile.file) {
    return vFile.file;
  } else if (vFile.handle) {
    return await vFile.handle.getFile();
  }
  throw new Error(`无法读取文件 ${vFile.relativePath}：缺少句柄或文件实体`);
}

/**
 * 根扫描入口
 */
export async function scanFolder(
  input: FileSystemDirectoryHandle | FileList | File[]
): Promise<{ files: VirtualFile[]; warning?: string }> {
  const files: VirtualFile[] = [];
  
  if (typeof FileSystemDirectoryHandle !== 'undefined' && input instanceof FileSystemDirectoryHandle) {
    await scanDirectoryHandle(input, '', 1, files);
  } else {
    await scanNativeFiles(input as FileList | File[], files);
  }

  files.sort((a, b) => a.relativePath.localeCompare(b.relativePath));

  const seen = new Set<string>();
  for (const file of files) {
    if (seen.has(file.relativePath)) {
      throw new Error(`检测到重复文件路径，已停止导入以避免读取错文件: ${file.relativePath}`);
    }
    seen.add(file.relativePath);
  }

  let warning: string | undefined;
  if (files.length >= MAX_FILES) {
    warning = `导入文件数已达上限 (${MAX_FILES} 篇)，部分超限文档已被忽略。`;
  }

  return { files, warning };
}
