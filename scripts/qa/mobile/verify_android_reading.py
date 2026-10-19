"""Actual Android WebView -> Tauri -> disk regression; isolated .qa package only."""
import argparse, base64, json, os, re, shutil, subprocess, time, urllib.request, zipfile
from pathlib import Path
from playwright.sync_api import sync_playwright
ROOT=Path(__file__).resolve().parents[3]
OUT=ROOT/'output/repair-20261018/android-device'; OUT.mkdir(parents=True,exist_ok=True)
PACKAGE='com.lingcang.immersivereading.qa'
SERIAL=None
ADB=shutil.which('adb.exe')
results=[]
def adb(*args):
    return subprocess.check_output([ADB,'-s',SERIAL,*args],text=True,encoding='utf-8',errors='replace',creationflags=subprocess.CREATE_NO_WINDOW).strip()
def record(name, evidence):
    results.append({'name':name,'evidence':evidence}); print('PASS',name,evidence,flush=True)
    (OUT/'report.json').write_text(json.dumps(results,ensure_ascii=False,indent=2),encoding='utf-8')

def status_bar_visible():
    dump=adb('shell','dumpsys','window','displays')
    states=re.findall(r'[^\n]*\b(?:mType|type)=statusBars\b[^\n]*\b(?:mVisible|visible)=(?:true|false)\b[^\n]*',dump)
    assert states, 'Native status-bar inset source unavailable'
    return any(re.search(r'\b(?:mVisible|visible)=true\b',line) for line in states)
