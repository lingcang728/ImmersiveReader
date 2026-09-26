package com.lingcang.immersivereading

import android.app.Activity
import android.content.Context
import android.net.Uri
import android.os.StatFs
import android.provider.OpenableColumns
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import androidx.documentfile.provider.DocumentFile
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import java.io.InputStream
import java.io.OutputStream
import java.nio.file.AtomicMoveNotSupportedException
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.security.KeyStore
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.Executors
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import org.json.JSONObject

@InvokeArg
class CopyToDirArgs {
  lateinit var uri: String
  lateinit var dir: String
}

@InvokeArg
class CopyTreeToDirArgs {
  lateinit var treeUri: String
  lateinit var dir: String
}

@InvokeArg
class SetVolumeKeyCaptureArgs {
  var enabled: Boolean = false
}

@InvokeArg
class SecretsTargetArgs {
  lateinit var target: String
}

@InvokeArg
class SecretsWriteArgs {
  lateinit var target: String
  lateinit var value: String
}

/**
 * Copies SAF `content://` picks into an app-private directory through
 * ContentResolver.openInputStream. The fs plugin's fd path
 * (`openAssetFileDescriptor`) may return null for some document providers,
 * and its Rust side panics (`unimplemented!()`) which leaves the invoke
 * pending forever — this command returns a real error instead.
 *
 * All file work runs on a cached background pool (never the main thread),
 * writes go to a `.part-*` temp file first and are moved into place
 * atomically, and expected failures carry a stable error code
 * (`FILE_TOO_LARGE`, `INSUFFICIENT_STORAGE`, `PATH_ESCAPE`, …).
 */
@TauriPlugin
class ContentReaderPlugin(private val activity: Activity) : Plugin(activity) {

  /** Expected rejection that surfaces [code] in the invoke error. */
  private class StageException(val code: String, message: String) : Exception(message)

  /** Running totals while walking a SAF tree for [copyTreeToDir]. */
  private class TreeBudget {
    var files = 0
    var bytes = 0L
  }

  private val io = Executors.newCachedThreadPool()

  companion object {
    private const val MAX_FILE_BYTES = 256L * 1024 * 1024
    private const val STORAGE_HEADROOM_BYTES = 8L * 1024 * 1024
    private const val MAX_TREE_BYTES = 2L * 1024 * 1024 * 1024
    private const val MAX_TREE_FILES = 2000
    private const val MAX_TREE_DEPTH = 16
    private const val COPY_BUFFER_BYTES = 64 * 1024
    private const val KEYSTORE_PROVIDER = "AndroidKeyStore"
    private const val SECRETS_PREF = "ir_secrets"
    private const val CIPHER_TRANSFORM = "AES/GCM/NoPadding"
    private const val GCM_TAG_BITS = 128
    private const val GCM_IV_BYTES = 12
  }

  // ---------------------------------------------------------------
  // copyToDir {uri, dir} -> {path, name}
  // ---------------------------------------------------------------

  @Command
  fun copyToDir(invoke: Invoke) {
    io.execute {
      try {
        val args = invoke.parseArgs(CopyToDirArgs::class.java)
        if (runCatching { args.uri.isBlank() || args.dir.isBlank() }.getOrDefault(true)) {
          invoke.reject("missing uri or dir")
          return@execute
        }
        val dir = File(args.dir)
        if (!dir.isDirectory && !dir.mkdirs()) {
          invoke.reject("cannot create staging dir: ${args.dir}")
          return@execute
        }
        val target = stageInto(dir, Uri.parse(args.uri), displayNameHint = null)
        invoke.resolve(
          JSObject().apply {
            put("path", target.absolutePath)
            put("name", target.name)
          },
        )
      } catch (e: StageException) {
        invoke.reject(e.message, e.code)
      } catch (e: Exception) {
        invoke.reject(e.message ?: e.toString())
      }
    }
  }

  // ---------------------------------------------------------------
  // copyTreeToDir {treeUri, dir} -> {files, rootName}
  // ---------------------------------------------------------------

