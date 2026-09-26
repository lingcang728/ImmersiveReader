package com.lingcang.immersivereading

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.view.KeyEvent
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {

  /**
   * The app's single WebView, captured via [WryActivity.onWebViewCreate]
   * (WryActivity calls it from `setWebView` when the Rust side creates the
   * view). Used to push `ir:volume-key` DOM events while the reader owns
   * the volume keys.
   */
  @Volatile
  private var webView: WebView? = null

  override fun onCreate(savedInstanceState: Bundle?) {
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
  }

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
    VolumeKeyBridge.enabled = false
    webView = null
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
}
