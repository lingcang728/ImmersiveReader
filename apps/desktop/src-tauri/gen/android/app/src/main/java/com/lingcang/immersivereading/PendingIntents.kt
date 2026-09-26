package com.lingcang.immersivereading

import java.util.concurrent.ConcurrentLinkedQueue

/**
 * URIs delivered by ACTION_VIEW / ACTION_SEND / ACTION_SEND_MULTIPLE
 * intents. [MainActivity] enqueues them (including cold-start intents
 * arriving before the WebView is ready) and the Rust side drains the queue
 * through the content-reader plugin's `takePendingOpenUris` command.
 * Everything is accepted as-is — Rust validates what it can import.
 */
object PendingIntents {
  val pendingOpenUris = ConcurrentLinkedQueue<String>()
}

/**
 * Shared flag for volume-key capture (page-turn while reading). The
 * frontend asserts it through the `setVolumeKeyCapture` command and
 * [MainActivity.dispatchKeyEvent] consults it. The flag is not cleared on
 * pause/stop — the frontend re-asserts on state changes — but
 * [MainActivity.onDestroy] resets it.
 */
object VolumeKeyBridge {
  @Volatile
  var enabled: Boolean = false
}
