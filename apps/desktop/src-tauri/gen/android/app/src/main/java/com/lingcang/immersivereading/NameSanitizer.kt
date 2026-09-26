package com.lingcang.immersivereading

import java.io.File

/**
 * Pure-JVM file name helpers shared by [ContentReaderPlugin]. Kept free of
 * Android imports so local unit tests can exercise them directly.
 */
internal object NameSanitizer {

  private val ILLEGAL_CHARS = Regex("[\\\\/:*?\"<>|]")

  /**
   * Reduces a provider-supplied display name (or URI path segment) to a
   * safe file name: basename only, illegal characters become `_`, leading
   * dots and surrounding whitespace are stripped. Returns null when
   * nothing usable remains.
   */
  fun sanitize(raw: String?): String? {
    // Trim before stripping dots so " ..evil" cannot smuggle a dot-prefix
    // through as whitespace padding.
    return raw
      ?.substringAfterLast('/')
      ?.substringAfterLast('\\')
      ?.replace(ILLEGAL_CHARS, "_")
      ?.trim()
      ?.trimStart('.')
      ?.trim()
      ?.ifEmpty { null }
  }

  /**
   * Picks the best available name for a picked file: the provider's
   * display name when present, otherwise the URI's last path segment with
   * any `doc:`-style prefix dropped. Both branches go through the same
   * [sanitize] pass. The real extension is preserved — the Rust importer
   * decides which formats it accepts.
   */
  fun normalizeName(displayName: String?, lastPathSegment: String?): String {
    return sanitize(displayName)
      ?: sanitize(lastPathSegment?.substringAfterLast(':'))
      ?: "picked-file"
  }

  /**
   * Returns a collision-free target inside [dir]: `name` itself, or
   * `stem-2.ext`, `stem-3.ext`, … until a free slot is found.
   */
  fun uniqueTarget(dir: File, name: String): File {
    val target = File(dir, name)
    if (!target.exists()) return target
    val stem = name.substringBeforeLast('.')
    val ext = name.substring(stem.length)
    var n = 2
    while (File(dir, "$stem-$n$ext").exists()) n += 1
    return File(dir, "$stem-$n$ext")
  }
}
