import { getBrowserContext, closeBrowserContext, syncCookiesToObscuraStorage, markInteractiveSession } from './browser.js';
import { logger } from './utils.js';

// P3-2：登录窗口启动失败（浏览器全灭、profile 占用）原本只进 sidecar 日志，
// 桌面端显示"已打开登录流程"然后什么都没有发生。最近一次失败原因挂在这里，
// 由 /api/login-status 透传给前端展示。
let loginLastError: string | null = null;
export function getLoginLastError(): string | null {
  return loginLastError;
}

export async function runLogin(): Promise<void> {
  logger.info('正在开启有头浏览器以进行知乎登录，请在弹出的浏览器中手动完成登录。');
  loginLastError = null;

  try {
    // 浏览器启动/开页也可能抛错：必须进 try，否则 finally 的 closeBrowserContext
    // 走不到，泄漏的有头窗口会一直被复用（P1-7/P1-8）。
    // purpose=interactive：任务持有浏览器时拒绝登录而不是互杀（P2）；
    // 会话旗标在 getBrowserContext 锁内立起。
    const context = await getBrowserContext(false, 'interactive');
    const page = await context.newPage();
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
        // 06-F-05：日志不落知乎用户名（个人身份信息）——原先读取
        // .AppHeader-profileName 只为拼欢迎语，整块移除。
        logger.info('检测到登录成功！');
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
    loginLastError = e?.message || String(e);
    logger.error(`登录过程中发生错误: ${e.message}`);
  } finally {
    markInteractiveSession(false);
    await closeBrowserContext();
    logger.info('浏览器已关闭，登录态已保存。');
  }
}
