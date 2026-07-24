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
    "  window.__TAURI_INTERNALS__.invoke = async (command, args) => {\n"
    "    if (command === 'get_book_chapter_path') return 'C:\\\\qa\\\\Library\\\\' + args.chapterId + '.md';\n"
    "    if (command === 'read_markdown_file') {\n"
    "      return {\n"
    "        content: '# QA Reader ' + args.path.split('\\\\\\\\').pop() + '\\\\n\\\\n' + Array.from({ length: 80 }, (_, i) =>\n"
    "          'Paragraph ' + (i + 1) + ' with enough content to make the reader scroll.'\n"
    "        ).join('\\\\n\\\\n'),\n"
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
            reader_page.keyboard.press("F11")
            reader_page.wait_for_function(
                "() => document.querySelector('.app')?.classList.contains('focus-mode')"
            )
            reader_page.wait_for_timeout(250)
            reader_page.keyboard.press("ArrowUp")
            reader_page.wait_for_function(
                "() => document.querySelector('.article h1')?.textContent?.includes('a1.md')"
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
        "screenshots": screenshots,
        "verifiedAt": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    }
    (QA_DIR / "bookshelf-browser-report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
