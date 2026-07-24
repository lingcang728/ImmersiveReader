import json
import os
import shutil
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright


ROOT = Path(__file__).resolve().parents[2]
DESKTOP = ROOT / "apps" / "desktop"
QA_DIR = ROOT / "artifacts" / "qa"
ORIGIN = "http://127.0.0.1:4178"
MOCK_SCRIPT = (Path(__file__).with_name("bookshelf_mock.js")).read_text(encoding="utf-8")
READER_NAVIGATION_MOCK = (
    "(() => {\n"
    "  const baseInvoke = window.__TAURI_INTERNALS__.invoke;\n"
    "  window.__QA_CHAPTER_LOAD__ = { pending: null, lastStarted: null };\n"
    "  window.__TAURI_INTERNALS__.invoke = async (command, args) => {\n"
    "    if (command === 'get_book_chapter_path') return 'C:\\\\qa\\\\Library\\\\' + args.chapterId + '.md';\n"
    "    if (command === 'read_markdown_file') {\n"
    "      const chapterName = args.path.split('\\\\').pop();\n"
    "      if (chapterName === 'a1.md') {\n"
    "        window.__QA_CHAPTER_LOAD__.pending = chapterName;\n"
    "        window.__QA_CHAPTER_LOAD__.lastStarted = chapterName;\n"
    "        try {\n"
    "          await new Promise(resolve => setTimeout(resolve, 400));\n"
    "        } finally {\n"
    "          window.__QA_CHAPTER_LOAD__.pending = null;\n"
    "        }\n"
    "      }\n"
    "      const paragraphs = chapterName === 'a3.md'\n"
    "        ? ['Short chapter used to verify held-key chapter boundaries.']\n"
    "        : Array.from({ length: 80 }, (_, i) =>\n"
    "            'Paragraph ' + (i + 1) + ' with enough content to make the reader scroll.'\n"
    "          );\n"
    "      return {\n"
    "        content: '# QA Reader ' + chapterName + '\\\\n\\\\n' + paragraphs.join('\\\\n\\\\n'),\n"
    "        encoding: 'utf-8'\n"
    "      };\n"
    "    }\n"
    "    if (command === 'load_reading_state') return { scroll_position: 0, bookmarks: [], progress: 0 };\n"
    "    if (command === 'save_reading_state' || command === 'save_book_progress' || command === 'close_reader_session') return null;\n"
    "    if (command === 'get_file_mtime') return 1;\n"
    "    return baseInvoke(command, args);\n"
    "  };\n"
    "})();\n"
)


def wait_for_server(process: subprocess.Popen[str]) -> None:
    deadline = time.time() + 30
    while time.time() < deadline:
        if process.poll() is not None:
            raise RuntimeError("Vite preview exited before becoming ready")
        try:
            with urllib.request.urlopen(ORIGIN, timeout=1):
                return
        except Exception:
            time.sleep(0.2)
    raise TimeoutError("Timed out waiting for Vite preview")


