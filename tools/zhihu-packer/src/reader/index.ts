import { getPackedArticles } from './modes/packed-mode.js';
import { initUniversalMode } from './modes/universal-mode.js';
import { getLastActiveSourceId } from './core/storage.js';
import { ReaderApp } from './ui/app.js';
import { VirtualFile } from './core/scanner.js';
import { loadServedMode } from './modes/served-mode.js';

async function bootstrap() {
  const served = await loadServedMode();
  if (served) {
    const landing = document.getElementById('landing-section');
    const container = document.getElementById('app-container');
    if (landing) landing.classList.add('hidden');
    if (container) container.classList.remove('hidden');
    new ReaderApp(
      new Map<string, VirtualFile>(),
      served.articles,
      served.sourceId,
      served.sourceName,
      served.mode,
    );
    return;
  }

  // 1. 检测是否包含注入打包文章数据 (Packed Mode)
  const packedArticles = getPackedArticles();

  if (packedArticles) {
    console.log('⚡ 检测到注入打包数据，进入专享模式...');
    
    // 隐藏导入主屏，展现阅读界面
    const landing = document.getElementById('landing-section');
    const container = document.getElementById('app-container');
    if (landing) landing.classList.add('hidden');
    if (container) container.classList.remove('hidden');

    // 在打包模式下没有本地 filesMap，传空 Map
    new ReaderApp(
      new Map<string, VirtualFile>(),
      packedArticles,
      'packed_source_archive',
      document.title || '内容归档',
      { kind: 'packed' },
    );
  } else {
    console.log('📂 未检测到注入数据，进入通用 Markdown 导入模式...');
    
    // 获取最后一次活动的文件夹 ID
    const lastActiveSourceId = getLastActiveSourceId();

    // 初始化通用模式并绑定回调
    initUniversalMode((filesMap, articles, sourceId, sourceName) => {
      console.log(`📁 成功导入文件夹 [${sourceName}], 包含 ${articles.length} 篇 Markdown 文件`);
      
      new ReaderApp(
        filesMap,
        articles,
        sourceId,
        sourceName,
        { kind: 'file' },
      );
    }, lastActiveSourceId);
  }
}

// 启动
window.addEventListener('DOMContentLoaded', () => void bootstrap().catch((error) => {
  console.error('Reader 启动失败:', error);
  const landing = document.getElementById('landing-section');
  if (landing) {
    landing.classList.remove('hidden');
    landing.replaceChildren();
    const content = document.createElement('div');
    content.className = 'landing-content';
    content.setAttribute('role', 'alert');
    const title = document.createElement('h1');
    title.textContent = '无法打开这本文集';
    const detail = document.createElement('p');
    detail.textContent = String(error);
    const retry = document.createElement('button');
    retry.type = 'button';
    retry.className = 'btn';
    retry.textContent = '重新载入';
    retry.onclick = () => window.location.reload();
    content.append(title, detail, retry);
    landing.appendChild(content);
  }
}));
export {};
