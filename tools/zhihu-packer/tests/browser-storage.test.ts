import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';

import type { BrowserContext } from 'playwright-core';

test('stores Obscura login cookies beneath the managed Zhihu profile', async () => {
  const sandbox = fs.mkdtempSync(path.join(os.tmpdir(), 'immersive-zhihu-browser-'));
  const profileRoot = path.join(sandbox, 'Data', 'Private', 'ZhihuProfile');
  const previousCwd = process.cwd();
  const previousProfile = process.env.IMMERSIVE_ZHIHU_PROFILE;
  process.chdir(sandbox);
  process.env.IMMERSIVE_ZHIHU_PROFILE = profileRoot;

  try {
    const { syncCookiesToObscuraStorage } = await import(
      `../src/browser.js?managed-storage-test=${Date.now()}`
    );
    const context = {
      cookies: async () => []
    } as unknown as BrowserContext;

    await syncCookiesToObscuraStorage(context);

    assert.equal(
      fs.existsSync(path.join(profileRoot, '.obscura-profile', 'cookies.json')),
      true
    );
    assert.equal(fs.existsSync(path.join(sandbox, '.obscura-profile', 'cookies.json')), false);
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
