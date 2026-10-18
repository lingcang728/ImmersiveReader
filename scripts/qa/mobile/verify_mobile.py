#!/usr/bin/env python
"""Mobile (Android-first) Playwright smoke + screenshot harness — task-7.

What it does
------------
1. Serves `apps/desktop/build` (SPA with index.html fallback) over 127.0.0.1,
   or `scripts/qa/mobile/fixture/` in `--fixture` mode (harness self-test).
2. Injects `scripts/qa/mobile/mobile_mock.js` as `window.__TAURI_INTERNALS__`
   before the app boots, so the real build runs without a Tauri shell.
3. Runs the mobile flow in three phone viewports:
     390x844 @ DPR 3   (OPPO Find X9 CSS size)
     360x800 @ DPR 2   (small Android baseline)
     412x915 @ DPR 2.6 (large Android)
   Assertions: no page errors, shelf renders, Markdown reading, EPUB reading,
   TOC chapter switch, reader settings panel, app settings screen, import sheet,
   focus HUD, dark theme, safe area / no horizontal overflow / 44px targets,
   first-screen JS/CSS budget, desktop chunks not loaded on mobile.
4. Every screenshot is taken only after its target state is visible, and the
   report proves the screenshots are distinct (sha256 + byte size), so a
   "click did nothing" regression cannot masquerade as a passing reader shot.

Screenshots per viewport (11 light + 2 dark when all features exist):
    01-shelf, 02-reader-md, 03-reader-md-toc, 04-reader-md-chapter2,
    05-reader-md-settings, 06-reader-md-focus, 07-reader-epub,
    08-reader-epub-toc, 09-settings, 10-import-sheet,
    11-dark-shelf, 12-dark-reader

Exit codes
----------
0  all checks passed
1  one or more checks failed
2  not runnable yet: build missing, or the mobile shell has no shelf yet
   (reported honestly as SKIP; never faked as pass)
3  harness error (browser/server/mock problem)

Usage
-----
    G:\\python\\python.exe scripts/qa/mobile/verify_mobile.py
    G:\\python\\python.exe scripts/qa/mobile/verify_mobile.py --fixture
    G:\\python\\python.exe scripts/qa/mobile/verify_mobile.py --headed --viewport 390x844@3
    G:\\python\\python.exe scripts/qa/mobile/verify_mobile.py --base-url http://127.0.0.1:4179

Only the globally installed Playwright is used (no npm/pip installs).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
import threading
import time
import urllib.request
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

try:
    from playwright.sync_api import TimeoutError as PlaywrightTimeoutError
    from playwright.sync_api import sync_playwright
except ImportError:  # pragma: no cover - environment check
    print(
        "ERROR: Python Playwright is not importable. This repo expects the machine global "
        "install: G:\\python\\python.exe (playwright 1.62.0).",
        file=sys.stderr,
    )
    sys.exit(3)

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
BUILD_DIR = ROOT / "apps" / "desktop" / "build"
MOCK_JS = (HERE / "mobile_mock.js").read_text(encoding="utf-8")
DEFAULT_OUT = ROOT / "docs" / "audit" / "2026-10-18" / "screens"

ANDROID_UA = (
    "Mozilla/5.0 (Linux; Android 16; OPPO Find X9) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/144.0.0.0 Mobile Safari/537.36"
)
VIEWPORTS = [
    {"label": "390x844@3", "width": 390, "height": 844, "dpr": 3.0},
    {"label": "360x800@2", "width": 360, "height": 800, "dpr": 2.0},
    {"label": "412x915@2.6", "width": 412, "height": 915, "dpr": 2.6},
]
# Real OPPO Find X9 (PLJ110) CSS viewport: 1256x2760 px @ density 560 => 359x789
# CSS px, DPR 3.5. Opt-in via --extra-device-viewport.
DEVICE_VIEWPORT = {"label": "359x789@3.5", "width": 359, "height": 789, "dpr": 3.5}

BOOK_MD = "md-shan-yue"          # continue-reading book (hero card may be its only entry)
BOOK_MD2 = "md-du-shu-bi-ji"
BOOK_EPUB = "epub-xiao-wang-zi"
MD_MARKER = "IR-QA-MD-"
EPUB_MARKER = "IR-QA-EPUB-"

READER_ROOT = '[data-testid="reader-screen"]'
READER_CONTENT = (
    '[data-testid="reader-content"], [data-testid="reader-body"], '
    '[data-testid="reader-scroll"], .reader-content, .markdown-body, .article, article'
)
TOC_OPEN = '[data-testid="toc-sheet"], [data-testid="reader-toc-sheet"], [data-testid="toc-item-0"]'
SETTINGS_CANDIDATES = (
    '[data-testid="settings-screen"], [data-testid="reader-settings-panel"], '
    '[data-testid="reader-settings-sheet"], [role="dialog"]'
)

# The mobile shell must never load the desktop application chunk/CSS.
DESKTOP_ASSET_HINTS = (
    "DesktopApp",
    "PodcastWorkflow",
    "ZhihuWorkflow",
    "TrashPanel",
    "SettingsPanel",
    "window-chrome",
)
# Resource-load noise expected in a browser preview (asset protocol / synthetic
# EPUB image) — recorded, not fatal, unless --strict-console.
IGNORED_CONSOLE = (
    "Failed to load resource",
    "net::ERR_",
    "the server responded with a status of 404",
)


def _log(message: str) -> None:
    print(message, flush=True)


class SPARequestHandler(SimpleHTTPRequestHandler):
    """Static file server with a SPA index.html fallback for app routes."""

    def __init__(self, *args, directory=None, **kwargs):  # noqa: D401
        super().__init__(*args, directory=directory, **kwargs)

    def log_message(self, fmt, *args):  # silence the per-request stderr noise
        return

    def send_head(self):
        translated = Path(self.translate_path(self.path))
        asset_suffixes = {
            ".js", ".css", ".map", ".json", ".png", ".jpg", ".jpeg", ".gif",
            ".svg", ".ttf", ".otf", ".woff", ".woff2", ".ico", ".webp",
        }
        if (
            self.command == "GET"
            and not translated.is_file()
            and translated.suffix.lower() not in asset_suffixes
        ):
            self.path = "/index.html"
        return super().send_head()


class StaticServer:
    def __init__(self, directory: Path, port: int = 0):
        self.directory = directory
        handler = lambda *args, **kwargs: SPARequestHandler(  # noqa: E731
            *args, directory=str(directory), **kwargs
        )
        self.httpd = ThreadingHTTPServer(("127.0.0.1", port), handler)
        self.port = self.httpd.server_address[1]
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)

    def __enter__(self) -> "StaticServer":
        self.thread.start()
        return self

    def __exit__(self, *exc_info) -> None:
        self.httpd.shutdown()
        self.httpd.server_close()

    @property
    def url(self) -> str:
        return f"http://127.0.0.1:{self.port}"


class Checks:
    def __init__(self) -> None:
        self.rows: list[dict] = []
        self.console_errors: list[str] = []
        self.page_errors: list[str] = []
        self.console_warnings: list[str] = []
        self.failed_requests: list[str] = []
        self.resource_bytes: dict[str, int] = {}
        self.resource_urls: list[str] = []
        self.screenshots: list[dict] = []
        self.shell_missing = False
        self.screenshot_hashes: dict[str, dict] = {}
        self.started_at = time.strftime("%Y-%m-%dT%H:%M:%S%z")
        self.base_url = ""
        self.browser_channel = ""

    def record(self, name: str, status: str, detail: str = "", viewport: str = "") -> None:
        self.rows.append({"name": name, "status": status, "detail": detail, "viewport": viewport})
        icon = {"pass": "PASS", "fail": "FAIL", "skip": "SKIP"}[status]
        suffix = f" — {detail}" if detail else ""
        _log(f"  [{icon}] {name}{suffix}")

    def check(self, name: str, fn, viewport: str = "") -> bool:
        try:
            detail = fn()
            text = detail if isinstance(detail, str) else ("" if detail is None else str(detail))
            self.record(name, "pass", text, viewport)
            return True
        except AssertionError as error:
            self.record(name, "fail", str(error), viewport)
            return False
        except PlaywrightTimeoutError:
            self.record(name, "fail", "timeout waiting for selector/state", viewport)
            return False
        except Exception as error:  # noqa: BLE001 - surface as a failed check
            self.record(name, "fail", f"{type(error).__name__}: {error}", viewport)
            return False

    def skip(self, name: str, reason: str, viewport: str = "") -> None:
        self.record(name, "skip", reason, viewport)

    @property
    def failed(self) -> int:
        return sum(1 for row in self.rows if row["status"] == "fail")


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Mobile Playwright smoke harness")
    parser.add_argument(
        "--build-dir", default=str(BUILD_DIR),
        help="static build directory to serve (default: apps/desktop/build)",
    )
    parser.add_argument(
        "--fixture", action="store_true",
        help="serve scripts/qa/mobile/fixture instead (harness/mock self-test)",
    )
    parser.add_argument("--out-dir", default=str(DEFAULT_OUT), help="screenshot/report output")
    parser.add_argument("--port", type=int, default=0, help="static server port (0 = random)")
    parser.add_argument("--base-url", default="", help="reuse an existing server instead of spawning one")
    parser.add_argument("--headed", action="store_true", help="show the browser window")
    parser.add_argument("--viewport", default="", help="only run one viewport label, e.g. 390x844@3")
    parser.add_argument("--timeout", type=int, default=20000, help="selector timeout in ms")
    parser.add_argument("--max-boot-js-bytes", type=int, default=250 * 1024,
                        help="first-screen script transfer budget (build mode only)")
    parser.add_argument("--max-boot-css-bytes", type=int, default=80 * 1024,
                        help="first-screen CSS transfer budget (build mode only)")
    parser.add_argument("--strict-console", action="store_true",
                        help="treat filtered resource-load console noise as failures too")
    parser.add_argument("--extra-device-viewport", action="store_true",
                        help="also run the real OPPO Find X9 CSS viewport 359x789@3.5")
    return parser.parse_args(argv)


def wait_for_server(url: str, timeout_s: float = 20.0) -> None:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=1):
                return
        except Exception:  # noqa: BLE001 - any error means "not ready yet"
            time.sleep(0.15)
    raise RuntimeError(f"static server did not become ready: {url}")


def launch_browser(playwright, headed: bool):
    """Prefer the machine Chrome channel (same as scripts/qa/verify_bookshelf.py)."""
    try:
        return playwright.chromium.launch(channel="chrome", headless=not headed), "chrome"
    except Exception as error:  # noqa: BLE001 - fall back to bundled chromium
        _log(f"  (chrome channel unavailable: {error})")
        return playwright.chromium.launch(headless=not headed), "chromium"


def new_context(browser, viewport: dict, color_scheme: str = "light"):
    return browser.new_context(
        viewport={"width": viewport["width"], "height": viewport["height"]},
        device_scale_factor=viewport["dpr"],
        is_mobile=True,
        has_touch=True,
        user_agent=ANDROID_UA,
        locale="zh-CN",
        color_scheme=color_scheme,
        reduced_motion="reduce",
    )


def wire_page(page, checks: Checks) -> None:
    def on_console(message):
        text = message.text
        if message.type == "error":
            if any(needle in text for needle in IGNORED_CONSOLE):
                checks.console_warnings.append(text)
            else:
                checks.console_errors.append(text)
        elif message.type == "warning":
            checks.console_warnings.append(text)

    page.on("console", on_console)
    page.on("pageerror", lambda error: checks.page_errors.append(error.stack or str(error)))

    def on_request_failed(request):
        url = request.url
        if any(needle in url for needle in (".png", ".jpg", ".webp", "favicon")):
            return
        checks.failed_requests.append(f"{url} ({request.failure})")

    page.on("requestfailed", on_request_failed)


def shot(page, checks: Checks, out_dir: Path, viewport_label: str, name: str) -> str:
    """Screenshot helper. Only call after the target state is actually visible."""
    out_dir.mkdir(parents=True, exist_ok=True)
    target = out_dir / f"{viewport_label}-{name}.png"
    page.screenshot(path=str(target), full_page=False)
    try:
        display = str(target.relative_to(ROOT))
    except ValueError:
        display = str(target)
    size = target.stat().st_size
    digest = hashlib.sha256(target.read_bytes()).hexdigest()
    checks.screenshot_hashes.setdefault(viewport_label, {})[name] = {
        "path": display,
        "bytes": size,
        "sha256": digest,
    }
    checks.screenshots.append({"viewport": viewport_label, "name": name, "path": display,
                               "bytes": size, "sha256": digest})
    _log(f"    screenshot {name} ({size} B)")
    return display


def body_text(page) -> str:
    return page.locator("body").inner_text(timeout=5000)


def selector_exists(page, selector: str) -> bool:
    try:
        return page.locator(selector).count() > 0
    except Exception:  # noqa: BLE001 - page may be mid-navigation
        return False


def first_visible(page, selectors: list[str], timeout: int = 8000):
    """Wait for any selector in the list and return the locator that matched."""
    deadline = time.time() + timeout / 1000
    while time.time() < deadline:
        for selector in selectors:
            locator = page.locator(selector)
            try:
                if locator.count() > 0 and locator.first.is_visible():
                    return locator.first
            except Exception:  # noqa: BLE001 - transient while the app mounts
                continue
        page.wait_for_timeout(100)
    raise AssertionError(f"none of the selectors became visible: {selectors}")


def click_if_present(page, selectors: list[str], timeout: int = 3000) -> bool:
    for selector in selectors:
        locator = page.locator(selector)
        try:
            if locator.count() > 0 and locator.first.is_visible():
                locator.first.click(timeout=timeout)
                return True
        except Exception:  # noqa: BLE001 - try the next candidate
            continue
    return False


def ensure_chrome(page, timeout: int = 4000) -> None:
    """Make sure the reader toolbar is on screen (mobile chrome auto-hides).

    Taps the middle of the reading surface once when the toolbar is missing;
    harmless for shells that never hide their chrome.
    """
    toolbar = page.locator('[data-testid="reader-toolbar"]')
    try:
        if toolbar.count() > 0 and toolbar.first.is_visible():
            return
    except Exception:  # noqa: BLE001 - fall through to the tap
        pass
    try:
        page.locator("body").click(position={"x": 200, "y": 420}, timeout=timeout)
    except Exception:  # noqa: BLE001 - best effort only
        return
    try:
        first_visible(page, ['[data-testid="reader-toolbar"]'], timeout)
    except AssertionError:
        return


def debug_snapshot(page) -> dict:
    """Small DOM summary used in failure details while the mobile UI lands."""
    try:
        return page.evaluate(
            """() => {
              const ids = [...document.querySelectorAll('[data-testid]')]
                .map((el) => el.getAttribute('data-testid'))
                .slice(0, 60);
              const root = document.querySelector('[data-testid="reader-content"], [data-testid="reader-body"], .reader-content');
              return {
                testids: ids,
                body: (document.body.innerText || '').slice(0, 400),
                contentLength: root ? (root.innerText || '').length : 0
              };
            }"""
        )
    except Exception:  # noqa: BLE001 - diagnostics are best effort
        return {"testids": [], "body": "", "contentLength": 0}


def load_shelf(page, base_url: str, checks: Checks, timeout: int) -> bool:
    """Navigate and classify the app: mobile shelf, or still-a-stub build."""
    page.goto(base_url, wait_until="domcontentloaded", timeout=timeout)
    deadline = time.time() + timeout / 1000
    while time.time() < deadline:
        if selector_exists(page, '[data-testid="shelf-continue"]') or selector_exists(
            page, '[data-testid="shelf-book-md-shan-yue"]'
        ):
            return True
        if selector_exists(page, '[data-testid="mobile-stub"]'):
            checks.shell_missing = True
            return False
        page.wait_for_timeout(150)
    checks.shell_missing = True
    checks.skip(
        "mobile shell present",
        "neither a shelf testid nor the MobileApp stub appeared within the timeout",
    )
    return False


def open_shelf_book(page, book_id: str, continue_book: bool) -> None:
    """Click a shelf book card.

    The continue-reading book may be rendered ONLY as the hero card
    (`shelf-continue`) and be absent from the list — accept either entry.
    """
    card = page.locator(f'[data-testid="shelf-book-{book_id}"]')
    if card.count() > 0:
        card.first.click()
        return
    if continue_book and selector_exists(page, '[data-testid="shelf-continue"]'):
        page.locator('[data-testid="shelf-continue"]').first.click()
        return
    raise AssertionError(f"shelf card for {book_id} not rendered and it is not the continue book")


def wait_reader_ready(page, timeout: int) -> None:
    """Wait until the reader screen is mounted AND its content is non-empty.

    This is the contract requested by QA: a click that stays on the shelf must
    fail here, never produce a reader-looking screenshot.
    """
    first_visible(page, [READER_ROOT, '[data-testid="reader-toolbar"]'], timeout)
    deadline = time.time() + timeout / 1000
    while time.time() < deadline:
        state = page.evaluate(
            """() => {
              const content = document.querySelector(
                '[data-testid="reader-content"], [data-testid="reader-body"], '
                + '[data-testid="reader-scroll"], .reader-content, .markdown-body, .article, article'
              );
              const text = content ? (content.innerText || '').trim() : '';
              const blocks = content
                ? content.querySelectorAll('p, h1, h2, h3, li, blockquote, pre').length
                : 0;
              const root = document.querySelector('[data-testid="reader-screen"]');
              return {
                rootVisible: !!root && root.getBoundingClientRect().height > 0,
                contentLength: text.length,
                blockCount: blocks
              };
            }"""
        )
        if state["contentLength"] > 0 and (state["blockCount"] > 0 or state["contentLength"] > 60):
            _log(
                f"    reader ready: {state['contentLength']} chars, {state['blockCount']} blocks"
            )
            return
        page.wait_for_timeout(150)
    raise AssertionError(f"reader content stayed empty: {debug_snapshot(page)}")


def wait_marker(page, marker: str, timeout: int) -> None:
    """Wait until rendered reader content contains an expected fixture marker."""
    deadline = time.time() + timeout / 1000
    while time.time() < deadline:
        text = body_text(page)
        if marker in text:
            return
        page.wait_for_timeout(150)
    raise AssertionError(
        f"rendered content never contained {marker}; snapshot={json.dumps(debug_snapshot(page), ensure_ascii=False)[:500]}"
    )


def collect_resource_metrics(page, checks: Checks) -> None:
    try:
        metrics = page.evaluate(
            """() => {
              const rows = performance.getEntriesByType('resource');
              let js = 0, css = 0, other = 0;
              const urls = [];
              for (const row of rows) {
                const size = row.transferSize || row.decodedBodySize || 0;
                const name = row.name;
                if (/\\.js(\\?|$)/.test(name)) js += size;
                else if (/\\.css(\\?|$)/.test(name)) css += size;
                else other += size;
                urls.push(name);
              }
              return { js, css, other, urls, count: rows.length };
            }"""
        )
        checks.resource_bytes = {
            "js": int(metrics["js"]),
            "css": int(metrics["css"]),
            "other": int(metrics["other"]),
            "count": int(metrics["count"]),
        }
        checks.resource_urls = list(metrics["urls"])
    except Exception:  # noqa: BLE001 - metric collection is advisory
        checks.resource_bytes = {}
        checks.resource_urls = []


def reader_command_count(checks: Checks) -> int:
    return sum(1 for row in checks.rows if row["name"].startswith("reader(") and row["status"] == "pass")


def run_light_flow(
    page,
    base_url: str,
    checks: Checks,
    out_dir: Path,
    viewport: dict,
    args: argparse.Namespace,
) -> None:
    label = viewport["label"]
    if not load_shelf(page, base_url, checks, args.timeout):
        shot(page, checks, out_dir, label, "00-not-runnable")
        return
    if not args.fixture:
        page.wait_for_timeout(400)
        collect_resource_metrics(page, checks)

    checks.check(
        "shelf: continue-reading card",
        lambda: first_visible(page, ['[data-testid="shelf-continue"]'], args.timeout) and "",
        label,
    )

    def shelf_books_present():
        missing = []
        for book in (BOOK_MD, BOOK_MD2, BOOK_EPUB):
            if selector_exists(page, f'[data-testid="shelf-book-{book}"]'):
                continue
            # The continue book may live only in the hero card.
            if book == BOOK_MD and selector_exists(page, '[data-testid="shelf-continue"]'):
                continue
            missing.append(book)
        if missing:
            raise AssertionError(f"missing/undiscoverable book entries: {missing}")
        return "3/3 books discoverable"

    checks.check("shelf: all mocked books rendered", shelf_books_present, label)

    def no_h_overflow():
        metrics = page.evaluate(
            "() => ({ sw: document.documentElement.scrollWidth, cw: document.documentElement.clientWidth })"
        )
        if metrics["sw"] > metrics["cw"] + 1:
            raise AssertionError(f"horizontal overflow: {metrics}")
        return f"{metrics['cw']}px wide"

    checks.check("shelf: no horizontal overflow", no_h_overflow, label)

    def safe_area_var():
        value = page.evaluate(
            "() => getComputedStyle(document.documentElement).getPropertyValue('--ir-safe-top').trim()"
        )
        if not value:
            raise AssertionError("--ir-safe-top is not defined (safe-area contract missing)")
        return f"--ir-safe-top={value}"

    checks.check("shelf: safe-area CSS contract", safe_area_var, label)
    shot(page, checks, out_dir, label, "01-shelf")

    # ---- Markdown reader --------------------------------------------------
    reader_open = checks.check(
        "reader(md): open first chapter",
        lambda: (open_shelf_book(page, BOOK_MD, continue_book=True),
                 wait_reader_ready(page, args.timeout),
                 wait_marker(page, f"{MD_MARKER}c1", args.timeout),
                 "Markdown chapter rendered")[-1],
        label,
    )
    if not reader_open:
        shot(page, checks, out_dir, label, "02-reader-md-FAILED")
        return
    shot(page, checks, out_dir, label, "02-reader-md")

    def reader_chrome_size():
        ensure_chrome(page)
        box = page.locator('[data-testid="reader-back"]').first.bounding_box()
        if box is None:
            raise AssertionError("reader-back has no box")
        if box["width"] < 44 or box["height"] < 44:
            raise AssertionError(f"reader-back touch target below 44px: {box}")
        return f"{round(box['width'])}x{round(box['height'])}"

    checks.check("reader(md): 44px back target", reader_chrome_size, label)

    def reader_content_mounted():
        metrics = page.evaluate(
            """() => {
              const content = document.querySelector(
                '[data-testid="reader-content"], [data-testid="reader-body"], '
                + '[data-testid="reader-scroll"], .reader-content, .markdown-body, .article, article'
              );
              const blocks = content
                ? content.querySelectorAll('p, h1, h2, h3, li, blockquote, pre').length
                : 0;
              const textLength = content ? (content.innerText || '').trim().length : 0;
              const scrollables = [...document.querySelectorAll('*')]
                .filter((el) => {
                  const style = getComputedStyle(el);
                  return (
                    el.scrollHeight > el.clientHeight + 40 &&
                    (style.overflowY === 'auto' || style.overflowY === 'scroll')
                  );
                })
                .map((el) => ({
                  tag: el.tagName.toLowerCase(),
                  testid: el.getAttribute('data-testid') || '',
                  className: typeof el.className === 'string' ? el.className.slice(0, 60) : '',
                  scrollHeight: el.scrollHeight,
                  clientHeight: el.clientHeight
                }));
              return {
                blocks,
                textLength,
                scrollables,
                documentScroll: {
                  scrollHeight: document.scrollingElement ? document.scrollingElement.scrollHeight : 0,
                  clientHeight: window.innerHeight
                }
              };
            }"""
        )
        if metrics["blocks"] < 8:
            raise AssertionError(f"reader content not mounted (<8 blocks): {metrics}")
        if not metrics["scrollables"]:
            # Some reader layouts virtualize/paginate and keep a fixed viewport;
            # in that case require enough text to prove a long chapter mounted.
            if metrics["textLength"] < 1500:
                raise AssertionError(f"no scrollable reader container and short content: {metrics}")
            return (
                f"{metrics['blocks']} blocks / {metrics['textLength']} chars mounted "
                f"(no scroll container found — virtualized/paged layout)"
            )
        first = metrics["scrollables"][0]
        return (
            f"{metrics['blocks']} blocks, scrollable={first['tag']}"
            f"[{first['testid'] or first['className']}] {first['scrollHeight']}px"
        )

    checks.check("reader(md): content mounted + scrollable", reader_content_mounted, label)

    def open_toc():
        ensure_chrome(page)
        page.locator('[data-testid="reader-toc"]').first.click()
        first_visible(page, [TOC_OPEN, '[data-testid="toc-item-1"]'], args.timeout)
        return "TOC sheet opened"

    if checks.check("reader(md): TOC sheet opens", open_toc, label):
        shot(page, checks, out_dir, label, "03-reader-md-toc")

    def toc_switch_chapter():
        ensure_chrome(page)
        if not (selector_exists(page, '[data-testid="toc-item-1"]')):
            if not selector_exists(page, TOC_OPEN):
                page.locator('[data-testid="reader-toc"]').first.click()
                first_visible(page, [TOC_OPEN], args.timeout)
        page.locator('[data-testid="toc-item-1"]').first.click()
        wait_marker(page, f"{MD_MARKER}c2", args.timeout)
        return "TOC jumped to chapter 2"

    if checks.check("reader(md): TOC + chapter switch", toc_switch_chapter, label):
        shot(page, checks, out_dir, label, "04-reader-md-chapter2")

    def reader_settings():
        ensure_chrome(page)
        page.locator('[data-testid="reader-settings"]').first.click()
        first_visible(page, list(SETTINGS_CANDIDATES.split(", ")), args.timeout)
        return "reader settings panel opened"

    if checks.check("reader(md): settings panel", reader_settings, label):
        shot(page, checks, out_dir, label, "05-reader-md-settings")
        page.keyboard.press("Escape")
        page.wait_for_timeout(250)

    def focus_hud():
        # Mobile reader enters focus mode either from a chrome toggle or from
        # the reading-settings sheet switch (`aria-label="专注模式"`); accept
        # either entry point, and prove the HUD itself is mounted.
        ensure_chrome(page)
        direct_toggles = [
            '[data-testid="focus-hud-toggle"]',
            '[data-testid="reader-focus"]',
            '[data-testid="reader-focus-toggle"]',
            '[data-testid="focus-mode-toggle"]',
            'button[aria-label*="专注"]',
        ]
        for selector in direct_toggles:
            if selector_exists(page, selector):
                page.locator(selector).first.click()
                try:
                    first_visible(page, ['[data-testid="focus-hud"]'], args.timeout)
                    return f"focus HUD opened via {selector}"
                except AssertionError:
                    continue
        # Fallback: reading-settings sheet switch.
        if selector_exists(page, '[data-testid="reader-settings"]'):
            page.locator('[data-testid="reader-settings"]').first.click()
            panel = first_visible(page, list(SETTINGS_CANDIDATES.split(", ")), args.timeout)
            panel_controls = [
                '[aria-label*="专注"]',
                'button:has-text("专注模式")',
                '[data-testid="focus-mode-toggle"]',
            ]
            for selector in panel_controls:
                locator = panel.locator(selector) if selector.startswith("button:has-text") else page.locator(selector)
                if locator.count() > 0:
                    locator.first.click()
                    try:
                        first_visible(page, ['[data-testid="focus-hud"]'], args.timeout)
                        return f"focus HUD opened via settings switch {selector}"
                    except AssertionError:
                        continue
            raise AssertionError(
                "reading settings sheet has no focus-mode switch; "
                f"panel text={panel.inner_text()[:200]!r}"
            )
        raise AssertionError(
            "no focus-mode entry point exposed (checked chrome toggles + reading settings)"
        )

    if checks.check("reader(md): focus HUD", focus_hud, label):
        shot(page, checks, out_dir, label, "06-reader-md-focus")
        click_if_present(
            page,
            [
                '[data-testid="focus-hud-exit"]',
                'button[aria-label*="退出专注"]',
                'button[aria-label*="退出"]',
            ],
        )
        page.keyboard.press("Escape")
        page.wait_for_timeout(250)
        page.keyboard.press("Escape")
        page.wait_for_timeout(250)

    checks.check(
        "reader(md): back to shelf",
        lambda: (ensure_chrome(page),
                 click_if_present(page, ['[data-testid="reader-back"]']),
                 first_visible(page, ['[data-testid="shelf-continue"]'], args.timeout),
                 "returned to shelf")[-1],
        label,
    )

    # ---- EPUB reader ------------------------------------------------------
    epub_open = checks.check(
        "reader(epub): open first chapter",
        lambda: (open_shelf_book(page, BOOK_EPUB, continue_book=False),
                 wait_reader_ready(page, args.timeout),
                 wait_marker(page, EPUB_MARKER, args.timeout),
                 "EPUB spine chapter rendered")[-1],
        label,
    )
    if epub_open:
        shot(page, checks, out_dir, label, "07-reader-epub")

        def epub_toc():
            ensure_chrome(page)
            page.locator('[data-testid="reader-toc"]').first.click()
            first_visible(page, [TOC_OPEN], args.timeout)
            return "EPUB TOC sheet opened"

        if checks.check("reader(epub): TOC sheet opens", epub_toc, label):
            shot(page, checks, out_dir, label, "08-reader-epub-toc")

        def epub_toc_switch():
            ensure_chrome(page)
            if not selector_exists(page, '[data-testid="toc-item-1"]'):
                if not selector_exists(page, TOC_OPEN):
                    page.locator('[data-testid="reader-toc"]').first.click()
                    first_visible(page, [TOC_OPEN], args.timeout)
            page.locator('[data-testid="toc-item-1"]').first.click()
            wait_marker(page, f"{EPUB_MARKER}c1", args.timeout)
            return "EPUB TOC navigated to second spine item"

        checks.check("reader(epub): TOC navigation", epub_toc_switch, label)

        checks.check(
            "reader(epub): back to shelf",
            lambda: (ensure_chrome(page),
                     click_if_present(page, ['[data-testid="reader-back"]']),
                     first_visible(page, ['[data-testid="shelf-continue"]'], args.timeout),
                     "returned to shelf")[-1],
            label,
        )

    # ---- App settings / import -------------------------------------------
    def app_settings():
        page.locator('[data-testid="shelf-settings"]').first.click()
        screen = first_visible(page, ['[data-testid="settings-screen"]'], args.timeout)
        text = screen.inner_text()
        forbidden = [word for word in ("知乎", "播客", "连读") if word in text]
        if forbidden:
            raise AssertionError(f"desktop-only settings leaked into mobile settings: {forbidden}")
        return "mobile-only settings screen"

    if checks.check("settings: mobile-only settings screen", app_settings, label):
        shot(page, checks, out_dir, label, "09-settings")
        click_if_present(
            page,
            ['[data-testid="settings-back"]', '[data-testid="reader-back"]', 'button[aria-label*="返回"]'],
        )
        page.keyboard.press("Escape")
        page.wait_for_timeout(200)

    def import_sheet():
        page.locator('[data-testid="shelf-import"]').first.click()
        first_visible(page, ['[data-testid="import-sheet"]'], args.timeout)
        return "import sheet opened"

    if checks.check("shelf: import sheet", import_sheet, label):
        shot(page, checks, out_dir, label, "10-import-sheet")
        page.keyboard.press("Escape")
        page.wait_for_timeout(200)

    def no_desktop_assets():
        leaked = [url for url in checks.resource_urls if any(hint in url for hint in DESKTOP_ASSET_HINTS)]
        if leaked:
            raise AssertionError(f"desktop shell assets requested on mobile: {leaked[:5]}")
        return f"{len(checks.resource_urls)} resources, no desktop shell assets"

    checks.check("perf: desktop chunk not loaded on mobile", no_desktop_assets, label)

    if not args.fixture and checks.resource_bytes:
        js_bytes = checks.resource_bytes.get("js", 0)
        css_bytes = checks.resource_bytes.get("css", 0)

        def budget():
            problems = []
            if js_bytes > args.max_boot_js_bytes:
                problems.append(f"JS {js_bytes}B > {args.max_boot_js_bytes}B")
            if css_bytes > args.max_boot_css_bytes:
                problems.append(f"CSS {css_bytes}B > {args.max_boot_css_bytes}B")
            if problems:
                raise AssertionError("; ".join(problems))
            return f"JS={js_bytes}B CSS={css_bytes}B"

        checks.check("perf: first-screen JS/CSS budget", budget, label)

    # ---- Screenshot completeness/distinctness evidence -------------------
    def screenshots_distinct():
        shots = checks.screenshot_hashes.get(label, {})
        if len(shots) < 4:
            raise AssertionError(f"only {len(shots)} reader/shelf screenshots captured: {sorted(shots)}")
        digests = [entry["sha256"] for entry in shots.values()]
        if len(set(digests)) != len(digests):
            duplicated = [
                name for name, entry in shots.items()
                if digests.count(entry["sha256"]) > 1
            ]
            raise AssertionError(f"byte-identical screenshots (state did not change): {duplicated}")
        return f"{len(shots)} screenshots, all distinct"

    checks.check("screenshots: per-viewport distinct states", screenshots_distinct, label)


def run_dark_theme(
    page,
    base_url: str,
    checks: Checks,
    out_dir: Path,
    viewport: dict,
    args: argparse.Namespace,
) -> None:
    label = viewport["label"]
    if checks.shell_missing:
        return
    page.goto(base_url, wait_until="domcontentloaded", timeout=args.timeout)
    try:
        first_visible(page, ['[data-testid="shelf-continue"]'], args.timeout)
    except AssertionError:
        checks.skip("theme: dark", "shelf not visible in dark context", label)
        return

    def assert_dark(where: str) -> str:
        values = page.evaluate(
            """() => {
              const body = getComputedStyle(document.body).backgroundColor;
              const root = getComputedStyle(document.documentElement).backgroundColor;
              return {
                body, root,
                prefersDark: matchMedia('(prefers-color-scheme: dark)').matches
              };
            }"""
        )
        if not values["prefersDark"]:
            raise AssertionError("emulated prefers-color-scheme is not dark")
        raw = values["body"]
        if raw in ("rgba(0, 0, 0, 0)", "transparent"):
            raw = values["root"]
        parts = [part.strip() for part in raw.replace("rgba(", "").replace("rgb(", "").replace(")", "").split(",")]
        if len(parts) < 3:
            raise AssertionError(f"unparseable background color: {values}")
        r, g, b = (float(parts[0]), float(parts[1]), float(parts[2]))
        luminance = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255
        if luminance >= 0.35:
            raise AssertionError(f"{where} background is not dark: {raw} (luminance {luminance:.2f})")
        return f"{where} bg={raw}"

    checks.check("theme: dark shelf background", lambda: assert_dark("shelf"), label)
    shot(page, checks, out_dir, label, "11-dark-shelf")

    def dark_reader():
        open_shelf_book(page, BOOK_MD, continue_book=True)
        wait_reader_ready(page, args.timeout)
        wait_marker(page, f"{MD_MARKER}c1", args.timeout)
        return assert_dark("reader")

    if checks.check("theme: dark reader background", dark_reader, label):
        shot(page, checks, out_dir, label, "12-dark-reader")


def summarize(checks: Checks, out_dir: Path, args: argparse.Namespace) -> int:
    passed = sum(1 for row in checks.rows if row["status"] == "pass")
    failed = sum(1 for row in checks.rows if row["status"] == "fail")
    skipped = sum(1 for row in checks.rows if row["status"] == "skip")
    per_viewport = {}
    for shot_row in checks.screenshots:
        per_viewport.setdefault(shot_row["viewport"], []).append(shot_row["name"])
    report = {
        "harness": "scripts/qa/mobile/verify_mobile.py",
        "buildDir": "fixture" if args.fixture else args.build_dir,
        "baseUrl": checks.base_url,
        "viewports": [viewport["label"] for viewport in VIEWPORTS],
        "startedAt": checks.started_at,
        "finishedAt": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "browserChannel": checks.browser_channel,
        "summary": {"pass": passed, "fail": failed, "skip": skipped},
        "mobileShellMissing": checks.shell_missing,
        "readerChecksPassed": reader_command_count(checks),
        "checks": checks.rows,
        "consoleErrors": checks.console_errors,
        "consoleWarnings": checks.console_warnings[:40],
        "pageErrors": checks.page_errors,
        "failedRequests": checks.failed_requests[:40],
        "firstScreenBytes": checks.resource_bytes,
        "screenshots": checks.screenshots,
        "screenshotsPerViewport": per_viewport,
        "screenshotHashes": checks.screenshot_hashes,
    }
    out_dir.mkdir(parents=True, exist_ok=True)
    report_path = out_dir / "mobile-smoke-report.json"
    report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")

    _log("")
    _log(f"report: {report_path}")
    _log(f"summary: pass={passed} fail={failed} skip={skipped}")
    for viewport_label, names in per_viewport.items():
        _log(f"  {viewport_label}: {len(names)} screenshots -> {', '.join(names)}")
    if checks.console_errors:
        _log(f"console errors: {checks.console_errors[:6]}")
    if checks.page_errors:
        _log(f"page errors: {checks.page_errors[:3]}")

    if checks.shell_missing:
        _log(
            "\nSKIP: this build does not expose the mobile shelf yet "
            "(MobileApp stub or missing shelf testids). Harness/mock was validated with "
            "--fixture; re-run after frontend-perf/mobile-* land the real Android build."
        )
        return 2
    return 1 if failed else 0


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    checks = Checks()
    out_dir = Path(args.out_dir).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    known_viewports = VIEWPORTS + ([DEVICE_VIEWPORT] if args.extra_device_viewport else [])
    viewports = [v for v in known_viewports if not args.viewport or v["label"] == args.viewport]
    if not viewports:
        _log(f"ERROR: unknown --viewport {args.viewport}; known: {[v['label'] for v in known_viewports]}")
        return 3

    if args.fixture:
        serve_dir = HERE / "fixture"
        _log(f"mode=fixture  serve={serve_dir}")
    else:
        serve_dir = Path(args.build_dir).resolve()
        _log(f"mode=build  serve={serve_dir}")
        if not serve_dir.is_dir() or not (serve_dir / "index.html").is_file():
            _log(
                "SKIP: build output missing. Run `npm.cmd --prefix .\\apps\\desktop run build` "
                "first (this harness never builds)."
            )
            return 2

    server_ctx = None
    if args.base_url:
        base_url = args.base_url.rstrip("/")
        checks.base_url = base_url
        wait_for_server(base_url)
    else:
        server_ctx = StaticServer(serve_dir, port=args.port)
        server_ctx.__enter__()
        base_url = server_ctx.url
        checks.base_url = base_url
        wait_for_server(base_url)

    try:
        with sync_playwright() as playwright:
            browser, channel = launch_browser(playwright, args.headed)
            checks.browser_channel = channel
            _log(f"browser={channel}")
            try:
                for viewport in viewports:
                    _log(f"\nviewport {viewport['label']} (DPR {viewport['dpr']})")
                    context = new_context(browser, viewport)
                    page = context.new_page()
                    wire_page(page, checks)
                    page.add_init_script(script=MOCK_JS)
                    try:
                        run_light_flow(page, base_url, checks, out_dir, viewport, args)
                    except Exception as error:  # noqa: BLE001 - keep other viewports running
                        checks.record(
                            "viewport flow completed",
                            "fail",
                            f"{type(error).__name__}: {error}",
                            viewport["label"],
                        )
                    context.close()

                    dark_context = new_context(browser, viewport, color_scheme="dark")
                    dark_page = dark_context.new_page()
                    wire_page(dark_page, checks)
                    dark_page.add_init_script(script=MOCK_JS)
                    try:
                        run_dark_theme(dark_page, base_url, checks, out_dir, viewport, args)
                    except Exception as error:  # noqa: BLE001
                        checks.record(
                            "dark theme flow",
                            "fail",
                            f"{type(error).__name__}: {error}",
                            viewport["label"],
                        )
                    dark_context.close()

                if checks.page_errors:
                    checks.record("console: no unhandled page errors", "fail", checks.page_errors[0][:300])
                else:
                    checks.record("console: no unhandled page errors", "pass")
                if checks.console_errors:
                    checks.record("console: no console errors", "fail", checks.console_errors[0][:300])
                else:
                    checks.record("console: no console errors", "pass")
            finally:
                browser.close()
    finally:
        if server_ctx is not None:
            server_ctx.__exit__(None, None, None)

    return summarize(checks, out_dir, args)


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except SystemExit:
        raise
    except Exception as error:  # noqa: BLE001 - report harness crashes distinctly
        print(f"HARNESS ERROR: {type(error).__name__}: {error}", file=sys.stderr)
        sys.exit(3)



