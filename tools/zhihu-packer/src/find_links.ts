import { getBrowserContext, closeBrowserContext } from './browser.js';
import { logger } from './utils.js';

async function main() {
  logger.info('正在启动浏览器并打开知乎主页以提取有效的回答和文章链接...');
  const context = await getBrowserContext(true); // 无头模式即可
  const page = await context.newPage();
  
  try {
    await page.goto('https://www.zhihu.com', { waitUntil: 'networkidle', timeout: 30000 });
    
    // 提取回答链接
    const answerLinks = await page.evaluate(() => {
      const links = Array.from(document.querySelectorAll('a'));
      return links
        .map(a => a.href)
        .filter(href => href.includes('/question/') && href.includes('/answer/'));
    });

    // 提取文章链接
    const articleLinks = await page.evaluate(() => {
      const links = Array.from(document.querySelectorAll('a'));
      return links
        .map(a => a.href)
        .filter(href => href.includes('zhuanlan.zhihu.com/p/') || href.includes('zhihu.com/p/'));
    });

    logger.info(`发现回答链接数量: ${answerLinks.length}`);
    answerLinks.slice(0, 5).forEach((l, i) => logger.info(`回答 ${i + 1}: ${l}`));

    logger.info(`发现文章链接数量: ${articleLinks.length}`);
    articleLinks.slice(0, 5).forEach((l, i) => logger.info(`文章 ${i + 1}: ${l}`));
  } catch (e: any) {
    logger.error(`提取链接失败: ${e.message}`);
  } finally {
    await closeBrowserContext();
  }
}

main();