  @Command
  fun copyTreeToDir(invoke: Invoke) {
    io.execute {
      try {
        val args = invoke.parseArgs(CopyTreeToDirArgs::class.java)
        if (runCatching { args.treeUri.isBlank() || args.dir.isBlank() }.getOrDefault(true)) {
          invoke.reject("missing treeUri or dir")
          return@execute
        }
        val dir = File(args.dir)
        if (!dir.isDirectory && !dir.mkdirs()) {
          invoke.reject("cannot create staging dir: ${args.dir}")
          return@execute
        }
        val root = DocumentFile.fromTreeUri(activity, Uri.parse(args.treeUri))
          ?: throw StageException("NO_TREE", "cannot open document tree ${args.treeUri}")
        val rootName = NameSanitizer.sanitize(root.name) ?: "picked-folder"
        val budget = TreeBudget()
        copyTree(root, dir.canonicalFile, budget, depth = 0)
        invoke.resolve(
          JSObject().apply {
            put("files", budget.files)
            put("rootName", rootName)
          },
        )
      } catch (e: StageException) {
        invoke.reject(e.message, e.code)
      } catch (e: Exception) {
        invoke.reject(e.message ?: e.toString())
      }
    }
  }

  /**
   * Recreates the SAF tree's relative structure under [dir]. Enforces the
   * tree-wide budgets (2 GiB total, 2000 files, depth 16); each file still
   * goes through the per-file temp+rename+size-check path in [stageInto].
   */
  private fun copyTree(node: DocumentFile, dir: File, budget: TreeBudget, depth: Int) {
    if (depth > MAX_TREE_DEPTH) {
      throw StageException("TREE_TOO_DEEP", "TREE_TOO_DEEP: nesting exceeds $MAX_TREE_DEPTH levels")
    }
    for (child in node.listFiles()) {
      if (child.isDirectory) {
        val sub =
          File(dir, NameSanitizer.sanitize(child.name) ?: "folder").canonicalFile
        if (!sub.startsWith(dir)) {
          throw StageException("PATH_ESCAPE", "PATH_ESCAPE: ${child.name} resolves outside ${dir.path}")
        }
        if (!sub.isDirectory && !sub.mkdirs()) {
          throw StageException("MKDIR_FAILED", "cannot create directory ${sub.path}")
        }
        copyTree(child, sub, budget, depth + 1)
      } else if (child.isFile) {
        if (budget.files + 1 > MAX_TREE_FILES) {
          throw StageException("TREE_TOO_MANY_FILES", "TREE_TOO_MANY_FILES: more than $MAX_TREE_FILES files")
        }
        val len = child.length()
        if (len > 0 && budget.bytes + len > MAX_TREE_BYTES) {
          throw StageException("TREE_TOO_LARGE", "TREE_TOO_LARGE: tree exceeds 2 GiB")
        }
        val staged = stageInto(dir, child.uri, displayNameHint = child.name, sizeHint = len)
        budget.files += 1
        budget.bytes += if (len > 0) len else staged.length()
      }
    }
  }

  // ---------------------------------------------------------------
  // Shared staging path: size checks -> sanitize -> temp -> rename
  // ---------------------------------------------------------------

  /**
   * Streams one `content://` document into [dir] via a `.part-<uuid>` temp
   * file followed by an atomic move. Returns the final, uniquely-named
   * file. A partial file never appears at the final name — any failure
   * deletes the temp file.
   */
  private fun stageInto(
    dir: File,
    uri: Uri,
    displayNameHint: String?,
    sizeHint: Long = -1L,
  ): File {
    val size = if (sizeHint >= 0) sizeHint else querySize(uri)
    if (size > MAX_FILE_BYTES) {
      throw StageException("FILE_TOO_LARGE", "FILE_TOO_LARGE: $size bytes exceeds the 256 MiB cap")
    }
    if (size >= 0) {
      val available = StatFs(dir.absolutePath).availableBytes
      if (available < size + STORAGE_HEADROOM_BYTES) {
        throw StageException(
          "INSUFFICIENT_STORAGE",
          "INSUFFICIENT_STORAGE: need ${size + STORAGE_HEADROOM_BYTES} bytes, $available available",
        )
      }
    }
    val canonicalDir = dir.canonicalFile
    val name = NameSanitizer.normalizeName(
      displayNameHint ?: queryDisplayName(uri),
      uri.lastPathSegment,
    )
    val target = NameSanitizer.uniqueTarget(canonicalDir, name).canonicalFile
    if (!target.startsWith(canonicalDir)) {
      throw StageException("PATH_ESCAPE", "PATH_ESCAPE: $name resolves outside ${canonicalDir.path}")
    }
    val tmp = File(canonicalDir, "${target.name}.part-${UUID.randomUUID()}")
    try {
      val stream = activity.contentResolver.openInputStream(uri)
        ?: throw StageException("NO_STREAM", "provider returned no stream for $uri")
      stream.use { input ->
        tmp.outputStream().use { output -> copyCapped(input, output) }
      }
      moveIntoPlace(tmp, target)
    } catch (e: Exception) {
      tmp.delete()
      throw e
    }
    return target
  }

