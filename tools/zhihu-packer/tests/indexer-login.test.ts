import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';

import type { Page } from 'playwright-core';

import { scrapePeopleIndex } from '../src/indexer.js';

class LoginWallPage {
  private currentUrl = '';

  on(): void {}
  off(): void {}

  async goto(url: string): Promise<void> {
    this.currentUrl = url;
  }

  url(): string {
    return this.currentUrl;
  }

  async waitForTimeout(): Promise<void> {}

  async $eval(): Promise<never> {
    throw new Error('missing initial data');
  }

  async evaluate(pageFunction: unknown): Promise<unknown> {
    const source = String(pageFunction);
    if (source.includes('hasInitial')) {
      return { hasInitial: true, cards: 0, challenge: false };
    }
    if (source.includes('没有更多了')) {
      return true;
    }
    if (source.includes('hasLoginWall')) {
      return { hasLoginWall: true, hasChallenge: false, hasExplicitEmpty: false };
    }
    return [];
  }

  context(): { cookies: () => Promise<Array<{ name: string }>> } {
    return { cookies: async () => [] };
  }

  async content(): Promise<never> {
    throw new Error('debug output disabled in test');
  }
}

test('rejects an empty logged-out people page as LOGIN_REQUIRED', async () => {
  const page = new LoginWallPage() as unknown as Page;

  await assert.rejects(
    scrapePeopleIndex(page, 'xiao-xue-shi-46-24', 'answers', 5),
    /LOGIN_REQUIRED/
  );
});

class StaleLoginWallPage extends LoginWallPage {
  context(): { cookies: () => Promise<Array<{ name: string }>> } {
    return { cookies: async () => [{ name: 'z_c0' }] };
  }
}

test('rejects a visible login wall even when a stale z_c0 cookie remains', async () => {
  const page = new StaleLoginWallPage() as unknown as Page;

  await assert.rejects(
    scrapePeopleIndex(page, 'xiao-xue-shi-46-24', 'answers', 5),
    /LOGIN_REQUIRED/
  );
});

class BlockedLoggedInPage extends LoginWallPage {
  override async evaluate(pageFunction: unknown): Promise<unknown> {
    if (String(pageFunction).includes('hasLoginWall')) {
      return { hasLoginWall: false, hasChallenge: false, hasExplicitEmpty: false };
    }
    return super.evaluate(pageFunction);
  }

  context(): { cookies: () => Promise<Array<{ name: string }>> } {
    return { cookies: async () => [{ name: 'z_c0' }] };
  }

  override async content(): Promise<string> {
    return '<html><body>upstream returned no verifiable content</body></html>';
  }
}

test('rejects an unverified empty page and writes diagnostics under browser cache', async () => {
  const cacheRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'immersive-zhihu-indexer-'));
  const previousCache = process.env.IMMERSIVE_ZHIHU_BROWSER_CACHE;
  process.env.IMMERSIVE_ZHIHU_BROWSER_CACHE = cacheRoot;

  try {
    const page = new BlockedLoggedInPage() as unknown as Page;

    await assert.rejects(
      scrapePeopleIndex(page, 'xiao-xue-shi-46-24', 'answers', 5),
      /CAPTCHA_REQUIRED/
    );
    assert.equal(fs.existsSync(path.join(cacheRoot, 'debug-people-answers.html')), true);
  } finally {
    if (previousCache === undefined) {
      delete process.env.IMMERSIVE_ZHIHU_BROWSER_CACHE;
    } else {
      process.env.IMMERSIVE_ZHIHU_BROWSER_CACHE = previousCache;
    }
    fs.rmSync(cacheRoot, { recursive: true, force: true });
  }
});

class ArticleIndexPage {
  private responseHandler: ((response: ArticleResponse) => Promise<void>) | null = null;
  navigatedUrl = '';

  on(event: string, handler: (response: ArticleResponse) => Promise<void>): void {
    if (event === 'response') this.responseHandler = handler;
  }

  off(): void {}

  async goto(url: string): Promise<void> {
    this.navigatedUrl = url;
    await this.responseHandler?.({
      url: () => 'https://www.zhihu.com/api/v4/members/xiao-xue-shi-46-24/posts',
      json: async () => ({
        data: [
          {
            type: 'article',
            id: 123,
            title: '测试文章',
            author: { id: 'author-id', name: '我要的是饼干' },
            created: 100,
            updated: 101,
            voteup_count: 2,
            likes_count: 2,
            comment_count: 3
          }
        ],
        paging: { is_end: true, next: '' }
      })
    });
  }

  url(): string {
    return this.navigatedUrl;
  }

  async waitForTimeout(): Promise<void> {}

  async $eval(): Promise<never> {
    throw new Error('missing initial data');
  }

  async evaluate(pageFunction: unknown): Promise<unknown> {
    const source = String(pageFunction);
    if (source.includes('hasInitial')) {
      return { hasInitial: true, cards: 0, challenge: false };
    }
    if (source.includes('没有更多了')) {
      return true;
    }
    return [];
  }

  context(): { cookies: () => Promise<Array<{ name: string }>> } {
    return { cookies: async () => [{ name: 'z_c0' }] };
  }

  async content(): Promise<never> {
    throw new Error('debug output disabled in test');
  }
}

type ArticleResponse = {
  url: () => string;
  json: () => Promise<unknown>;
};

test('uses the current /posts route for a people article index', async () => {
  const fakePage = new ArticleIndexPage();

  const items = await scrapePeopleIndex(
    fakePage as unknown as Page,
    'xiao-xue-shi-46-24',
    'articles',
    5
  );

  assert.equal(fakePage.navigatedUrl, 'https://www.zhihu.com/people/xiao-xue-shi-46-24/posts');
  assert.equal(items.length, 1);
});