def main() -> int:
    QA_DIR.mkdir(parents=True, exist_ok=True)
    creation_flags = subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0
    subprocess.run(
        ["npm.cmd", "run", "build"],
        cwd=DESKTOP,
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        creationflags=creation_flags,
    )
    node = shutil.which("node") or "node"
    vite = DESKTOP / "node_modules" / "vite" / "bin" / "vite.js"
    process = subprocess.Popen(
        [node, str(vite), "preview", "--host", "127.0.0.1", "--port", "4178", "--strictPort"],
        cwd=DESKTOP,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        text=True,
        creationflags=creation_flags,
    )
    screenshots: list[str] = []
    try:
        wait_for_server(process)
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(channel="chrome", headless=True)
            page = browser.new_page(viewport={"width": 900, "height": 700}, locale="zh-CN")
            browser_errors: list[str] = []
            page.on("pageerror", lambda error: browser_errors.append(error.stack or str(error)))
            page.add_init_script(MOCK_SCRIPT)
            page.goto(ORIGIN, wait_until="networkidle")
            page.wait_for_selector(".book-card", timeout=10000)
            assert page.locator(".book-card").count() == 3
            assert page.get_by_role("button", name="获取内容").count() == 1
            assert page.get_by_role("button", name="精读").count() == 3
            assert page.get_by_role("button", name="连读 ↗").count() == 3
            page.get_by_role("button", name="获取内容").click()
            assert page.get_by_role("button", name="归档知乎").count() >= 1
            page.keyboard.press("Escape")
            page.get_by_role("searchbox", name="搜索书架").fill("Jonathan")
            assert page.locator(".book-card").count() == 1
            page.get_by_role("searchbox", name="搜索书架").fill("")

            page.get_by_role("button", name="详情").first.click()
            page.wait_for_selector(".book-detail-dialog")
            assert page.locator(".book-detail-dialog h2").inner_text() == "你的ZombieMan · 知乎归档"
            assert page.get_by_role("button", name="继续阅读").count() == 1
            # Technical fields are folded by default (product: no always-visible revision grid).
            assert page.locator(".tech-details").count() == 1
            assert page.locator(".tech-details[open]").count() == 0
            assert page.locator(".tech-details[open] .provenance-grid").count() == 0
            assert page.locator(".task-history-item").count() == 0
            assert page.locator(".chapter-list li").count() <= 40
            page.locator(".tech-details summary").click()
            assert page.locator(".tech-details[open]").count() == 1
            assert "版本" in page.locator(".tech-details").inner_text()
            assert page.locator(".provenance-grid").count() == 1
            detail_target = QA_DIR / "bookshelf-detail-900x700.png"
            page.screenshot(path=str(detail_target), full_page=False)
            screenshots.append(str(detail_target))
            page.get_by_role("button", name="关闭详情").click()

            def assert_no_h_overflow(label: str) -> None:
                metrics = page.evaluate(
                    "() => ({ sw: document.documentElement.scrollWidth, cw: document.documentElement.clientWidth })"
                )
                assert metrics["sw"] == metrics["cw"], f"horizontal overflow at {label}: {metrics}"

            def assert_bookshelf_below_chrome(label: str) -> None:
                chrome = page.locator(".window-chrome").bounding_box()
                shelf = page.locator(".bookshelf").bounding_box()
                header = page.locator(".bs-header").bounding_box()
                assert chrome is not None and shelf is not None and header is not None
                chrome_bottom = chrome["y"] + chrome["height"]
                tolerance = 1
                assert shelf["y"] >= chrome_bottom - tolerance, (
                    f"bookshelf overlaps window chrome at {label}: "
                    f"chrome={chrome} shelf={shelf}"
                )
                assert header["y"] >= chrome_bottom - tolerance, (
                    f"bookshelf header overlaps window chrome at {label}: "
                    f"chrome={chrome} header={header}"
                )

            def assert_dialog_in_safe_area(selector: str) -> None:
                box = page.locator(selector).bounding_box()
                assert box is not None, f"missing dialog {selector}"
                viewport = page.viewport_size
                assert viewport is not None
                margin = 12
                assert box["x"] >= margin - 1, f"{selector} left out of safe area: {box}"
                assert box["y"] >= margin - 1, f"{selector} top out of safe area: {box}"
                assert box["x"] + box["width"] <= viewport["width"] - margin + 1
                assert box["y"] + box["height"] <= viewport["height"] - margin + 1
                # Desktop widths: dialog roughly centered horizontally.
                if viewport["width"] >= 900:
                    center = box["x"] + box["width"] / 2
                    assert abs(center - viewport["width"] / 2) < viewport["width"] * 0.12, (
                        f"{selector} not horizontally centered: {box} vp={viewport}"
                    )

            for width, height in (
                (600, 400),
                (900, 700),
                (1280, 800),
                (1366, 768),
                (1440, 900),
                (1920, 1080),
            ):
                page.set_viewport_size({"width": width, "height": height})
                page.wait_for_timeout(150)
                assert_no_h_overflow(f"{width}x{height}")
                assert_bookshelf_below_chrome(f"{width}x{height}")
                target = QA_DIR / f"bookshelf-{width}x{height}.png"
                page.screenshot(path=str(target), full_page=False)
                screenshots.append(str(target))

            # Font scale 100% / 150% (via CSS var used by the reader shell).
            page.set_viewport_size({"width": 1280, "height": 800})
            for scale in (1.0, 1.5):
                page.evaluate(
                    """(s) => {
                      document.documentElement.style.setProperty('--font-scale', String(s));
                    }""",
                    scale,
                )
                page.wait_for_timeout(100)
                assert_no_h_overflow(f"font-scale {int(scale * 100)}%")
                assert_bookshelf_below_chrome(f"font-scale {int(scale * 100)}%")

            # Fault injection for the reported regression: a stale immersive
            # overlay class must not be able to cover the visible homepage.
            page.locator(".chrome-stack").evaluate("el => el.classList.add('overlay')")
            assert_bookshelf_below_chrome("stale overlay class")
            page.locator(".chrome-stack").evaluate("el => el.classList.remove('overlay')")

            # Match the reported installed-app appearance as well as the
            # default light theme.
            page.evaluate("() => localStorage.setItem('mmbook-theme', 'suzhi-dark')")
            page.reload(wait_until="networkidle")
            page.wait_for_selector(".book-card")
            assert_bookshelf_below_chrome("dark theme")
            dark_target = QA_DIR / "bookshelf-dark-1280x800.png"
            page.screenshot(path=str(dark_target), full_page=False)
            screenshots.append(str(dark_target))

            # Settings: advanced section default collapsed; dialog in safe area.
            page.get_by_role("button", name="设置").first.click()
            page.wait_for_selector(".settings-panel")
            assert page.locator(".advanced-toggle").count() == 1
            assert page.locator(".advanced-block").count() == 0
            assert_dialog_in_safe_area(".settings-panel")
            page.locator(".advanced-toggle").click()
            page.wait_for_selector(".advanced-block")
            assert page.locator(".advanced-block").count() == 1
            page.get_by_role("button", name="关闭设置").click()

            # Detail dialog safe area + chapter cap already asserted above.
            page.get_by_role("button", name="详情").first.click()
            page.wait_for_selector(".book-detail-dialog")
            assert_dialog_in_safe_area(".book-detail-dialog")
            page.get_by_role("button", name="关闭详情").click()

            reader_page = browser.new_page(viewport={"width": 1280, "height": 800}, locale="zh-CN")
            reader_page.on("pageerror", lambda error: browser_errors.append(error.stack or str(error)))
            reader_page.add_init_script(MOCK_SCRIPT)
            reader_page.add_init_script(READER_NAVIGATION_MOCK)
            reader_page.goto(f"{ORIGIN}/?state=ready", wait_until="networkidle")
            for _ in range(10):
                reader_page.keyboard.press("Control+=")
            reader_page.wait_for_timeout(350)
            persisted_scale = reader_page.evaluate(
                "() => JSON.parse(sessionStorage.getItem('qa.reader-preferences') || '{}').fontScale"
            )
            assert persisted_scale == 1.5
            reader_page.evaluate("() => localStorage.clear()")
            reader_page.reload(wait_until="networkidle")
            reader_page.wait_for_selector(".book-card")
            restored_scale = reader_page.evaluate(
                "() => getComputedStyle(document.documentElement).getPropertyValue('--font-scale').trim()"
            )
            assert restored_scale == "1.5"
            reader_page.get_by_role("button", name="精读").first.click()
            reader_page.wait_for_selector(".article")
            initial_scroll = reader_page.locator(".content").evaluate("el => el.scrollTop")
            reader_page.keyboard.press("ArrowDown")
            reader_page.wait_for_timeout(180)
            assert reader_page.locator(".content").evaluate("el => el.scrollTop") > initial_scroll
            reader_page.locator(".content").evaluate(
                "el => { el.style.scrollBehavior = 'auto'; "
                "el.scrollTop = el.scrollHeight - el.clientHeight; }"
            )
            reader_page.wait_for_timeout(50)
            reader_page.keyboard.press("ArrowDown")
            reader_page.wait_for_function(
                "() => document.querySelector('.article h1')?.textContent?.includes('a2.md')"
            )
            reader_page.wait_for_timeout(550)

            def reader_scroll_snapshot() -> dict[str, float]:
                return reader_page.locator(".content").evaluate(
                    """el => ({
                      top: el.scrollTop,
                      max: Math.max(0, el.scrollHeight - el.clientHeight)
                    })"""
                )

            # A backward boundary gesture must reveal the previous chapter's
            # ending. Keep the current chapter visible while the delayed mock
            # prepares it, then carry the ArrowUp distance through the seam.
            reader_page.locator(".content").evaluate("el => { el.scrollTop = 0; }")
            reader_page.evaluate("() => { window.__QA_CHAPTER_LOAD__.lastStarted = null; }")
            reader_page.keyboard.press("ArrowUp")
            reader_page.wait_for_function(
                "() => window.__QA_CHAPTER_LOAD__?.lastStarted === 'a1.md'"
            )
            assert reader_page.evaluate(
                "() => window.__QA_CHAPTER_LOAD__?.pending"
            ) == "a1.md"
            assert "a2.md" in reader_page.locator(".article h1").inner_text()
            assert reader_page.locator(".loading").count() == 0
            reader_page.wait_for_function(
                "() => document.querySelector('.article h1')?.textContent?.includes('a1.md')"
            )
            reader_page.wait_for_timeout(250)
            backward_key_scroll = reader_scroll_snapshot()
            assert backward_key_scroll["max"] > 200
            assert abs(
                backward_key_scroll["top"] - (backward_key_scroll["max"] - 56)
            ) <= 2, f"backward key seam did not preserve its delta: {backward_key_scroll}"
            assert backward_key_scroll["top"] > backward_key_scroll["max"] * 0.8

            # Forward input carries the same line distance into chapter two.
            reader_page.locator(".content").evaluate(
                "el => { el.scrollTop = el.scrollHeight - el.clientHeight; }"
            )
            reader_page.keyboard.press("ArrowDown")
            reader_page.wait_for_function(
                """() => {
                  const el = document.querySelector('.content');
                  const title = document.querySelector('.article h1')?.textContent || '';
                  return !!el && title.includes('a2.md') && Math.abs(el.scrollTop - 56) <= 2;
                }"""
            )

            # Reverse immediately from the 56 px carried into chapter two.
            # A -120 px wheel gesture consumes those 56 px first, then carries
            # the remaining 64 px backward into chapter one's ending.
            content_box = reader_page.locator(".content").bounding_box()
            assert content_box is not None
            reader_page.mouse.move(
                content_box["x"] + content_box["width"] / 2,
                content_box["y"] + content_box["height"] / 2,
            )
            reader_page.mouse.wheel(0, -120)
            reader_page.wait_for_function(
                """() => {
                  const el = document.querySelector('.content');
                  const title = document.querySelector('.article h1')?.textContent || '';
                  if (!el || !title.includes('a1.md')) return false;
                  const max = Math.max(0, el.scrollHeight - el.clientHeight);
                  return max > 200 && Math.abs(el.scrollTop - (max - 64)) <= 2;
                }"""
            )
            # Reverse again immediately: 64 px returns to chapter one's edge,
            # and the remaining 56 px continues into chapter two.
            reader_page.mouse.wheel(0, 120)
            reader_page.wait_for_function(
                """() => {
                  const el = document.querySelector('.content');
                  const title = document.querySelector('.article h1')?.textContent || '';
                  return !!el && title.includes('a2.md') && Math.abs(el.scrollTop - 56) <= 2;
                }"""
            )

            # Advance to a long fourth chapter, then hold ArrowUp at its start.
            # The previous chapter is intentionally shorter than the viewport:
            # repeated keydown events must not cascade through several chapters
            # before the physical key is released.
            for chapter_name in ("a3.md", "a4.md"):
                reader_page.wait_for_timeout(550)
                reader_page.locator(".content").evaluate(
                    "el => { el.style.scrollBehavior = 'auto'; "
                    "el.scrollTop = el.scrollHeight - el.clientHeight; }"
                )
                reader_page.keyboard.press("ArrowDown")
                reader_page.wait_for_function(
                    "(name) => document.querySelector('.article h1')?.textContent?.includes(name)",
                    arg=chapter_name,
                )
            reader_page.wait_for_timeout(550)
            reader_page.locator(".content").evaluate("el => { el.scrollTop = 0; }")
            reader_page.evaluate(
                "() => window.dispatchEvent(new KeyboardEvent('keydown', "
                "{ key: 'ArrowUp', bubbles: true }))"
            )
            reader_page.wait_for_function(
                "() => document.querySelector('.article h1')?.textContent?.includes('a3.md')"
            )
            for _ in range(4):
                reader_page.evaluate(
                    "() => window.dispatchEvent(new KeyboardEvent('keydown', "
                    "{ key: 'ArrowUp', repeat: true, bubbles: true }))"
                )
                reader_page.wait_for_timeout(120)
            assert "a3.md" in reader_page.locator(".article h1").inner_text()
            reader_page.evaluate(
                "() => window.dispatchEvent(new KeyboardEvent('keyup', "
                "{ key: 'ArrowUp', bubbles: true }))"
            )

            # Return to the long chapter and compare vertical repeated input with
            # the precise horizontal step from the same focus unit.
            reader_page.locator(".content").evaluate(
                "el => { el.scrollTop = el.scrollHeight - el.clientHeight; }"
            )
            reader_page.keyboard.press("ArrowDown")
            reader_page.wait_for_function(
                "() => document.querySelector('.article h1')?.textContent?.includes('a4.md')"
            )
            reader_page.keyboard.press("F11")
            reader_page.wait_for_function(
                "() => document.querySelector('.app')?.classList.contains('focus-mode')"
            )
            reader_page.wait_for_timeout(250)

            def focus_snapshot() -> dict[str, object]:
                return reader_page.evaluate(
                    """() => ({
                      focused: Array.from(document.querySelectorAll(
                        '.article [data-focus-block="true"]'
                      )).filter((el) => el.style.opacity === '1')
                        .map((el) => (el.textContent || '').trim()),
                      scrollTop: document.querySelector('.content')?.scrollTop || 0
                    })"""
                )

            reader_page.keyboard.press("ArrowRight")
            reader_page.wait_for_timeout(500)
            right_step = focus_snapshot()
            reader_page.keyboard.press("ArrowLeft")
            reader_page.wait_for_timeout(500)
            reader_page.evaluate(
                "() => window.dispatchEvent(new KeyboardEvent('keydown', "
                "{ key: 'ArrowDown', repeat: true, bubbles: true }))"
            )
            reader_page.wait_for_timeout(160)
            reader_page.evaluate(
                "() => window.dispatchEvent(new KeyboardEvent('keyup', "
                "{ key: 'ArrowDown', bubbles: true }))"
            )
            reader_page.wait_for_timeout(500)
            down_step = focus_snapshot()
            assert down_step["focused"] == right_step["focused"]
            assert abs(float(down_step["scrollTop"]) - float(right_step["scrollTop"])) <= 2

            reader_page.keyboard.press("ArrowLeft")
            reader_page.wait_for_timeout(500)
            reader_page.keyboard.press("ArrowUp")
            reader_page.wait_for_function(
                "() => document.querySelector('.article h1')?.textContent?.includes('a3.md')"
            )
            assert "focus-mode" in (reader_page.locator(".app").get_attribute("class") or "")
            reader_page.locator("button.back-btn").dispatch_event("click")
            reader_page.wait_for_selector(".bookshelf")
            reader_page.locator(".bs-body").evaluate(
                "(el) => { const probe = document.createElement('div'); "
                "probe.dataset.qaProbe = 'reader-return-scroll'; "
                "probe.style.cssText = 'height:2400px;pointer-events:none;'; "
                "el.appendChild(probe); }"
            )
            reader_page.wait_for_timeout(350)
            assert "focus-mode" not in (reader_page.locator(".app").get_attribute("class") or "")
            body_box = reader_page.locator(".bs-body").bounding_box()
            assert body_box is not None
            reader_page.mouse.move(body_box["x"] + 300, body_box["y"] + 300)
            reader_page.mouse.wheel(0, 700)
            reader_page.wait_for_timeout(250)
            assert reader_page.locator(".bs-body").evaluate("el => el.scrollTop") > 0
            reader_page.close()

            loading_page = browser.new_page(viewport={"width": 900, "height": 700}, locale="zh-CN")
            loading_page.on("pageerror", lambda error: browser_errors.append(error.stack or str(error)))
            loading_page.add_init_script(MOCK_SCRIPT)
            loading_page.goto(f"{ORIGIN}/?state=loading", wait_until="domcontentloaded")
            loading_page.wait_for_selector(".empty-state")
            loading_target = QA_DIR / "bookshelf-loading-900x700.png"
            loading_page.screenshot(path=str(loading_target), full_page=False)
            screenshots.append(str(loading_target))
            loading_page.close()

            empty_page = browser.new_page(viewport={"width": 900, "height": 700}, locale="zh-CN")
            empty_page.on("pageerror", lambda error: browser_errors.append(error.stack or str(error)))
            empty_page.add_init_script(MOCK_SCRIPT)
            empty_page.goto(f"{ORIGIN}/?state=empty", wait_until="networkidle")
            empty_page.wait_for_selector(".empty-state")
            assert "书架还是空的" in empty_page.locator(".empty-state").inner_text()
            assert empty_page.locator(".brand-name").inner_text() == "沉浸阅读"
            assert empty_page.get_by_role("button", name="获取内容").count() == 1
            empty_target = QA_DIR / "bookshelf-empty-900x700.png"
            empty_page.screenshot(path=str(empty_target), full_page=False)
            screenshots.append(str(empty_target))
            empty_page.close()

            error_page = browser.new_page(viewport={"width": 900, "height": 700}, locale="zh-CN")
            error_page.on("pageerror", lambda error: browser_errors.append(error.stack or str(error)))
            error_page.add_init_script(MOCK_SCRIPT)
            error_page.goto(f"{ORIGIN}/?state=error", wait_until="networkidle")
            error_page.wait_for_selector(".state-banner.error")
            error_page.wait_for_selector(".brand-name", state="visible")
            assert error_page.locator(".state-banner.warning").count() == 1
            assert error_page.get_by_role("button", name="选择书库").count() == 1
            assert error_page.get_by_role("button", name="获取内容").count() == 1
            error_page.wait_for_timeout(150)
            error_target = QA_DIR / "bookshelf-error-900x700.png"
            error_page.screenshot(path=str(error_target), full_page=False)
            screenshots.append(str(error_target))
            error_page.close()
            if browser_errors:
                print(json.dumps(browser_errors, ensure_ascii=False), file=sys.stderr)
            assert browser_errors == []
            browser.close()
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()

    report = {
        "bookCount": 3,
        "chapterCount": 1469,
        "viewports": ["900x700", "1280x800", "1440x900"],
        "states": ["ready", "loading", "empty", "unwritable-with-corrupt-book"],
        "homepageLayout": [
            "bookshelf starts below window chrome at all tested viewports",
            "100% and 150% font scales keep the homepage below window chrome",
            "dark theme keeps the homepage below window chrome",
            "stale immersive overlay class cannot cover the homepage",
        ],
        "readerKeyboard": [
            "ordinary ArrowDown scroll",
            "ArrowUp reveals the previous chapter ending without a loading flash",
            "keyboard and wheel deltas carry through both chapter seam directions",
            "held ArrowUp crosses at most one chapter",
            "focus repeated ArrowDown matches ArrowRight focus step",
            "focus ArrowUp returns to previous chapter",
        ],
        "screenshots": screenshots,
        "verifiedAt": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    }
    (QA_DIR / "bookshelf-browser-report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
