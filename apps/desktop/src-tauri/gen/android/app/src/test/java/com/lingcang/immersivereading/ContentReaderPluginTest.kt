package com.lingcang.immersivereading

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/**
 * JVM tests for the pure name/target helpers in [NameSanitizer] — the same
 * logic ContentReaderPlugin applies to SAF picks before staging them.
 */
class ContentReaderPluginTest {

  @get:Rule
  val tmp = TemporaryFolder()

  @Test
  fun `sanitize strips path separators down to the basename`() {
    assertEquals("name.md", NameSanitizer.sanitize("dir/sub/name.md"))
    assertEquals("name.md", NameSanitizer.sanitize("dir\\sub\\name.md"))
    assertEquals("name.md", NameSanitizer.sanitize("name.md"))
  }

  @Test
  fun `sanitize replaces illegal characters with underscores`() {
    assertEquals("a_b_c_d_e_f_g_h.md", NameSanitizer.sanitize("a*b?c\"d<e>f|g:h.md"))
    assertEquals("a_b.md", NameSanitizer.sanitize("a:b.md"))
  }

  @Test
  fun `sanitize strips leading dots and surrounding whitespace`() {
    assertEquals("note.md", NameSanitizer.sanitize("..note.md"))
    assertEquals("note.md", NameSanitizer.sanitize("  note.md  "))
    assertEquals("note.md", NameSanitizer.sanitize(" ..note.md "))
  }

  @Test
  fun `sanitize returns null when nothing usable remains`() {
    assertNull(NameSanitizer.sanitize(null))
    assertNull(NameSanitizer.sanitize(""))
    assertNull(NameSanitizer.sanitize("   "))
    assertNull(NameSanitizer.sanitize("..."))
    assertNull(NameSanitizer.sanitize("/"))
  }

  @Test
  fun `normalizeName preserves the real extension`() {
    assertEquals("book.epub", NameSanitizer.normalizeName("book.epub", null))
    assertEquals("archive.zip", NameSanitizer.normalizeName("archive.zip", null))
    assertEquals("notes.txt", NameSanitizer.normalizeName("notes.txt", null))
    assertEquals("page.markdown", NameSanitizer.normalizeName("page.markdown", null))
  }

  @Test
  fun `normalizeName falls back to the last path segment`() {
    // content:// providers often expose "msf:123" style segments — the
    // prefix before ':' is dropped, then the same sanitization applies.
    assertEquals("name_.md", NameSanitizer.normalizeName(null, "msf:name?.md"))
    assertEquals("report.txt", NameSanitizer.normalizeName(null, "doc:report.txt"))
    assertEquals("picked-file", NameSanitizer.normalizeName(null, null))
    assertEquals("picked-file", NameSanitizer.normalizeName(null, "msf:..."))
    assertEquals("picked-file", NameSanitizer.normalizeName("   ", null))
  }

  @Test
  fun `normalizeName fallback branch sanitizes identically to displayName`() {
    val displayNamed = NameSanitizer.normalizeName("weird*name?.md", null)
    val fallback = NameSanitizer.normalizeName(null, "msf:weird*name?.md")
    assertEquals(displayNamed, fallback)
    assertEquals("weird_name_.md", fallback)
  }

  @Test
  fun `uniqueTarget returns the bare name when no collision exists`() {
    val dir = tmp.newFolder("inbox")
    assertEquals(File(dir, "a.md"), NameSanitizer.uniqueTarget(dir, "a.md"))
  }

  @Test
  fun `uniqueTarget appends -2 -3 suffixes on collision`() {
    val dir = tmp.newFolder("inbox")
    File(dir, "a.md").writeText("x")
    assertEquals(File(dir, "a-2.md"), NameSanitizer.uniqueTarget(dir, "a.md"))
    File(dir, "a-2.md").writeText("x")
    assertEquals(File(dir, "a-3.md"), NameSanitizer.uniqueTarget(dir, "a.md"))
  }

  @Test
  fun `uniqueTarget suffixes before the last extension`() {
    val dir = tmp.newFolder("inbox")
    File(dir, "a.b.md").writeText("x")
    assertEquals(File(dir, "a.b-2.md"), NameSanitizer.uniqueTarget(dir, "a.b.md"))
    File(dir, "README").writeText("x")
    assertEquals(File(dir, "README-2"), NameSanitizer.uniqueTarget(dir, "README"))
  }
}
