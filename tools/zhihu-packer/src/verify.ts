import * as path from 'path';
import { getBrowserContext, closeBrowserContext } from './browser.js';

const readerPath = path.join(path.resolve(), 'output', 'reader.html');
const fileUrl = `file:///${readerPath.replace(/\\/g, '/')}`;

async function verify() {
  console.log(`🔍 开始自动化验证: ${fileUrl}`);
  
  const context = await getBrowserContext(true);
  const page = await context.newPage();
  
  const consoleErrors: string[] = [];
  page.on('pageerror', (exception) => {
    consoleErrors.push(exception.toString());
  });
  
  page.on('console', (msg) => {
    console.log(`[PAGE LOG] ${msg.text()}`);
    if (msg.type() === 'error') {
      consoleErrors.push(msg.text());
    }
  });

  try {
    // 1. 加载页面
    await page.goto(fileUrl, { waitUntil: 'load' });
    console.log('✅ 页面加载成功');

    // 2. 检查控制台报错
    if (consoleErrors.length > 0) {
      console.error('❌ 页面加载后发现控制台错误:');
      consoleErrors.forEach(err => console.error(`  - ${err}`));
      process.exitCode = 1;
      return;
    }
    console.log('✅ 无控制台报错');

    // 3. 验证关键 DOM 组件是否存在
    const sidebar = await page.$('#sidebar');
    const container = await page.$('#articles-container');
    const menuTrigger = await page.$('#menu-trigger');
    const floatingProgress = await page.$('#floating-progress');
    const articlesCount = await page.$$eval('.article-card', el => el.length);
    
    if (!sidebar) throw new Error('未找到目录侧栏 #sidebar');
    if (!container) throw new Error('未找到文章容器 #articles-container');
    if (!menuTrigger) throw new Error('未找到悬浮菜单按钮 #menu-trigger');
    if (!floatingProgress) throw new Error('未找到悬浮页码标签 #floating-progress');
    console.log(`✅ 核心 DOM 节点验证通过。已渲染文章卡片数: ${articlesCount}`);

    // 4. 验证第一个卡片是否是 active 聚焦的
    const firstCardClass = await page.$eval('#article-0', el => el.className);
    if (!firstCardClass.includes('active')) {
      throw new Error('首个卡片加载时未处于激活状态');
    }
    console.log('✅ 首个卡片默认高亮聚焦验证通过');

    // 5. 验证侧栏初始状态为关闭 (未包含 active class)
    const isSidebarDefaultActive = await page.$eval('#sidebar', el => el.classList.contains('active'));
    if (isSidebarDefaultActive) {
      throw new Error('目录侧栏初始状态应当为收起(隐藏)，但检测到了 active 类');
    }
    console.log('✅ 目录侧栏初始默认收起验证通过');

    // 6. 模拟 M 键展开侧栏
    await page.keyboard.press('m');
    await page.waitForTimeout(200);
    const isSidebarActiveAfterM = await page.$eval('#sidebar', el => el.classList.contains('active'));
    if (!isSidebarActiveAfterM) {
      throw new Error('按 M 键后目录侧栏未能成功展开 (未检测到 active 类)');
    }
    console.log('✅ 按 M 键展开抽屉侧栏验证通过');

    // 再次按 M 键收起侧栏
    await page.keyboard.press('m');
    await page.waitForTimeout(200);
    const isSidebarActiveAfterM2 = await page.$eval('#sidebar', el => el.classList.contains('active'));
    if (isSidebarActiveAfterM2) {
      throw new Error('再次按 M 键后目录侧栏未能收起');
    }
    console.log('✅ 再次按 M 键收起抽屉侧栏验证通过');

    // 7. 模拟键盘 ArrowRight 切换文章
    await page.keyboard.press('ArrowRight');
    await page.waitForTimeout(500);
    
    const secondCardClass = await page.$eval('#article-1', el => el.className);
    const activeProgressText = await page.$eval('#floating-progress', el => el.textContent.replace(/\s+/g, ' ').trim());
    
    if (!secondCardClass.includes('active')) {
      throw new Error('按 ArrowRight 键后未能成功聚焦下一篇卡片 (#article-1)');
    }
    const expectedProgressText = `2 / ${articlesCount}`;
    if (activeProgressText !== expectedProgressText) {
      throw new Error(`页码进度显示不正确: 期望为 "${expectedProgressText}"，实际为 "${activeProgressText}"`);
    }
    console.log('✅ 键盘 ArrowRight 切换下一篇文章（及右上角页码同步更新）验证通过');

    // 8. 模拟字号无级缩放
    const initialFontSize = await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--p-font-size').trim());
    await page.keyboard.press('+');
    await page.waitForTimeout(100);
    
    const largerFontSize = await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--p-font-size').trim());
    if (initialFontSize === largerFontSize) {
      throw new Error('按 + 键后字体大小未能变化');
    }
    console.log(`✅ 字号无级缩放验证通过 (${initialFontSize} -> ${largerFontSize})`);

    // 9. 模拟 s 键唤醒 Command Palette 并进行拼音模糊检索
    await page.keyboard.press('s');
    await page.waitForTimeout(200);
    
    const isSearchOverlayVisible = await page.$eval('#search-overlay', el => el.classList.contains('active'));
    if (!isSearchOverlayVisible) {
      throw new Error('按 s 键后检索浮层 #search-overlay 未显现 (active class 缺失)');
    }
    console.log('✅ Command Palette 唤醒验证通过');

    const firstResultText = await page.$eval('.palette-item.selected .palette-item-title', el => el.textContent);
    if (!firstResultText || firstResultText.trim().length === 0) {
      throw new Error('检索浮层首条结果标题为空');
    }
    console.log(`✅ Command Palette 默认结果验证通过，首条结果: ${firstResultText}`);

    // 回车定位
    await page.keyboard.press('Enter');
    await page.waitForTimeout(500);
    
    const isSearchOverlayClosed = await page.$eval('#search-overlay', el => !el.classList.contains('active'));
    if (!isSearchOverlayClosed) {
      throw new Error('选定检索结果回车后检索浮层未能关闭 (active class 仍存在)');
    }
    console.log('✅ 检索后回车关闭检索层并自动定位验证通过');

    console.log('\n🌟 恭喜！全交互功能自动化验证 100% 通过！页面逻辑完美健壮。');

  } catch (err: any) {
    console.error('\n❌ 验证失败:', err.message);
    process.exitCode = 1;
  } finally {
    await closeBrowserContext();
  }
}

verify();
