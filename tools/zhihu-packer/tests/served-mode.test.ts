import assert from 'node:assert/strict';
import test from 'node:test';

import { buildServedContentUrl, manifestToArticles } from '../src/reader/modes/served-mode.ts';

test('served metadata preserves manifest order for a 379 chapter book', () => {
  const chapters = Array.from({ length: 379 }, (_, index) => ({
    id: `chapter:${index}`,
    path: `chapters/${String(index).padStart(3, '0')}.md`,
    title: `第 ${index + 1} 篇`,
    voteCount: index,
    wordCount: 100 + index,
  }));
  const articles = manifestToArticles({
    schemaVersion: 1,
    bookId: 'manual:fixture',
    title: '379 章夹具',
    source: 'manual',
    generatedAt: '2026-07-10T00:00:00.000Z',
    updatedAt: '2026-07-10T00:00:00.000Z',
    chapters,
  });
  assert.equal(articles.length, 379);
  assert.equal(articles[0]?.articleId, 'chapter:0');
  assert.equal(articles[378]?.articleId, 'chapter:378');
});

test('served content URLs encode every relative path segment', () => {
  assert.equal(
    buildServedContentUrl('/s/token/content', 'assets/封 面.png'),
    '/s/token/content/assets/%E5%B0%81%20%E9%9D%A2.png',
  );
});
