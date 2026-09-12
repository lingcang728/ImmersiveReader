import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { withBrowserLock } from '../src/browser.js';

test('browser lifecycle mutex serializes concurrent operations', async () => {
  const order: string[] = [];
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });

  const first = withBrowserLock(async () => {
    order.push('a:start');
    await gate;
    order.push('a:end');
  });
  const second = withBrowserLock(async () => {
    order.push('b');
  });

  release();
  await Promise.all([first, second]);
  assert.deepEqual(order, ['a:start', 'a:end', 'b']);
});

test('browser lifecycle mutex recovers after a rejected operation', async () => {
  await assert.rejects(
    withBrowserLock(async () => { throw new Error('boom'); }),
    /boom/
  );
  const result = await withBrowserLock(async () => 'ok');
  assert.equal(result, 'ok');
});

test('login status probe is read-only and never launches a browser', async () => {
  const sandbox = fs.mkdtempSync(path.join(os.tmpdir(), 'zhihu-login-status-'));
  const profileRoot = path.join(sandbox, 'Data', 'Private', 'ZhihuProfile');
  const previousCwd = process.cwd();
  const previousProfile = process.env.IMMERSIVE_ZHIHU_PROFILE;
  process.chdir(sandbox);
  process.env.IMMERSIVE_ZHIHU_PROFILE = profileRoot;

  try {
    const browser = await import(`../src/browser.js?login-status-probe=${Date.now()}`);
    const storageDir = path.join(profileRoot, '.obscura-profile');
    const cookiesFile = path.join(storageDir, 'cookies.json');

    // 无凭据、无活跃 context：只读返回未登录，且绝不能为一次状态查询拉起浏览器——
    // 若真去 launch，profile 目录会被创建。
    assert.deepEqual(await browser.getLoginStatus(), { loggedIn: false });
    assert.equal(fs.existsSync(profileRoot), false, 'status probe must not launch a browser');

    fs.mkdirSync(storageDir, { recursive: true });
    fs.writeFileSync(cookiesFile, JSON.stringify([
      { name: 'z_c0', value: 'token', domain: '.zhihu.com', expires: Math.floor(Date.now() / 1000) + 3600 }
    ]), 'utf8');
    assert.deepEqual(await browser.getLoginStatus(), { loggedIn: true });

    fs.writeFileSync(cookiesFile, JSON.stringify([
      { name: 'z_c0', value: 'token', domain: '.zhihu.com', expires: 1000 }
    ]), 'utf8');
    assert.deepEqual(await browser.getLoginStatus(), { loggedIn: false }, 'expired z_c0 is not logged in');

    fs.writeFileSync(cookiesFile, '{corrupt', 'utf8');
    assert.deepEqual(await browser.getLoginStatus(), { loggedIn: false }, 'corrupt credentials must not throw');
  } finally {
    process.chdir(previousCwd);
    if (previousProfile === undefined) {
      delete process.env.IMMERSIVE_ZHIHU_PROFILE;
    } else {
      process.env.IMMERSIVE_ZHIHU_PROFILE = previousProfile;
    }
    fs.rmSync(sandbox, { recursive: true, force: true, maxRetries: 3, retryDelay: 50 });
  }
});

test('login status probe never reconfigures or closes the active context', () => {
  const here = path.dirname(fileURLToPath(import.meta.url));
  const source = fs.readFileSync(path.join(here, '..', 'src', 'browser.ts'), 'utf8');
  const probeStart = source.indexOf('export function getLoginStatus');
  const probeEnd = source.indexOf('export function closeBrowserContext', probeStart);
  assert.ok(probeStart >= 0 && probeEnd > probeStart);
  const probeBody = source.slice(probeStart, probeEnd);
  assert.equal(probeBody.includes('getBrowserContext'), false, 'probe must not create a context');
  assert.equal(probeBody.includes('closeBrowserContext'), false, 'probe must not close a headed login window');
  assert.ok(source.includes(".on('error'"), 'spawned browser child must observe error events');
});
