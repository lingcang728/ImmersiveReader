import json
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

from playwright.sync_api import sync_playwright


ROOT = Path(__file__).resolve().parents[2]
QA_DIR = ROOT / "artifacts" / "qa"
URL_FILE = QA_DIR / "reader-server.url"
MANIFEST_FILE = Path.home() / "Documents" / "沉浸阅读" / "Library" / "知乎" / "Jonathan Z" / "manifest.json"


def request_status(url: str, method: str = "GET", body: bytes | None = None, origin: str | None = None) -> int:
    headers = {"Content-Type": "application/json"}
    if origin:
        headers["Origin"] = origin
    request = urllib.request.Request(url, data=body, method=method, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=5) as response:
            return response.status
    except urllib.error.HTTPError as error:
        return error.code


def put_progress(url: str, origin: str, progress: dict[str, object]) -> int:
    return request_status(url, "PUT", json.dumps(progress).encode("utf-8"), origin)


def main() -> int:
    reader_url = URL_FILE.read_text(encoding="utf-8").strip()
    manifest = json.loads(MANIFEST_FILE.read_text(encoding="utf-8"))
    chapters = manifest["chapters"]
    parsed = urllib.parse.urlsplit(reader_url)
    origin = f"{parsed.scheme}://{parsed.netloc}"
    session_base = reader_url.rsplit("/reader", 1)[0]
    progress_url = f"{session_base}/progress"
    initial = {
        "schemaVersion": 1,
        "current": chapters[0]["id"],
        "position": 0.0,
        "read": [],
        "updated": "2026-07-10T08:00:00.000Z",
    }
    assert put_progress(progress_url, origin, initial) == 204

    security = {
        "invalidToken": request_status(f"{origin}/s/invalid/manifest"),
        "pathTraversal": request_status(f"{session_base}/content/%2e%2e/settings.json"),
        "controlFile": request_status(f"{session_base}/content/manifest.json"),
        "missingOriginWrite": request_status(progress_url, "PUT", json.dumps(initial).encode("utf-8")),
        "oversizedWrite": request_status(progress_url, "PUT", b"{" + b" " * 70000 + b"}", origin),
    }
    assert security == {
        "invalidToken": 403,
        "pathTraversal": 403,
        "controlFile": 403,
        "missingOriginWrite": 403,
        "oversizedWrite": 413,
    }

    screenshots: list[str] = []
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(channel="chrome", headless=True)
        page = browser.new_page(viewport={"width": 900, "height": 700}, locale="zh-CN")
        browser_errors: list[str] = []
        page.on("pageerror", lambda error: browser_errors.append(str(error)))
        page.goto(reader_url, wait_until="networkidle")
        page.wait_for_selector("#app-container:not(.hidden)")
        page.wait_for_selector(".article-card.active[data-rendered='true']")
        assert page.title() == f"{manifest['title']} - 沉浸阅读"
        assert page.locator(".article-card").count() == len(chapters)
        assert page.locator(".article-card[data-rendered='true']").count() <= 3
        assert page.locator(".article-card.active .article-title").inner_text() == chapters[0]["title"]
        assert page.locator(".article-card.active .article-meta-row .meta-badge").first.inner_text() == chapters[0]["date"]
        assert page.locator(".article-card.active .markdown-body-wrapper h1").count() == 0

        page.keyboard.press("Tab")
        active_after_tab = page.evaluate("document.activeElement?.id || document.activeElement?.className || document.activeElement?.tagName")
        if active_after_tab == "BODY":
            page.keyboard.press("Tab")
            active_after_tab = page.evaluate("document.activeElement?.id || document.activeElement?.className || document.activeElement?.tagName")
        assert active_after_tab == "menu-trigger", active_after_tab
        page.keyboard.press("Enter")
        page.wait_for_selector("#sidebar.active")
        page.wait_for_function("document.activeElement?.id === 'sidebar-search'")
        assert page.locator(".menu-item").count() == len(chapters)
        assert page.locator("button.menu-item").count() == len(chapters)
        assert page.locator("button.menu-group-header").count() > 0
        sidebar_target = QA_DIR / "reader-served-sidebar-900x700.png"
        page.screenshot(path=str(sidebar_target), full_page=False)
        screenshots.append(str(sidebar_target))
        page.keyboard.press("Escape")
        assert page.evaluate("document.activeElement?.id") == "menu-trigger"

        page.evaluate("document.activeElement?.blur()")
        page.keyboard.press("/")
        page.wait_for_selector("#search-overlay.active[role='dialog'][aria-modal='true']")
        assert page.evaluate("document.activeElement?.id") == "palette-input"
        page.locator("#palette-input").fill("dxs")
        page.wait_for_selector(".palette-item")
        assert page.locator("button.palette-item").count() > 0
        assert "大学室友" in page.locator(".palette-item-title").first.inner_text()
        page.keyboard.press("Shift+Tab")
        assert page.evaluate("document.activeElement?.classList.contains('palette-item')") is True
        page.keyboard.press("Escape")
        assert page.evaluate("document.activeElement?.id") == "menu-trigger"

        page.keyboard.press("j")
        page.wait_for_function(
            "expected => document.querySelector('.article-card.active .article-title')?.textContent === expected",
            arg=chapters[1]["title"],
        )
        page.wait_for_timeout(900)
        with urllib.request.urlopen(progress_url, timeout=5) as response:
            saved = json.loads(response.read().decode("utf-8"))
        assert saved["current"] == chapters[1]["id"]
        assert chapters[0]["id"] in saved["read"]

        for width, height in ((900, 700), (1280, 800), (1440, 900)):
            page.set_viewport_size({"width": width, "height": height})
            page.wait_for_timeout(250)
            target = QA_DIR / f"reader-served-{width}x{height}.png"
            page.screenshot(path=str(target), full_page=False)
            screenshots.append(str(target))

        page.keyboard.press("/")
        page.wait_for_selector("#search-overlay.active")
        page.wait_for_timeout(350)
        search_target = QA_DIR / "reader-served-search-1440x900.png"
        page.screenshot(path=str(search_target), full_page=False)
        screenshots.append(str(search_target))
        page.keyboard.press("Escape")

        page.reload(wait_until="networkidle")
        page.wait_for_selector(".article-card.active[data-rendered='true']")
        assert page.locator(".article-card.active .article-title").inner_text() == chapters[1]["title"]

        content_error_page = browser.new_page(viewport={"width": 900, "height": 700}, locale="zh-CN")
        content_error_page.route("**/content/**", lambda route: route.fulfill(status=404, body="chapter missing"))
        content_error_page.goto(reader_url, wait_until="networkidle")
        content_error_page.wait_for_selector(".article-card.active .article-error-placeholder")
        content_error_target = QA_DIR / "reader-content-error-900x700.png"
        content_error_page.screenshot(path=str(content_error_target), full_page=False)
        screenshots.append(str(content_error_target))
        content_error_page.close()

        manifest_error_page = browser.new_page(viewport={"width": 900, "height": 700}, locale="zh-CN")
        manifest_error_page.route("**/manifest", lambda route: route.fulfill(status=500, body="manifest damaged"))
        manifest_error_page.goto(reader_url, wait_until="networkidle")
        manifest_error_page.wait_for_selector(".landing-content[role='alert']")
        assert manifest_error_page.get_by_role("button", name="重新载入").count() == 1
        manifest_error_target = QA_DIR / "reader-manifest-error-900x700.png"
        manifest_error_page.screenshot(path=str(manifest_error_target), full_page=False)
        screenshots.append(str(manifest_error_target))
        manifest_error_page.close()

        assert browser_errors == []
        browser.close()

    report = {
        "readerUrlOrigin": origin,
        "chapterCount": len(chapters),
        "initialRenderedCountMax": 3,
        "searchQuery": "dxs",
        "restoredChapterId": chapters[1]["id"],
        "keyboard": ["Tab menu trigger", "Enter sidebar", "search focus trap", "Escape focus restore", "J next chapter"],
        "states": ["ready", "sidebar", "search", "content error", "manifest error"],
        "security": security,
        "screenshots": screenshots,
        "verifiedAt": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    }
    (QA_DIR / "reader-browser-report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
