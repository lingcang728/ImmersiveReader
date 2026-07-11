import { getBrowserContext, closeBrowserContext, syncCookiesToObscuraStorage } from './browser.js';
import { logger } from './utils.js';

export async function runLogin(): Promise<void> {
  logger.info('正在开启有头浏览器以进行知乎登录，请在弹出的浏览器中手动完成登录。');
  
  const context = await getBrowserContext(false); // 有头模式
  const page = await context.newPage();
  
  try {
    await page.goto('https://www.zhihu.com/signin', { waitUntil: 'domcontentloaded' });
    
    logger.info('已打开知乎登录页面。正在检测登录状态...');
    
    const maxWaitTime = 5 * 60 * 1000; // 5分钟
    const checkInterval = 2000;
    let elapsed = 0;
    let loggedIn = false;
    
    while (elapsed < maxWaitTime) {
      const currentUrl = page.url();
      const cookies = await context.cookies();
      const hasLoginCookie = cookies.some(c => c.name === 'z_c0');
      
      const profileExists = await page.$('.AppHeader-profile, .AppHeader-user').then(el => !!el);
      const isUnhuman = currentUrl.includes('unhuman') || currentUrl.includes('captcha');
      
      if (!isUnhuman && ((!currentUrl.includes('signin') && hasLoginCookie) || profileExists)) {
        logger.info('检测到登录成功！');
        
        try {
          await page.waitForSelector('.AppHeader-profileName', { timeout: 3000 });
          const name = await page.$eval('.AppHeader-profileName', el => el.textContent);
          logger.info(`欢迎回来，${name ? name.trim() : '知乎用户'}！`);
        } catch (e) {
          // ignore profile name fetch error
        }
        
        loggedIn = true;
        break;
      }
      
      await new Promise(resolve => setTimeout(resolve, checkInterval));
      elapsed += checkInterval;
    }
    
    if (!loggedIn) {
      logger.warn('登录超时或未完成。');
    } else {
      await syncCookiesToObscuraStorage(context);
    }
  } catch (e: any) {
    logger.error(`登录过程中发生错误: ${e.message}`);
  } finally {
    await closeBrowserContext();
    logger.info('浏览器已关闭，登录态已保存。');
  }
}
