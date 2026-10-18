package com.lingcang.immersivereading

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.view.KeyEvent
import android.view.View
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.content.ContextCompat
import androidx.core.splashscreen.SplashScreen.Companion.installSplashScreen
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import androidx.webkit.ScriptHandler
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import java.util.Locale

class MainActivity : TauriActivity() {

  /**
   * The app's single WebView, captured via [WryActivity.onWebViewCreate]
   * (WryActivity calls it from `setWebView` when the Rust side creates the
   * view). Used to push `ir:volume-key` DOM events while the reader owns
   * the volume keys, and to inject the Android safe-area insets.
   */
  @Volatile
  private var webView: WebView? = null

  /**
   * Set in [onDestroy] before the WebView is torn down: every queued
   * `evaluateJavascript` closure checks it so a destroyed view is never
   * touched (calling into a destroyed WebView throws / logs hard).
   */
  @Volatile
  private var webViewDestroyed = false

  /** Last known safe-area insets, already converted to CSS pixels. */
  @Volatile
  private var safeArea: SafeArea = SafeArea.ZERO

  private var insetsListenerInstalled = false

  /**
   * Document-start injection that re-applies the last known insets to every
   * new document (live-reload / SPA reload / Tauri navigation).
   *
   * NOTE: Tauri/Wry owns the WebViewClient (`RustWebViewClient` does request
   * interception on the Rust side) — replacing it to hook `onPageFinished`
   * would break content loading. The document-start script is the supported
   * equivalent: it runs for every new document, before the SPA's first paint,
   * which is strictly better for avoiding a safe-area layout jump. Insets
   * changes are additionally pushed into the live document below.
   */
  private var safeAreaScript: ScriptHandler? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    // Must be the very first call and run before super.onCreate(): it installs
    // the platform (API 31+) / compat (API 26-30) splash and swaps the
    // starting theme for `postSplashScreenTheme` before WryActivity creates
    // the WebView.
    installSplashScreen()
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    enqueueOpenIntent(intent)
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    enqueueOpenIntent(intent)
  }

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    this.webView = webView
    webViewDestroyed = false
    configureWebView(webView)
    installSafeAreaInsets(webView)
  }

  /**
   * Rendering/UX tuning for the reader surface. Runs before the Rust side
   * loads the frontend, so the very first WebView frame already uses the
   * paper palette and the smooth-scrolling configuration.
   */
  private fun configureWebView(webView: WebView) {
    // Match the splash/window paper colour (浅 #faf8f5 / 深 #1e1e1c) so the
    // gap between "splash gone" and "content painted" never flashes white.
    webView.setBackgroundColor(ContextCompat.getColor(this, R.color.ir_window_background))
    // 120Hz AMOLED: no overscroll glow/bounce and no scrollbar gutter.
    webView.overScrollMode = View.OVER_SCROLL_NEVER
    webView.isVerticalScrollBarEnabled = false
    webView.isHorizontalScrollBarEnabled = false
    // Composite the page on the GPU instead of repainting it on the raster
    // thread — scrolling stays smooth on high-refresh displays.
    webView.setLayerType(View.LAYER_TYPE_HARDWARE, null)
    // Chrome DevTools attach is debug-only; a release APK must never expose it.
    if (BuildConfig.DEBUG) {
      WebView.setWebContentsDebuggingEnabled(true)
    }
  }

  /**
   * Watches the real window insets (status bar, navigation/gesture bar,
   * display cutout) and mirrors them into the document as
   * `--ir-safe-top/bottom/left/right` CSS pixels. The listener never consumes
   * the insets: Tauri's own layout keeps receiving them unchanged.
   */
  private fun installSafeAreaInsets(webView: WebView) {
    if (insetsListenerInstalled) return
    insetsListenerInstalled = true
    ViewCompat.setOnApplyWindowInsetsListener(webView) { _, insets ->
      updateSafeArea(insets)
      insets
    }
    // onWebViewCreate can run before the view is attached to the window; ask
    // for insets again once it is, so the first dispatch is not missed.
    webView.addOnAttachStateChangeListener(object : View.OnAttachStateChangeListener {
      override fun onViewAttachedToWindow(view: View) {
        ViewCompat.requestApplyInsets(view)
      }

      override fun onViewDetachedFromWindow(view: View) = Unit
    })
    ViewCompat.requestApplyInsets(webView)
    if (webView.isAttachedToWindow) {
      ViewCompat.requestApplyInsets(webView)
    }
  }

  private fun updateSafeArea(insets: WindowInsetsCompat) {
    val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars())
    val cutout = insets.getInsets(WindowInsetsCompat.Type.displayCutout())
    val gestures = insets.getInsets(WindowInsetsCompat.Type.mandatorySystemGestures())
    // Insets arrive in physical pixels; the WebView viewport uses
    // width=device-width so 1 CSS px == 1 density-independent pixel.
    val density = resources.displayMetrics.density.takeIf { it > 0f } ?: 1f
    val next = SafeArea(
      top = maxOf(bars.top, cutout.top).toDouble() / density,
      bottom = maxOf(bars.bottom, cutout.bottom, gestures.bottom).toDouble() / density,
      left = maxOf(bars.left, cutout.left).toDouble() / density,
      right = maxOf(bars.right, cutout.right).toDouble() / density,
    )
    if (next == safeArea) return
    safeArea = next
    registerSafeAreaDocumentScript()
    injectSafeAreaIntoCurrentPage()
    if (!WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)) {
      scheduleFallbackInjections()
    }
  }

  /** Re-registers the document-start script with the latest values. */
  private fun registerSafeAreaDocumentScript() {
    if (webViewDestroyed) return
    val view = webView ?: return
    if (!WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)) return
    safeAreaScript?.remove()
    safeAreaScript = try {
      WebViewCompat.addDocumentStartJavaScript(view, documentScript(safeArea), setOf("*"))
    } catch (_: RuntimeException) {
      // Some vendor WebView builds advertise the feature but reject the call;
      // the evaluateJavascript path below still covers the live document.
      null
    }
  }

  /** Pushes the current values into the already-loaded document. */
  private fun injectSafeAreaIntoCurrentPage() {
    val view = webView ?: return
    val area = safeArea
    view.post {
      if (webViewDestroyed || webView !== view) return@post
      view.evaluateJavascript(applyScript(area), null)
    }
  }

  /**
   * Fallback for WebViews without DOCUMENT_START_SCRIPT (pre-Chromium-83):
   * re-inject a few times after creation so the page that is still loading
   * picks the values up. Bounded — no recurring timer survives.
   */
  private fun scheduleFallbackInjections() {
    val view = webView ?: return
    for (delayMs in longArrayOf(0L, 250L, 750L, 1_500L, 3_000L)) {
      view.postDelayed({
        if (webViewDestroyed || webView !== view) return@postDelayed
        if (safeArea == SafeArea.ZERO) return@postDelayed
        view.evaluateJavascript(applyScript(safeArea), null)
      }, delayMs)
    }
  }

  /** One-shot injector used for the live document. */
  private fun applyScript(area: SafeArea): String =
    "(function(){var r=document.documentElement;if(!r||!r.style)return;" +
      "r.style.setProperty('--ir-safe-top','${formatCssPx(area.top)}');" +
      "r.style.setProperty('--ir-safe-bottom','${formatCssPx(area.bottom)}');" +
      "r.style.setProperty('--ir-safe-left','${formatCssPx(area.left)}');" +
      "r.style.setProperty('--ir-safe-right','${formatCssPx(area.right)}');" +
      "})();"

  /**
   * Runs at document start for every document. `document.documentElement` may
   * not exist yet at that point, so it retries on DOMContentLoaded/load.
   */
  private fun documentScript(area: SafeArea): String = """
    (function () {
      function apply() {
        var r = document.documentElement;
        if (!r || !r.style) return false;
        r.style.setProperty('--ir-safe-top', '${formatCssPx(area.top)}');
        r.style.setProperty('--ir-safe-bottom', '${formatCssPx(area.bottom)}');
        r.style.setProperty('--ir-safe-left', '${formatCssPx(area.left)}');
        r.style.setProperty('--ir-safe-right', '${formatCssPx(area.right)}');
        return true;
      }
      if (apply()) return;
      document.addEventListener('DOMContentLoaded', apply);
      window.addEventListener('load', apply);
    })();
  """.trimIndent()

  /** Locale.ROOT formatting: a comma decimal separator would be invalid CSS. */
  private fun formatCssPx(value: Double): String =
    String.format(Locale.US, "%.2fpx", value)

  /**
   * Volume keys become page-turn controls while the frontend asserts
   * capture through `setVolumeKeyCapture`. DOWN dispatches an
   * `ir:volume-key` CustomEvent (`dir` = -1 up / +1 down); DOWN/UP/
   * MULTIPLE are all consumed so the system volume UI never reacts.
   */
  override fun dispatchKeyEvent(event: KeyEvent): Boolean {
    if (VolumeKeyBridge.enabled &&
      (event.keyCode == KeyEvent.KEYCODE_VOLUME_UP ||
        event.keyCode == KeyEvent.KEYCODE_VOLUME_DOWN)
    ) {
      if (event.action == KeyEvent.ACTION_DOWN) {
        val dir = if (event.keyCode == KeyEvent.KEYCODE_VOLUME_UP) -1 else 1
        webView?.post {
          webView?.evaluateJavascript(
            "window.dispatchEvent(new CustomEvent('ir:volume-key',{detail:{dir:$dir}}))",
            null,
          )
        }
      }
      return true
    }
    return super.dispatchKeyEvent(event)
  }

  override fun onDestroy() {
    // Flip the guards before WryActivity destroys the native WebView: any
    // queued injection becomes a no-op from here on.
    webViewDestroyed = true
    VolumeKeyBridge.enabled = false
    val view = webView
    webView = null
    if (view != null) {
      ViewCompat.setOnApplyWindowInsetsListener(view, null)
    }
    safeAreaScript?.remove()
    safeAreaScript = null
    super.onDestroy()
  }

  /**
   * Queue document URIs handed to us by VIEW / SEND intents. Everything is
   * accepted as-is — the Rust side validates the queue contents when it
   * drains it via `takePendingOpenUris`.
   */
  private fun enqueueOpenIntent(intent: Intent?) {
    if (intent == null) return
    val uris = mutableListOf<String>()
    when (intent.action) {
      Intent.ACTION_VIEW -> intent.data?.let { uris.add(it.toString()) }
      Intent.ACTION_SEND, Intent.ACTION_SEND_MULTIPLE ->
        collectStream(intent.extras?.get(Intent.EXTRA_STREAM), uris)
    }
    PendingIntents.pendingOpenUris.addAll(uris)
  }

  /** EXTRA_STREAM arrives as a Uri, a String, or an ArrayList mixing both
   *  (SEND_MULTIPLE); normalize every entry to its string form. */
  private fun collectStream(value: Any?, out: MutableList<String>) {
    when (value) {
      null -> return
      is Uri -> out.add(value.toString())
      is String -> if (value.isNotBlank()) out.add(value)
      is Iterable<*> -> value.forEach { collectStream(it, out) }
      is Array<*> -> value.forEach { collectStream(it, out) }
    }
  }

  /**
   * Safe-area insets converted to CSS pixels. Equality ignores sub-pixel
   * jitter so re-dispatch (keyboard, status-bar shade) does not spam JS.
   */
  private data class SafeArea(
    val top: Double,
    val bottom: Double,
    val left: Double,
    val right: Double,
  ) {
    companion object {
      val ZERO = SafeArea(0.0, 0.0, 0.0, 0.0)
    }
  }
}
