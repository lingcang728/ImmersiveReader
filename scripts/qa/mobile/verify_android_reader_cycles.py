"""Check reader teardown and one-key/one-page behavior using synthetic QA data."""
import argparse
import json
import time

from playwright.sync_api import sync_playwright
import verify_android_reading as qa


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--serial', required=True)
    args = parser.parse_args()
    qa.SERIAL = args.serial
    assert qa.PACKAGE.endswith('.qa')
    qa.adb('shell', 'am', 'start', '-W', '-n',
           qa.PACKAGE + '/com.lingcang.immersivereading.MainActivity')
    samples, errors = [], []
    with sync_playwright() as playwright:
        browser, page = qa.attach(playwright)
        page.on('pageerror', lambda error: errors.append(str(error)))
        assert qa.invoke(page, 'get_storage_locations')['channel'] == 'qa'
        if page.locator('[data-testid="focus-hud"]').count():
            page.get_by_role('button', name='退出专注模式').click()
        if page.locator('[data-testid="reader-back"]').count():
            page.locator('[data-testid="reader-back"]').click()
            page.locator('[data-testid="reader-screen"]').wait_for(state='detached')
        books = qa.invoke(page, 'scan_library')['books']
        book = next((row['bookId'] for row in books if row['title'] == 'QA 长句与进度'), None)
        if book is None:
            fixture_dir = f'/sdcard/Android/data/{qa.PACKAGE}/files/Download/repair-20261018'
            qa.adb('shell', 'mkdir', '-p', fixture_dir)
            qa.adb('shell', 'chmod', '755',
                   f'/sdcard/Android/data/{qa.PACKAGE}/files/Download', fixture_dir)
            qa.adb('push', str(qa.fixture()), fixture_dir + '/long.epub')
            book = qa.import_file(page, fixture_dir + '/long.epub')
        chapter = qa.invoke(page, 'open_book', {'bookId': book})['manifest']['chapters'][0]['id']
        page.evaluate("() => localStorage.setItem('mmbook-auto-focus','true')")
        page.reload()
        page.locator('[data-testid="shelf-continue"], [data-testid^="shelf-book-"]').first.wait_for()
        cdp = browser.contexts[0].new_cdp_session(page)
        try:
            for cycle in range(12):
                locator = dict(schemaVersion=1, bookId=book, chapterId=chapter,
                               anchor=dict(kind='ratio'), ratio=0,
                               updated=time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()))
                qa.invoke(page, 'save_reader_locator', {
                    'bookId': book, 'locatorJson': json.dumps(locator)})
                qa.open_book(page, book)
                page.locator('[data-testid="focus-hud"]').wait_for()
                before = page.locator('.epub-scroll').evaluate('(el) => el.scrollTop')
                qa.adb('shell', 'input', 'keyevent', '25')
                page.wait_for_timeout(750)
                after = page.locator('.epub-scroll').evaluate('(el) => el.scrollTop')
                assert 200 < after - before < 800, (cycle, before, after)
                assert '1 / 3' in page.locator('[data-testid="focus-hud"]').inner_text()
                page.get_by_role('button', name='退出专注模式').click()
                focus_exit_scroll = page.locator('.epub-scroll').evaluate('(el) => el.scrollTop')
                page.locator('[data-testid="reader-back"]').click()
                # The shelf stays mounted beneath the reader. Its visibility
                # alone does not prove the asynchronous save/exit completed.
                page.locator('[data-testid="reader-screen"]').wait_for(state='detached')
                page.locator('[data-testid="shelf-continue"]').wait_for()
                page.wait_for_timeout(100)
                saved = json.loads(qa.invoke(page, 'load_reader_locator', {'bookId': book}))
                if saved['chapterId'] != chapter or saved['ratio'] <= 0:
                    raise AssertionError(dict(saved=saved, before=before, after=after,
                        focusExitScroll=focus_exit_scroll, errors=errors))
                assert qa.status_bar_visible()
                released_observer = cycle in (1, 11)
                if released_observer:
                    # Release CDP/Playwright object handles before comparing
                    # retained memory. Keep the app process and document alive.
                    cdp.detach()
                    browser.close()
                    browser, page = qa.attach(playwright)
                    page.on('pageerror', lambda error: errors.append(str(error)))
                    cdp = browser.contexts[0].new_cdp_session(page)
                cdp.send('Runtime.discardConsoleEntries')
                cdp.send('HeapProfiler.collectGarbage')
                page.wait_for_timeout(100)
                cdp.send('HeapProfiler.collectGarbage')
                heap = cdp.send('Runtime.getHeapUsage')
                counters = cdp.send('Memory.getDOMCounters')
                listener_query = cdp.send('Runtime.evaluate', dict(
                    expression='JSON.stringify({volume:(getEventListeners(window)["ir:volume-key"]||[]).length,back:(getEventListeners(window)["ir:android-back"]||[]).length})',
                    includeCommandLineAPI=True, returnByValue=True))
                assert 'exceptionDetails' not in listener_query, listener_query
                reader_listeners = json.loads(listener_query['result']['value'])
                assert reader_listeners == dict(volume=0, back=0), reader_listeners
                sample = dict(cycle=cycle + 1, pageStep=round(after - before, 2),
                              observerReferencesReleased=released_observer,
                              readerListeners=reader_listeners,
                              usedHeapBytes=heap['usedSize'], **counters)
                samples.append(sample)
                print('PASS reader teardown cycle', sample, flush=True)
            baseline, final = samples[1], samples[-1]
            # Warm up resource/font caches before checking retained growth.
            # Total listener counts also include newly attached DevTools scripts.
            # Assert the reader-owned event types above, not inspector internals.
            assert final['nodes'] <= baseline['nodes'] + 80, samples
            assert final['documents'] <= baseline['documents'] + 4, samples
            assert final['usedHeapBytes'] <= baseline['usedHeapBytes'] * 1.4 + 3_000_000, samples
            assert not errors, errors
            report = dict(samples=samples, errors=errors,
                          scope='12 short QA reader cycles; not an hours-long memory soak')
            output = qa.OUT / 'reader-cycles.json'
            output.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding='utf-8')
            (qa.OUT / 'reader-cycles-memory.txt').write_text(
                qa.adb('shell', 'dumpsys', 'meminfo', qa.PACKAGE), encoding='utf-8')
            print('PASS native reader cycles, no retained reader listeners or DOM growth', flush=True)
        finally:
            cdp.detach()
            browser.close()
            qa.adb('forward', '--remove', 'tcp:9228')


if __name__ == '__main__':
    main()