def fixture():
    path=OUT/'long-sentence.epub'
    with zipfile.ZipFile(path,'w') as z:
        def add(name,data,**kwargs):
            z.writestr(zipfile.ZipInfo(name,(2026,10,18,0,0,0)),data,**kwargs)
        add('mimetype','application/epub+zip',compress_type=zipfile.ZIP_STORED)
        add('META-INF/container.xml','<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/book.opf" media-type="application/oebps-package+xml"/></rootfiles></container>')
        add('OEBPS/book.opf',"""<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="uid">ir-qa-reading</dc:identifier><dc:title>QA 长句与进度</dc:title><dc:language>zh</dc:language><meta property="dcterms:modified">2026-10-18T12:00:00Z</meta></metadata><manifest><item id="c0" href="Text/one.xhtml" media-type="application/xhtml+xml"/><item id="c1" href="Text/two.xhtml" media-type="application/xhtml+xml"/><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/><item id="css" href="Styles/book.css" media-type="text/css"/><item id="image" href="Images/deep/a/b/one.png" media-type="image/png"/></manifest><spine><itemref idref="c0"/><itemref idref="c1"/></spine></package>""")
        add('OEBPS/nav.xhtml','<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><a href="Text/one.xhtml">第一章</a></li><li><a href="Text/two.xhtml">第二章</a></li></ol></nav></body></html>')
        for name in ['one','two']:
            add(f'OEBPS/Text/{name}.xhtml','<html xmlns="http://www.w3.org/1999/xhtml"><head><link rel="stylesheet" href="../Styles/book.css"/></head><body><p id="long">'+('长句阅读验证需要保留上下文并在一句之内逐屏推进'*100)+'。</p><p id="short">这是下一句。</p><p>最后一句。</p><img src="../Images/deep/a/b/one.png" alt="QA图片"/></body></html>')
        add('OEBPS/Styles/book.css','@font-face {font-family: QAFont; src: url("../Fonts/qa.woff2");} body { font-family: serif; } p { line-height: 1.8; letter-spacing: 0.5px; }')
        add('OEBPS/Fonts/qa.woff2',(ROOT/'apps/desktop/node_modules/katex/dist/fonts/KaTeX_Main-Regular.woff2').read_bytes())
        add('OEBPS/Images/deep/a/b/one.png',base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j3l8AAAAASUVORK5CYII='))
    return path

def attach(playwright):
    process=adb('shell','pidof',PACKAGE).split()[0]
    adb('forward','tcp:9228',f'localabstract:webview_devtools_remote_{process}')
    for _ in range(40):
        try:
            with urllib.request.urlopen('http://127.0.0.1:9228/json/version',timeout=1) as response: json.load(response)
            browser=playwright.chromium.connect_over_cdp('http://127.0.0.1:9228')
            page=browser.contexts[0].pages[0]; page.wait_for_function('!!window.__TAURI_INTERNALS__',timeout=10000)
            return browser,page
        except Exception: time.sleep(.25)
    raise RuntimeError('QA WebView DevTools unavailable')
def invoke(page, command, args=None):
    return page.evaluate('([command,args]) => window.__TAURI_INTERNALS__.invoke(command,args||{})',[command,args])
def import_file(page,path):
    op=invoke(page,'begin_import',{'request':{'kind':'files','paths':[path]}})
    for _ in range(200):
        status=invoke(page,'get_import_status',{'opId':op['opId']})
        if status['status']!='running':
            assert status['status']=='succeeded',status
            return status['result']['bookId']
        time.sleep(.1)
    raise RuntimeError('Import timeout')
def open_book(page,book):
    selector=f'[data-testid="shelf-book-{book}"]'
    if page.locator(selector).count(): page.locator(selector).click()
    else: page.locator('[data-testid="shelf-continue"]').click()
    page.locator('.epub-content #long').wait_for(timeout=15000); page.wait_for_timeout(700)
def main():
    global SERIAL
    parser=argparse.ArgumentParser(); parser.add_argument('--apk',required=True); parser.add_argument('--serial',required=True); parser.add_argument('--skip-install',action='store_true'); args=parser.parse_args()
    SERIAL=args.serial
    assert PACKAGE.endswith('.qa') and ADB
    if not args.skip_install: print(adb('install','-r',args.apk),flush=True)
    device_dir=f'/sdcard/Android/data/{PACKAGE}/files/Download/repair-20261018'
    adb('shell','mkdir','-p',device_dir)
    # ADB creates these fixture folders as shell:ext_data_rw with mode 2770.
    # Allow the QA app to traverse only our synthetic fixture folders.
    adb('shell','chmod','755',f'/sdcard/Android/data/{PACKAGE}/files/Download',device_dir)
    path=fixture(); adb('push',str(path),device_dir+'/long.epub')
    adb('shell','am','force-stop',PACKAGE); print(adb('shell','am','start','-W','-n',PACKAGE+'/com.lingcang.immersivereading.MainActivity'),flush=True)
    with sync_playwright() as playwright:
        browser,page=attach(playwright)
        book=import_file(page,device_dir+'/long.epub')
        same=import_file(page,device_dir+'/long.epub'); assert book==same; record('native managed import deduplicates',book)
        scan=invoke(page,'scan_library'); assert sum(row['bookId']==book for row in scan['books'])==1 and not scan['issues'],scan; record('deep EPUB resources do not produce scan warnings',len(scan['books']))
        chapters=invoke(page,'open_book',{'bookId':book})['manifest']['chapters']; first_chapter=chapters[0]['id']; second_chapter=chapters[1]['id']
        locator={'schemaVersion':1,'bookId':book,'chapterId':second_chapter,'anchor':{'kind':'text','quote':'长句阅读验证','offset':0},'ratio':.52,'updated':'2026-10-18T14:00:00Z'}
        invoke(page,'save_reader_locator',{'bookId':book,'locatorJson':json.dumps(locator,ensure_ascii=False)})
        assert json.loads(invoke(page,'load_reader_locator',{'bookId':book}))==locator
        row=invoke(page,'add_reader_bookmark',{'bookId':book,'locatorJson':json.dumps(locator,ensure_ascii=False),'label':'QA真实书签'})
        assert len(invoke(page,'list_reader_bookmarks',{'bookId':book}))==1
        invoke(page,'remove_reader_bookmark',{'bookId':book,'bookmarkId':row['bookmarkId']})
        assert not invoke(page,'list_reader_bookmarks',{'bookId':book})
        progress=next(row['progress'] for row in invoke(page,'scan_library')['books'] if row['bookId']==book); assert abs(progress-.76)<.001,progress
        record('actual Android Tauri persistence and shelf progress',progress)
        rejected=page.evaluate("""async (bookId) => { let rejected=0; for(const [cmd,args] of [['get_reader_locator',{bookId}],['save_reader_locator',{bookId,locator:{}}]]) { try { await window.__TAURI_INTERNALS__.invoke(cmd,args); } catch { rejected++; } } return rejected; }""",book)
        assert rejected==2; record('unknown commands and wrong arguments rejected',rejected)
        # Reopen actual SQLite-backed cursor after process death.
        adb('shell','am','force-stop',PACKAGE); browser.close()
        adb('shell','am','start','-W','-n',PACKAGE+'/com.lingcang.immersivereading.MainActivity')
        browser,page=attach(playwright); assert json.loads(invoke(page,'load_reader_locator',{'bookId':book}))==locator
        record('locator survives Android process restart',book)
        # Begin focus at chapter start; no production preferences/data are touched.
        invoke(page,'save_reader_locator',{'bookId':book,'locatorJson':json.dumps({**locator,'chapterId':first_chapter,'anchor':{'kind':'ratio'},'ratio':0,'updated':'2026-10-18T14:00:01Z'})})
        page.evaluate("() => { localStorage.setItem('mmbook-auto-focus','true'); localStorage.setItem('mmbook-font-scale','1.3'); }")
        page.reload(); page.wait_for_selector('[data-testid="shelf-continue"], [data-testid^="shelf-book-"]'); open_book(page,book)
        page.wait_for_selector('[data-testid="focus-hud"]'); page.wait_for_timeout(600)
        geometry=page.evaluate("""() => {
          const body=document.querySelector('.reader-body').getBoundingClientRect();
          const hud=document.querySelector('[data-testid="focus-hud"]').getBoundingClientRect();
          const css=getComputedStyle(document.documentElement);
          return {top:body.top,bottom:body.bottom,viewport:innerHeight,mask:getComputedStyle(document.querySelector('.reader-body')).maskImage,hudTop:hud.top,hudGap:innerHeight-hud.bottom,safeTop:parseFloat(css.getPropertyValue('--ir-safe-top'))||0,safeBottom:parseFloat(css.getPropertyValue('--ir-safe-bottom'))||0};
        }""")
        assert geometry['top']==0 and abs(geometry['bottom']-geometry['viewport'])<1 and geometry['mask']!='none',geometry
        assert 3<=geometry['hudGap']-geometry['safeBottom']<=5,geometry
        assert not status_bar_visible()
        record('native status bar hidden; continuous fading viewport and floating lower HUD',geometry)
        assert '1 / 3' in page.locator('[data-testid="focus-hud"]').inner_text()
        start=page.locator('.epub-scroll').evaluate('(el) => el.scrollTop')
        adb('shell','input','keyevent','25'); page.wait_for_timeout(750)
        first=page.locator('.epub-scroll').evaluate('(el) => el.scrollTop'); assert first>start+200,(start,first)
        assert '1 / 3' in page.locator('[data-testid="focus-hud"]').inner_text()
        adb('shell','input','keyevent','--longpress','25'); page.wait_for_timeout(750)
        second=page.locator('.epub-scroll').evaluate('(el) => el.scrollTop'); assert 200<second-first<800,(first,second)
        record('physical volume key and long press advance one long-sentence page',{'start':start,'first':first,'longPress':second})
        images=page.locator('.epub-content img').evaluate_all('(imgs) => imgs.map(img => ({complete:img.complete,width:img.naturalWidth,src:img.src}))'); assert images and all(img['width']>0 for img in images),images
        record('relative EPUB image loads through real asset protocol',images)
        spacing=page.locator('.epub-content #long').evaluate('(el) => getComputedStyle(el).letterSpacing'); assert spacing=='0.5px',spacing
        fonts=page.evaluate("async () => { await document.fonts.load('16px QAFont'); return [...document.fonts].filter(f => f.family==='QAFont').map(f => f.status); }")
        assert fonts and all(status=='loaded' for status in fonts),fonts
        record('local stylesheet and relative font load through real CSP',{'spacing':spacing,'font':fonts})
        with (OUT/'focus-130-percent.png').open('wb') as screenshot:
            subprocess.run([ADB,'-s',SERIAL,'exec-out','screencap','-p'],stdout=screenshot,check=True,timeout=10,creationflags=subprocess.CREATE_NO_WINDOW)
        first_title=page.locator('[data-testid="reader-chapter-title"]').text_content()
        for _ in range(80):
            page.get_by_role('button',name='下一句',exact=True).click()
            page.wait_for_timeout(370)
            if page.locator('[data-testid="reader-chapter-title"]').text_content()!=first_title: break
        page.wait_for_timeout(650)
        assert page.locator('[data-testid="reader-chapter-title"]').text_content()!=first_title
        assert '1 / 3' in page.locator('[data-testid="focus-hud"]').inner_text()
        page.get_by_role('button',name='上一句',exact=True).click(); page.wait_for_timeout(900)
        assert page.locator('[data-testid="reader-chapter-title"]').text_content()==first_title
        assert '3 / 3' in page.locator('[data-testid="focus-hud"]').inner_text()
        record('focus crosses to next chapter start and previous chapter end',True)
        page.get_by_role('button',name='退出专注模式').click(); page.wait_for_timeout(200)
        # Background flush and in-app return retain the latest cursor.
        page.evaluate('window.dispatchEvent(new Event("pagehide"))'); page.wait_for_timeout(300)
        saved=json.loads(invoke(page,'load_reader_locator',{'bookId':book})); assert saved['ratio']>0,saved
        adb('shell','input','keyevent','3'); time.sleep(.5)
        adb('shell','am','start','-W','-n',PACKAGE+'/com.lingcang.immersivereading.MainActivity'); page.wait_for_timeout(500)
        assert json.loads(invoke(page,'load_reader_locator',{'bookId':book}))['chapterId']==saved['chapterId']
        record('background/foreground cursor retained',saved['ratio'])
        ensure=page.locator('[data-testid="reader-back"]'); ensure.click()
        page.wait_for_selector('[data-testid="reader-screen"]',state='detached')
        returned=json.loads(invoke(page,'load_reader_locator',{'bookId':book})); assert returned['ratio']>0,returned
        record('reader unmount returns to shelf without overwriting the native locator',returned['ratio'])
        page.wait_for_timeout(500); assert status_bar_visible(); record('status bar restored on shelf',True)
        # Native system Back unwinds focus first, then returns to the shelf.
        open_book(page,book); page.wait_for_selector('[data-testid="focus-hud"]')
        adb('shell','input','keyevent','4'); page.wait_for_timeout(400)
        assert page.locator('[data-testid="reader-screen"]').count() == 1
        assert not page.locator('[data-testid="focus-hud"]').count()
        adb('shell','input','keyevent','4'); page.wait_for_selector('[data-testid="shelf-continue"]')
        page.wait_for_timeout(500); assert status_bar_visible()
        record('native system Back exits focus then reader and restores status bar',True)
        (OUT/'gfxinfo.txt').write_text(adb('shell','dumpsys','gfxinfo',PACKAGE),encoding='utf-8')
        (OUT/'memory.txt').write_text(adb('shell','dumpsys','meminfo',PACKAGE),encoding='utf-8')
        browser.close()
    adb('forward','--remove','tcp:9228')
if __name__=='__main__': main()
