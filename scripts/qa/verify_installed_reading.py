"""Exercise the installed Windows executable with isolated data and an invisible QA window."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import urllib.request

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / 'output/repair-20261018/windows-installed'
OUT.mkdir(parents=True, exist_ok=True)
sys.path.insert(0, str(Path(__file__).parent / 'mobile'))
from verify_android_reading import fixture


def call(page, command, args=None):
    return page.evaluate('([c,a]) => window.__TAURI_INTERNALS__.invoke(c,a||{})', [command, args])


def prepare(run_id):
    library = OUT / run_id / 'Library'
    library.mkdir(parents=True, exist_ok=True)
    for index in range(100):
        book = library / f'book-{index:03}'
        book.mkdir(exist_ok=True)
        chapter = '# 合成章节\n\n' + '\n\n'.join(f'第 {n} 段。阅读位置与启动性能验证。' * 4 for n in range(80))
        (book / '001.md').write_text(chapter, encoding='utf-8')
        manifest = dict(schemaVersion=1, bookId=f'qa:book-{index:03}', title=f'QA 合成书 {index:03}', source='manual', generatedAt='2026-10-18T00:00:00Z', updatedAt='2026-10-18T00:00:00Z', chapters=[dict(id='one', path='001.md', title='合成章节', voteCount=0, wordCount=len(chapter))])
        (book / 'manifest.json').write_text(json.dumps(manifest, ensure_ascii=False), encoding='utf-8')
    settings = Path(os.environ['LOCALAPPDATA']) / 'ImmersiveReader-QA' / run_id / 'Settings/settings.json'
    settings.parent.mkdir(parents=True, exist_ok=True)
    settings.write_text(json.dumps(dict(schemaVersion=3, libraryRoot=str(library))), encoding='utf-8')


def start(playwright, exe, run_id, port, expected):
    env = os.environ.copy()
    env.update(IMMERSIVE_QA_RUN_ID=run_id, IMMERSIVE_QA_HEADLESS='1', WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f'--remote-debugging-port={port}')
    before = time.perf_counter()
    process = subprocess.Popen([exe], env=env, creationflags=subprocess.CREATE_NO_WINDOW)
    try:
        browser = None
        for _ in range(200):
            if process.poll() is not None:
                raise RuntimeError(f'Installed application exited early: {process.returncode}')
            try:
                with urllib.request.urlopen(f'http://127.0.0.1:{port}/json/version', timeout=.2) as response:
                    json.load(response)
                browser = playwright.chromium.connect_over_cdp(f'http://127.0.0.1:{port}')
                break
            except Exception:
                time.sleep(.05)
        assert browser, 'WebView2 CDP unavailable'
        page = browser.contexts[0].pages[0]
        page.wait_for_function('(n) => document.querySelectorAll(".book-card").length === n', arg=expected, timeout=15000, polling=50)
        elapsed = round((time.perf_counter() - before) * 1000, 2)
        locations = call(page, 'get_storage_locations')
        assert locations['channel'] == 'qa', locations
        assert page.get_by_role('button', name='关闭应用', exact=True).count() == 1
        return process, browser, page, elapsed
    except BaseException:
        process.kill()
        process.wait()
        raise


def close(process, browser, page):
    page.locator('.chrome-btn-close').evaluate('(el) => el.click()')
    process.wait(timeout=15)
    assert process.returncode == 0, process.returncode
    browser.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--exe', required=True)
    parser.add_argument('--run-id', required=True)
    parser.add_argument('--port', type=int, default=9229)
    args = parser.parse_args()
    assert args.run_id.startswith('repair-') and all(c.isalnum() or c in '-_' for c in args.run_id)
    prepare(args.run_id)
    results = []
    def record(name, evidence):
        results.append(dict(name=name, evidence=evidence))
        print('PASS', name, evidence, flush=True)
        (OUT / 'report.json').write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding='utf-8')
    with sync_playwright() as playwright:
        process, browser, page, elapsed = start(playwright, args.exe, args.run_id, args.port, 100)
        record('100-book first launch ready including CDP connection overhead', elapsed)
        assert not call(page, 'scan_library')['issues']
        close(process, browser, page)
        record('native closeRequested exits installed process', 0)
        process, browser, page, elapsed = start(playwright, args.exe, args.run_id, args.port, 100)
        record('100-book warm launch ready including CDP connection overhead', elapsed)
        archive = fixture()
        op = call(page, 'begin_import', {'request': {'kind':'files', 'paths':[str(archive)]}})
        for _ in range(200):
            status = call(page, 'get_import_status', {'opId':op['opId']})
            if status['status'] != 'running': break
            page.wait_for_timeout(100)
        assert status['status'] == 'succeeded', status
        book_id = status['result']['bookId']
        page.reload()
        page.wait_for_function('document.querySelectorAll(".book-card").length === 101', polling=50)
        page.get_by_role('searchbox', name='搜索书架').fill('QA 长句与进度')
        page.locator('.book-card').get_by_role('button', name='精读', exact=True).evaluate('(el) => el.click()')
        page.wait_for_selector('.epub-content #long')
        page.wait_for_timeout(700)
        spacing = page.locator('.epub-content #long').evaluate('(el) => getComputedStyle(el).letterSpacing')
        assert spacing == '0.5px', spacing
        fonts = page.evaluate("async () => { await document.fonts.load('16px QAFont'); return [...document.fonts].filter(f => f.family==='QAFont').map(f => f.status); }")
        assert fonts and all(status == 'loaded' for status in fonts), fonts
        record('installed EPUB stylesheet and font pass actual Tauri CSP', dict(spacing=spacing, fonts=fonts))
        page.locator('.epub-scroll').evaluate('(el) => { el.scrollTop=(el.scrollHeight-el.clientHeight)*.4; el.dispatchEvent(new Event("scroll")); }')
        page.wait_for_timeout(100)
        close(process, browser, page)
        record('close during EPUB reading waits for durable locator write', book_id)
        process, browser, page, _ = start(playwright, args.exe, args.run_id, args.port, 101)
        saved = json.loads(call(page, 'load_reader_locator', {'bookId':book_id}))
        assert .3 < saved['ratio'] < .5, saved
        record('installed executable reloads SQLite locator after exit', saved)
        close(process, browser, page)


if __name__ == '__main__':
    main()
