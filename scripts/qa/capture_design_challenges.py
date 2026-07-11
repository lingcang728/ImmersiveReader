import sys
from pathlib import Path

from playwright.sync_api import sync_playwright


ROOT = Path(__file__).resolve().parents[2]
PROTOTYPE = ROOT / "docs" / "design" / "prototype"
OUTPUT = ROOT / "docs" / "design" / "challenges"
STYLES = OUTPUT / "challenge-overrides.css"
BASE_STYLES = PROTOTYPE / "styles.css"
SCREENS = (
    ("index.html", "challenge-bookshelf-1440x900.png"),
    ("reading.html", "challenge-flow-1440x900.png"),
    ("focus.html", "challenge-focus-1440x900.png"),
)


def main() -> int:
    OUTPUT.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(channel="chrome", headless=True)
        page = browser.new_page(viewport={"width": 1440, "height": 900}, locale="zh-CN")
        for source, target in SCREENS:
            page.set_content((PROTOTYPE / source).read_text(encoding="utf-8"), wait_until="domcontentloaded")
            page.add_style_tag(path=str(BASE_STYLES))
            page.add_style_tag(path=str(STYLES))
            page.screenshot(path=str(OUTPUT / target), full_page=False)
        browser.close()
    for _, target in SCREENS:
        print(OUTPUT / target)
    return 0


if __name__ == "__main__":
    sys.exit(main())
