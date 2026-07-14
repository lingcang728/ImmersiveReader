import assert from 'node:assert/strict';
import test from 'node:test';
import {
  emptyPagingState,
  mergeListApiPage,
  shouldStopIndexScroll,
  type ScrapedIndexItem
} from '../src/indexer.js';

function answerItem(id: number) {
  return {
    type: 'answer',
    id,
    question: { id: 9000 + id, title: `Q${id}` },
    author: { id: 'a1', name: '作者' },
    created_time: id,
    updated_time: id,
    voteup_count: id,
    comment_count: 0
  };
}

test('mergeListApiPage de-duplicates and tracks is_end / totals', () => {
  const collected = new Map<string, ScrapedIndexItem>();
  const paging = emptyPagingState();

  const first = mergeListApiPage(
    'answers',
    'author-token',
    {
      data: [answerItem(1), answerItem(2)],
      paging: { is_end: false, next: 'https://api/page2', totals: 3 }
    },
    collected,
    paging
  );
  assert.equal(first.added, 2);
  assert.equal(first.repeatedCursor, false);
  assert.equal(paging.isEnd, false);
  assert.equal(paging.totals, 3);
  assert.equal(paging.next, 'https://api/page2');

  const second = mergeListApiPage(
    'answers',
    'author-token',
    {
      data: [answerItem(2), answerItem(3)],
      paging: { is_end: true, next: '', totals: 3 }
    },
    collected,
    paging
  );
  assert.equal(second.added, 1);
  assert.equal(collected.size, 3);
  assert.equal(paging.isEnd, true);
  assert.equal(paging.pagesSeen, 2);
});

test('mergeListApiPage detects repeated next cursor', () => {
  const collected = new Map<string, ScrapedIndexItem>();
  const paging = emptyPagingState();
  mergeListApiPage(
    'answers',
    'author-token',
    { data: [answerItem(1)], paging: { is_end: false, next: 'cursor-a' } },
    collected,
    paging
  );
  const again = mergeListApiPage(
    'answers',
    'author-token',
    { data: [answerItem(1)], paging: { is_end: false, next: 'cursor-a' } },
    collected,
    paging
  );
  assert.equal(again.repeatedCursor, true);
});

test('shouldStopIndexScroll prioritizes api is_end over no-new streak', () => {
  const paging = emptyPagingState();
  paging.isEnd = true;
  const decision = shouldStopIndexScroll({
    topN: null,
    collectedSize: 122,
    paging,
    hasDomNoMore: false,
    noNewCount: 0,
    scrollCount: 3,
    maxScrolls: 200,
    repeatedCursor: false
  });
  assert.equal(decision.stop, true);
  assert.equal(decision.reason, 'api_is_end');
});

test('shouldStopIndexScroll uses topN before weak no-new stop', () => {
  const paging = emptyPagingState();
  const decision = shouldStopIndexScroll({
    topN: 10,
    collectedSize: 10,
    paging,
    hasDomNoMore: false,
    noNewCount: 8,
    scrollCount: 20,
    maxScrolls: 200,
    repeatedCursor: false
  });
  assert.equal(decision.reason, 'top_n:10');
});

test('shouldStopIndexScroll falls back to no-new when paging absent', () => {
  const paging = emptyPagingState();
  const decision = shouldStopIndexScroll({
    topN: null,
    collectedSize: 90,
    paging,
    hasDomNoMore: false,
    noNewCount: 8,
    scrollCount: 40,
    maxScrolls: 200,
    repeatedCursor: false
  });
  assert.equal(decision.reason, 'no_new:8');
});

test('shouldStopIndexScroll stops when api totals reached', () => {
  const paging = emptyPagingState();
  paging.totals = 50;
  const decision = shouldStopIndexScroll({
    topN: null,
    collectedSize: 50,
    paging,
    hasDomNoMore: false,
    noNewCount: 0,
    scrollCount: 5,
    maxScrolls: 200,
    repeatedCursor: false
  });
  assert.equal(decision.reason, 'api_totals:50');
});

test('simulates 750-item pagination without early no-new stop while is_end false', () => {
  const collected = new Map<string, ScrapedIndexItem>();
  const paging = emptyPagingState();
  const pageSize = 20;
  const total = 750;
  let pages = 0;
  for (let offset = 0; offset < total; offset += pageSize) {
    pages += 1;
    const count = Math.min(pageSize, total - offset);
    const data = Array.from({ length: count }, (_, i) => answerItem(offset + i + 1));
    const isEnd = offset + count >= total;
    mergeListApiPage(
      'answers',
      'author-token',
      {
        data,
        paging: {
          is_end: isEnd,
          next: isEnd ? '' : `cursor-${pages}`,
          totals: total
        }
      },
      collected,
      paging
    );
    const decision = shouldStopIndexScroll({
      topN: null,
      collectedSize: collected.size,
      paging,
      hasDomNoMore: false,
      noNewCount: 0,
      scrollCount: pages,
      maxScrolls: 200,
      repeatedCursor: false
    });
    if (decision.stop) {
      assert.ok(
        decision.reason === 'api_is_end' || decision.reason === `api_totals:${total}`,
        `unexpected stop ${decision.reason} at page ${pages}`
      );
      break;
    }
  }
  assert.equal(collected.size, total);
  assert.equal(paging.isEnd, true);
  assert.ok(pages >= Math.ceil(total / pageSize));
});
