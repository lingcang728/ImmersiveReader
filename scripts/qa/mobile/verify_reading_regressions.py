"""Headless synthetic EPUB persistence, focus paging and CSS isolation regressions."""
import json
from pathlib import Path
from playwright.sync_api import sync_playwright
from verify_mobile import StaticServer, BUILD_DIR, MOCK_JS, launch_browser, new_context, ensure_chrome

OUT = Path(__file__).resolve().parents[3] / "output/repair-20261018/reading-regressions"
OUT.mkdir(parents=True, exist_ok=True)
viewport = {"label": "359x789@3.5", "width": 359, "height": 789, "dpr": 3.5}
results = []
def record(name, value):
    results.append({"name": name, "evidence": value})
    print("PASS", name, value, flush=True)

def open_epub(page):
    page.locator('[data-testid="shelf-book-epub-xiao-wang-zi"]').click()
    page.wait_for_selector('.epub-content p')
    page.wait_for_timeout(700)

def main():
    with StaticServer(BUILD_DIR) as server, sync_playwright() as playwright:
        browser, _ = launch_browser(playwright, False)
        context = new_context(browser, viewport)
        page = context.new_page(); errors=[]
        page.on("pageerror", lambda error: errors.append(str(error)))
        page.add_init_script(MOCK_JS)
        page.goto(server.url); open_epub(page)
        page.locator('.epub-scroll').evaluate('(el) => { el.scrollTop = (el.scrollHeight-el.clientHeight)*0.55; el.dispatchEvent(new Event("scroll")); }')
        page.wait_for_timeout(1800)
        saved = page.evaluate('JSON.parse(localStorage.getItem("qa:locator:epub-xiao-wang-zi"))')
        assert saved and 0.45 < saved["ratio"] < 0.65, saved
        before = page.locator('.epub-scroll').evaluate('(el) => el.scrollTop')
        page.reload(); open_epub(page)
        after = page.locator('.epub-scroll').evaluate('(el) => el.scrollTop')
        assert abs(before-after) < 80, (before, after, saved)
        record("EPUB reload restores paragraph text offset", {"before": before, "after": after, "anchor": saved["anchor"]})
        ensure_chrome(page)
        page.locator('[data-testid="reader-settings"]').click()
        page.wait_for_timeout(1700)
        before_font = page.evaluate('JSON.parse(localStorage.getItem("qa:locator:epub-xiao-wang-zi"))')
        page.get_by_role('button', name='增大字号', exact=True).click()
        page.locator('[aria-label="阅读设置"] .sheet-close').click()
        page.wait_for_timeout(1700)
        after_font = page.evaluate('JSON.parse(localStorage.getItem("qa:locator:epub-xiao-wang-zi"))')
        assert before_font['anchor']['kind'] == after_font['anchor']['kind'] == 'text'
        assert abs(before_font['anchor']['offset'] - after_font['anchor']['offset']) < 100, (before_font, after_font)
        record("live font reflow retains the reading text anchor", {"before": before_font['anchor']['offset'], "after": after_font['anchor']['offset']})
        ensure_chrome(page)
        page.locator('[data-testid="reader-bookmarks"]').click()
        page.get_by_role("button", name="添加当前页书签").click()
        page.wait_for_timeout(350)
        rows = page.evaluate('JSON.parse(localStorage.getItem("qa:bookmarks:epub-xiao-wang-zi") || "[]")')
        assert len(rows) == 1, rows
        record("bookmark uses native row shape and persists", len(rows))
        page.keyboard.press("Escape"); page.keyboard.press("Escape"); page.wait_for_selector('[data-testid="shelf-continue"]')
        page.wait_for_selector('[data-testid="reader-screen"]', state='detached')
        after_exit = page.evaluate('JSON.parse(localStorage.getItem("qa:locator:epub-xiao-wang-zi"))')
        assert after_exit and after_exit['ratio'] > 0.4, after_exit
        record("reader unmount retains the durable nonzero locator", after_exit)
        context.close()
        context = new_context(browser, viewport)
        # Match packaged Tauri: a nonce disables unsafe-inline for style blocks.
        # The reader must copy the app-head nonce only after sanitizing/scoping CSS.
        def strict_styles(route):
            response = route.fetch()
            headers = {**response.headers, 'content-security-policy': "style-src 'self' 'nonce-qa-trusted-style'; style-src-attr 'unsafe-inline'"}
            route.fulfill(response=response, headers=headers)
        context.route(server.url, strict_styles)
        page = context.new_page(); page.on("pageerror", lambda error: errors.append(str(error)))
        page.add_init_script(MOCK_JS)
        page.add_init_script(r"""(() => {
          localStorage.setItem('mmbook-auto-focus', 'true');
          localStorage.setItem('mmbook-font-scale', '1.3');
          const original = window.__TAURI_INTERNALS__.invoke;
          window.__TAURI_INTERNALS__.invoke = async (command, args) => {
            const value = await original(command, args);
            if (command === 'get_readable_chapter') value.content = '<html><head><style>body { font-size: 99px; color: red; } p { line-height: 1.8; letter-spacing: 0.5px; }</style></head><body><p id="long">' + '长句阅读验证需要保留上下文而不会跳到下一句话'.repeat(100) + '。</p><p id="short">这是下一句。</p><p>最后一句。</p></body></html>';
            return value;
          };
        })();""")
        page.goto(server.url)
        page.evaluate("() => { const style=document.createElement('style'); style.setAttribute('nonce','qa-trusted-style'); style.textContent=':root { --qa-trusted-style: 1; }'; document.head.append(style); }")
        open_epub(page)
        page.wait_for_selector('[data-testid="focus-hud"]')
        page.wait_for_timeout(600)
        assert page.locator('[data-testid="focus-hud"]').inner_text().find('1 / 3') >= 0
        chrome_font = page.locator('[data-testid="reader-title"]').evaluate('(el) => getComputedStyle(el).fontSize') if page.locator('[data-testid="reader-title"]').count() else page.locator('[data-testid="reader-back"]').evaluate('(el) => getComputedStyle(el).fontSize')
        assert chrome_font != '99px', chrome_font
        spacing = page.locator('.epub-content #long').evaluate('(el) => getComputedStyle(el).letterSpacing')
        assert spacing == '0.5px', page.evaluate("() => ({spacing:getComputedStyle(document.querySelector('#long')).letterSpacing,head:[...document.head.querySelectorAll('style')].map(s=>({nonce:s.nonce,css:s.textContent.slice(0,120)})),article:[...document.querySelectorAll('.epub-content style')].map(s=>({nonce:s.nonce,css:s.textContent.slice(0,200)}))})")
        assert page.locator('.epub-content style').evaluate_all("els => els.every(el => el.nonce === 'qa-trusted-style')")
        record("scoped EPUB styles work under nonce CSP without leaking to chrome", chrome_font)
        initial = page.locator('.epub-scroll').evaluate('(el) => el.scrollTop')
        page.get_by_role('button', name='下一句', exact=True).click()
        page.wait_for_timeout(750)
        first = page.locator('.epub-scroll').evaluate('(el) => el.scrollTop')
        assert first > initial + 200, (initial, first)
        assert '1 / 3' in page.locator('[data-testid="focus-hud"]').inner_text()
        # Three quick physical presses must queue three page steps within the same sentence.
        page.evaluate("() => { for(let i=0;i<3;i++) window.dispatchEvent(new CustomEvent('ir:volume-key', {detail:{dir:1}})); }")
        page.wait_for_timeout(1900)
        queued = page.locator('.epub-scroll').evaluate('(el) => el.scrollTop')
        assert queued > first + 400, (first, queued)
        assert '1 / 3' in page.locator('[data-testid="focus-hud"]').inner_text()
        record("130 percent long sentence pages before advancing", {"initial":initial,"first":first,"queued":queued})
        page.get_by_role('button', name='上一句', exact=True).click(); page.wait_for_timeout(750)
        reverse = page.locator('.epub-scroll').evaluate('(el) => el.scrollTop')
        assert reverse < queued - 200, (queued, reverse)
        record("reverse paging remains within sentence", reverse)
        page.screenshot(path=str(OUT / 'long-sentence-focus.png'))
        assert not errors, errors
        record("no unhandled runtime errors", len(errors))
        context.close(); browser.close()
    (OUT / 'report.json').write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding='utf-8')
if __name__ == '__main__': main()