  /** Copy with a hard byte cap so providers that under-report SIZE still
   *  cannot stream more than 256 MiB into the staging dir. */
  private fun copyCapped(input: InputStream, output: OutputStream) {
    val buffer = ByteArray(COPY_BUFFER_BYTES)
    var total = 0L
    while (true) {
      val read = input.read(buffer)
      if (read < 0) break
      total += read
      if (total > MAX_FILE_BYTES) {
        throw StageException("FILE_TOO_LARGE", "FILE_TOO_LARGE: stream exceeded the 256 MiB cap")
      }
      output.write(buffer, 0, read)
    }
  }

  private fun moveIntoPlace(tmp: File, target: File) {
    try {
      Files.move(tmp.toPath(), target.toPath(), StandardCopyOption.ATOMIC_MOVE)
      return
    } catch (_: AtomicMoveNotSupportedException) {
      try {
        Files.move(tmp.toPath(), target.toPath())
        return
      } catch (_: Exception) {
        // fall through to renameTo
      }
    } catch (_: Exception) {
      // Other IO failure on the atomic move — try a plain rename.
    }
    if (!tmp.renameTo(target)) {
      throw StageException("RENAME_FAILED", "could not move ${tmp.name} into place as ${target.name}")
    }
  }

  private fun querySize(uri: Uri): Long {
    return try {
      activity.contentResolver
        .query(uri, arrayOf(OpenableColumns.SIZE), null, null, null)
        ?.use { cursor ->
          if (cursor.moveToFirst() && !cursor.isNull(0)) cursor.getLong(0) else -1L
        }
        ?: -1L
    } catch (_: Exception) {
      -1L
    }
  }

  private fun queryDisplayName(uri: Uri): String? {
    return try {
      activity.contentResolver
        .query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
        ?.use { cursor -> if (cursor.moveToFirst()) cursor.getString(0) else null }
    } catch (_: Exception) {
      null
    }
  }

  // ---------------------------------------------------------------
  // takePendingOpenUris {} -> {uris: [...]}
  // ---------------------------------------------------------------

  @Command
  fun takePendingOpenUris(invoke: Invoke) {
    val uris = generateSequence { PendingIntents.pendingOpenUris.poll() }.toList()
    invoke.resolve(JSObject().apply { put("uris", JSArray(uris)) })
  }

  // ---------------------------------------------------------------
  // setVolumeKeyCapture {enabled}
  // ---------------------------------------------------------------

  @Command
  fun setVolumeKeyCapture(invoke: Invoke) {
    try {
      val args = invoke.parseArgs(SetVolumeKeyCaptureArgs::class.java)
      VolumeKeyBridge.enabled = args.enabled
      invoke.resolve(JSObject())
    } catch (e: Exception) {
      invoke.reject(e.message ?: e.toString())
    }
  }

  // ---------------------------------------------------------------
  // Android Keystore secrets: secretsRead / secretsWrite / secretsDelete
  // ---------------------------------------------------------------

  @Command
  fun secretsRead(invoke: Invoke) {
    io.execute {
      try {
        val args = invoke.parseArgs(SecretsTargetArgs::class.java)
        val target = validatedTarget(runCatching { args.target }.getOrNull())
        invoke.resolve(JSObject().apply { put("value", readSecret(target) ?: JSONObject.NULL) })
      } catch (e: StageException) {
        invoke.reject(e.message, e.code)
      } catch (e: Exception) {
        invoke.reject(e.message ?: e.toString())
      }
    }
  }

