"""Exercise the installed Windows executable with isolated data and an invisible QA window."""
import argparse
import base64
import ctypes
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
ACTIVE_PROCESSES = []
sys.path.insert(0, str(Path(__file__).parent / 'mobile'))
from verify_android_reading import fixture


def call(page, command, args=None):
    return page.evaluate('([c,a]) => window.__TAURI_INTERNALS__.invoke(c,a||{})', [command, args])


def child_processes(process_id):
    # Read only this isolated QA process tree; never inspect unrelated command lines.
    script = r'''
    $ProgressPreference = 'SilentlyContinue'
    $rows = @(Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,ExecutablePath)
    $ids = [Collections.Generic.HashSet[uint32]]::new()
    [void]$ids.Add(__QA_PID__)
    do {
      $changed = $false
      foreach ($row in $rows) {
        if ($ids.Contains([uint32]$row.ParentProcessId) -and $ids.Add([uint32]$row.ProcessId)) { $changed = $true }
      }
    } while ($changed)
    ConvertTo-Json -Compress -InputObject @($rows | Where-Object { $_.ProcessId -ne __QA_PID__ -and $ids.Contains([uint32]$_.ProcessId) })
    '''.replace('__QA_PID__', str(int(process_id)))
    encoded = base64.b64encode(script.encode('utf-16-le')).decode('ascii')
    raw = subprocess.check_output(['powershell.exe', '-NoProfile', '-NonInteractive', '-OutputFormat', 'Text', '-EncodedCommand', encoded], encoding='utf-8', creationflags=subprocess.CREATE_NO_WINDOW, timeout=15)
    return json.loads(raw)


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


def start(playwright, exe, run_id, port, expected, open_path=None):
    env = os.environ.copy()
    # Hidden WebViews otherwise clamp timers; emulate foreground scheduling
    # without displaying or focusing a native window on the user's desktop.
    env.update(IMMERSIVE_QA_RUN_ID=run_id, IMMERSIVE_QA_HEADLESS='1', WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=f'--remote-debugging-port={port} --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows')
    before = time.perf_counter()
    before_epoch_ms = time.time() * 1000
    process = subprocess.Popen([exe, *([open_path] if open_path else [])], env=env, creationflags=subprocess.CREATE_NO_WINDOW)
    ACTIVE_PROCESSES.append(process)
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
        if expected is None:
            page.wait_for_function("() => document.querySelector('.article')?.textContent.includes('IRQA associated file')", timeout=20000, polling=50)
        else:
            page.wait_for_function('(n) => document.querySelectorAll(".book-card").length === n', arg=expected, timeout=15000, polling=50)
        observed = round((time.perf_counter() - before) * 1000, 2)
        timing = page.evaluate("() => ({origin:performance.timeOrigin,ready:performance.getEntriesByName('ir-shelf-ready')[0]?.startTime??null})")
        elapsed = dict(observedWithCdpMs=observed, nativeDomReadyMs=round(timing['origin']+timing['ready']-before_epoch_ms,2) if timing['ready'] is not None else None, frontendReadyMs=timing['ready'], processToNavigationMs=round(timing['origin']-before_epoch_ms,2))
        locations = call(page, 'get_storage_locations')
        assert locations['channel'] == 'qa', locations
        assert page.get_by_role('button', name='关闭应用', exact=True).count() == 1
        return process, browser, page, elapsed
    except BaseException:
        process.kill()
        process.wait()
        raise


