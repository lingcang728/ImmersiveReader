import test from 'node:test';
import assert from 'node:assert/strict';
import { selectIndexItems, type ScrapedIndexItem } from '../src/indexer.js';

function item(
  id: string,
  type: 'answer' | 'article',
  createdTime: number,
  voteupCount: number
): ScrapedIndexItem {
  return {
    id,
    type,
    title: id,
    authorId: 'author',
    authorName: 'Author',
    url: 'https://example.test/' + id,
    createdTime,
    updatedTime: createdTime,
    voteupCount,
    commentCount: 0
  };
}

test('all plus topN selects a single combined result set', () => {
  const items = [
    item('answer:1', 'answer', 100, 2),
    item('article:2', 'article', 90, 50),
    item('answer:3', 'answer', 80, 40),
    item('article:4', 'article', 70, 30),
    item('answer:5', 'answer', 60, 20),
    item('article:6', 'article', 50, 10)
  ];

  const selected = selectIndexItems(items, 5, 'vote');

  assert.equal(selected.length, 5);
  assert.deepEqual(selected.map((entry) => entry.id), [
    'article:2',
    'answer:3',
    'article:4',
    'answer:5',
    'article:6'
  ]);
});

test('time ordering uses vote count and id as deterministic tie breakers', () => {
  const items = [
    item('answer:2', 'answer', 100, 1),
    item('article:1', 'article', 100, 4),
    item('answer:3', 'answer', 90, 99)
  ];

  const selected = selectIndexItems(items, null, 'time');

  assert.deepEqual(selected.map((entry) => entry.id), ['article:1', 'answer:2', 'answer:3']);
  assert.deepEqual(items.map((entry) => entry.id), ['answer:2', 'article:1', 'answer:3']);
});
