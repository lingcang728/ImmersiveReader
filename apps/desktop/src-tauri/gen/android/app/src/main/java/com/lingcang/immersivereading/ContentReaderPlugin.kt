package com.lingcang.immersivereading

import android.app.Activity
import android.net.Uri
import android.provider.OpenableColumns
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File

@InvokeArg
class CopyToDirArgs {
  lateinit var uri: String
  lateinit var dir: String
}

/**
 * Copies a SAF `content://` pick into an app-private directory through
 * ContentResolver.openInputStream. The fs plugin's fd path
 * (`openAssetFileDescriptor`) may return null for some document providers,
 * and its Rust side panics (`unimplemented!()`) which leaves the invoke
 * pending forever — this command returns a real error instead, and derives
 * a filesystem-safe .md name so the importer can see the file.
 */
@TauriPlugin
class ContentReaderPlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun copyToDir(invoke: Invoke) {
    try {
      val args = invoke.parseArgs(CopyToDirArgs::class.java)
      if (args.uri.isBlank() || args.dir.isBlank()) {
        invoke.reject("missing uri or dir")
        return
      }
      val dir = File(args.dir)
      if (!dir.isDirectory && !dir.mkdirs()) {
        invoke.reject("cannot create staging dir: ${args.dir}")
        return
      }
      val uri = Uri.parse(args.uri)
      val resolver = activity.contentResolver
      val displayName = resolver
        .query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
        ?.use { cursor -> if (cursor.moveToFirst()) cursor.getString(0) else null }
      val target = uniqueTarget(dir, normalizeName(displayName, uri))
      resolver.openInputStream(uri)?.use { input ->
        target.outputStream().use { output -> input.copyTo(output) }
      } ?: run {
        invoke.reject("provider returned no stream for $uri")
        return
      }
      invoke.resolve(
        JSObject().apply {
          put("path", target.absolutePath)
          put("name", target.name)
        },
      )
    } catch (e: Exception) {
      invoke.reject(e.message ?: e.toString())
    }
  }

  /** Display names are free-form — strip separators/dot-prefixes and force a
   *  Markdown extension so `import_markdown_folder` collects the file. */
  private fun normalizeName(displayName: String?, uri: Uri): String {
    var name = displayName
      ?.substringAfterLast('/')
      ?.substringAfterLast('\\')
      ?.replace(Regex("[\\\\/:*?\"<>|]"), "_")
      ?.trimStart('.')
      ?.trim()
      ?.ifEmpty { null }
      ?: uri.lastPathSegment?.substringAfterLast(':')?.ifEmpty { null }
      ?: "picked-file"
    if (!name.endsWith(".md", true) && !name.endsWith(".markdown", true)) {
      name = name.removeSuffix(".txt").removeSuffix(".TXT") + ".md"
    }
    return name
  }

  private fun uniqueTarget(dir: File, name: String): File {
    var target = File(dir, name)
    if (!target.exists()) return target
    val stem = name.substringBeforeLast('.')
    val ext = name.substring(stem.length)
    var n = 2
    while (File(dir, "$stem-$n$ext").exists()) n += 1
    return File(dir, "$stem-$n$ext")
  }
}