def close(process, browser, page, system_request=False):
    # Retain handles to the actual QA descendants before exiting. Waiting on
    # handles also avoids PID reuse, and prevents a warm relaunch from racing
    # WebView2's asynchronous profile shutdown.
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel.OpenProcess.argtypes = [ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
    kernel.OpenProcess.restype = ctypes.c_void_p
    kernel.WaitForMultipleObjects.argtypes = [ctypes.c_ulong, ctypes.POINTER(ctypes.c_void_p), ctypes.c_int, ctypes.c_ulong]
    kernel.CloseHandle.argtypes = [ctypes.c_void_p]
    handles = [handle for row in child_processes(process.pid) if (handle := kernel.OpenProcess(0x100000, False, row['ProcessId']))]
    try:
        if system_request:
            # The same native CloseRequested path as the OS close command.
            call(page, 'plugin:window|close', {'label':'main'})
        else:
            page.locator('.chrome-btn-close').evaluate('(el) => el.click()')
        process.wait(timeout=15)
        assert process.returncode == 0, process.returncode
        browser.close()
        if handles:
            result = kernel.WaitForMultipleObjects(len(handles), (ctypes.c_void_p * len(handles))(*handles), True, 10000)
            assert result == 0, f'QA child processes did not exit: wait={result}'
    finally:
        for handle in handles:
            kernel.CloseHandle(handle)


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
        (OUT / args.run_id / 'report.json').write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding='utf-8')
    with sync_playwright() as playwright:
        process, browser, page, elapsed = start(playwright, args.exe, args.run_id, args.port, 100)
        record('100-book first launch ready including CDP connection overhead', elapsed)
        call(page, 'get_acquisition_snapshot', {'kind':None})
        page.wait_for_timeout(700)
        children = child_processes(process.pid)
        engines = [row for row in children if row['Name'].lower() in ('node.exe', 'python.exe', 'pythonw.exe')]
        assert not engines, engines
        record('bookshelf startup does not launch acquisition engines', [row['Name'] for row in children])
        assert not call(page, 'scan_library')['issues']
        close(process, browser, page, system_request=True)
        record('native closeRequested exits installed process', 0)
        process, browser, page, elapsed = start(playwright, args.exe, args.run_id, args.port, 100)
        record('100-book warm launch ready including CDP connection overhead', elapsed)
        for sample in range(2):
            close(process, browser, page)
            process, browser, page, elapsed = start(playwright, args.exe, args.run_id, args.port, 100)
            record('100-book additional warm launch', dict(sample=sample+2, **elapsed))
        archive = fixture()
        op = call(page, 'begin_import', {'request': {'kind':'files', 'paths':[str(archive)]}})
        for _ in range(200):
            status = call(page, 'get_import_status', {'opId':op['opId']})
            if status['status'] != 'running': break
            page.wait_for_timeout(100)
        assert status['status'] == 'succeeded', status
        book_id = status['result']['bookId']
        hits = call(page, 'search_book', {'bookId':book_id, 'query':'长句', 'limit':10})
        assert len(hits) == 2, hits
        record('native EPUB search indexes both chapters and supports short CJK queries', len(hits))
        page.reload()
        page.wait_for_function('() => document.querySelectorAll(".book-card").length === 101', polling=50)
        page.get_by_role('searchbox', name='搜索书架').fill('QA 长句与进度')
        page.locator('.book-card').get_by_role('button', name='阅读', exact=True).evaluate('(el) => el.click()')
        page.wait_for_selector('.epub-content #long')
        page.wait_for_timeout(700)
        spacing = page.locator('.epub-content #long').evaluate('(el) => getComputedStyle(el).letterSpacing')
        assert spacing == '0.5px', spacing
        fonts = page.evaluate("async () => { await document.fonts.load('16px QAFont'); return [...document.fonts].filter(f => f.family==='QAFont').map(f => f.status); }")
        assert fonts and all(status == 'loaded' for status in fonts), fonts
        record('installed EPUB stylesheet and font pass actual Tauri CSP', dict(spacing=spacing, fonts=fonts))
        page.locator('.epub-scroll').evaluate('(el) => { el.scrollTop=(el.scrollHeight-el.clientHeight)*.4; el.dispatchEvent(new Event("scroll")); }')
        page.wait_for_timeout(100)
        page.locator('.epub-back').evaluate('(el) => el.click()')
        page.wait_for_selector('.epub-content', state='detached')
        returned = json.loads(call(page, 'load_reader_locator', {'bookId':book_id}))
        assert .3 < returned['ratio'] < .5, returned
        record('EPUB unmount after shelf return retains the durable locator', returned)
        page.locator('.book-card').get_by_role('button', name='阅读', exact=True).evaluate('(el) => el.click()')
        page.wait_for_selector('.epub-content #long')
        page.wait_for_timeout(700)
        close(process, browser, page)
        record('close during EPUB reading waits for durable locator write', book_id)
        process, browser, page, _ = start(playwright, args.exe, args.run_id, args.port, 101)
        saved = json.loads(call(page, 'load_reader_locator', {'bookId':book_id}))
        assert .3 < saved['ratio'] < .5, saved
        record('installed executable reloads SQLite locator after exit', saved)
        close(process, browser, page)
        source_dir = OUT / args.run_id / 'source'
        source_dir.mkdir(exist_ok=True)
        source = source_dir / 'associated-file.md'
        content = '# IRQA associated file\n\nNative command-line opening automatically adds a managed copy.\n'
        source.write_text(content, encoding='utf-8')
        for attempt in range(2):
            process, browser, page, _ = start(playwright, args.exe, args.run_id, args.port, None, str(source))
            scan = call(page, 'scan_library')
            assert len(scan['books']) == 102, len(scan['books'])
            assert source.read_text(encoding='utf-8') == content
            close(process, browser, page)
            record('native file-opening arguments import once and preserve original', dict(attempt=attempt+1, bookCount=102))
        process, browser, page, _ = start(playwright, args.exe, args.run_id, args.port, 102)
        incoming = subprocess.Popen([args.exe, str(archive)], env={**os.environ, 'IMMERSIVE_QA_RUN_ID':args.run_id, 'IMMERSIVE_QA_HEADLESS':'1'}, creationflags=subprocess.CREATE_NO_WINDOW)
        ACTIVE_PROCESSES.append(incoming)
        assert incoming.wait(timeout=15) == 0
        page.wait_for_selector('.epub-content #long')
        assert len(call(page, 'scan_library')['books']) == 102
        close(process, browser, page)
        record('second-instance EPUB handoff opens existing managed book', 102)


if __name__ == '__main__':
    try:
        main()
    finally:
        # Only processes created by this isolated QA run may be terminated.
        for process in ACTIVE_PROCESSES:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=15)