  @Command
  fun secretsWrite(invoke: Invoke) {
    io.execute {
      try {
        val args = invoke.parseArgs(SecretsWriteArgs::class.java)
        val target = validatedTarget(runCatching { args.target }.getOrNull())
        val value = runCatching { args.value }.getOrNull()
          ?: throw StageException("BAD_ARGS", "missing value")
        writeSecret(target, value)
        invoke.resolve(JSObject())
      } catch (e: StageException) {
        invoke.reject(e.message, e.code)
      } catch (e: Exception) {
        invoke.reject(e.message ?: e.toString())
      }
    }
  }

  @Command
  fun secretsDelete(invoke: Invoke) {
    io.execute {
      try {
        val args = invoke.parseArgs(SecretsTargetArgs::class.java)
        val target = validatedTarget(runCatching { args.target }.getOrNull())
        deleteSecret(target)
        invoke.resolve(JSObject())
      } catch (e: StageException) {
        invoke.reject(e.message, e.code)
      } catch (e: Exception) {
        invoke.reject(e.message ?: e.toString())
      }
    }
  }

  private fun validatedTarget(target: String?): String {
    return target?.takeIf { it.isNotBlank() }
      ?: throw StageException("BAD_ARGS", "missing target")
  }

  /** Keystore alias derived from the logical target name — aliases are
   *  per-app, and hashing keeps arbitrary target strings usable. */
  private fun aliasFor(target: String): String {
    val digest = MessageDigest.getInstance("SHA-256").digest(target.toByteArray(Charsets.UTF_8))
    val hex = digest.joinToString("") { "%02x".format(it) }
    return "ir-secret-${hex.take(16)}"
  }

  private fun secretKey(alias: String): SecretKey {
    val keyStore = KeyStore.getInstance(KEYSTORE_PROVIDER).apply { load(null) }
    (keyStore.getEntry(alias, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }
    val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE_PROVIDER)
    generator.init(
      KeyGenParameterSpec.Builder(
        alias,
        KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
      )
        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
        .setKeySize(256)
        .build(),
    )
    return generator.generateKey()
  }

  private fun readSecret(target: String): String? {
    val alias = aliasFor(target)
    val prefs = activity.getSharedPreferences(SECRETS_PREF, Context.MODE_PRIVATE)
    val encoded = prefs.getString(alias, null) ?: return null
    val blob = try {
      Base64.decode(encoded, Base64.DEFAULT)
    } catch (_: Exception) {
      return null
    }
    if (blob.size <= GCM_IV_BYTES) return null
    val iv = blob.copyOfRange(0, GCM_IV_BYTES)
    val ciphertext = blob.copyOfRange(GCM_IV_BYTES, blob.size)
    return try {
      val cipher = Cipher.getInstance(CIPHER_TRANSFORM)
      cipher.init(Cipher.DECRYPT_MODE, secretKey(alias), GCMParameterSpec(GCM_TAG_BITS, iv))
      String(cipher.doFinal(ciphertext), Charsets.UTF_8)
    } catch (_: Exception) {
      // Key invalidated or blob corrupted — drop it instead of wedging reads.
      deleteSecret(target)
      null
    }
  }

  private fun writeSecret(target: String, value: String) {
    val alias = aliasFor(target)
    val cipher = Cipher.getInstance(CIPHER_TRANSFORM)
    cipher.init(Cipher.ENCRYPT_MODE, secretKey(alias))
    val blob = cipher.iv + cipher.doFinal(value.toByteArray(Charsets.UTF_8))
    activity.getSharedPreferences(SECRETS_PREF, Context.MODE_PRIVATE)
      .edit()
      .putString(alias, Base64.encodeToString(blob, Base64.NO_WRAP))
      .apply()
  }

  private fun deleteSecret(target: String) {
    val alias = aliasFor(target)
    activity.getSharedPreferences(SECRETS_PREF, Context.MODE_PRIVATE)
      .edit()
      .remove(alias)
      .apply()
    try {
      KeyStore.getInstance(KEYSTORE_PROVIDER).apply { load(null) }.deleteEntry(alias)
    } catch (_: Exception) {
      // Entry may not exist — deletion stays idempotent.
    }
  }
}
