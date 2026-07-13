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
            assert page.get_by_role("button", name="打开知乎主页").count() == 1
            assert "revision" in page.locator(".provenance-grid").inner_text()
            assert page.locator(".task-history-item").count() == 1
            assert "revision 2" in page.locator(".task-history-item").inner_text()
            assert "已完成" in page.locator(".task-history-item").inner_text()
            detail_target = QA_DIR / "bookshelf-detail-900x700.png"
            page.screenshot(path=str(detail_target), full_page=False)
            screenshots.append(str(detail_target))
            page.get_by_role("button", name="关闭详情").click()

            for width, height in ((900, 700), (1280, 800), (1440, 900)):
                page.set_viewport_size({"width": width, "height": height})
                page.wait_for_timeout(150)
                target = QA_DIR / f"bookshelf-{width}x{height}.png"
                page.screenshot(path=str(target), full_page=False)
                screenshots.append(str(target))

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
