# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# ---------------------------------------------------------------------------
# ImmersiveReader native layer keeps (release builds run R8 full mode).
# The Wry/Tauri entry points are mostly covered by the generated
# proguard-wry.pro (src/main/java/.../generated) and the tauri-android AAR
# consumer rules; the keeps below make the critical reflection/JNI surface
# explicit so an AGP or Tauri template upgrade cannot silently strip it.
# ---------------------------------------------------------------------------

# Launch activity: instantiated by the platform from AndroidManifest.xml and
# its lifecycle/inset callbacks are invoked by the framework.
-keep class com.lingcang.immersivereading.MainActivity { *; }

# Wry JNI bridge: Rust resolves these classes and calls into them by exact
# name/signature (setWebView, loadUrlMainThread, evalScript, Ipc.postMessage…).
-keep class com.lingcang.immersivereading.WryActivity {
    public <init>(...);
    void setWebView(...);
    java.lang.Class getAppClass(...);
    java.lang.String getVersion();
}
-keep class com.lingcang.immersivereading.RustWebView {
    public <init>(...);
    void loadUrlMainThread(...);
    void loadHTMLMainThread(...);
    void evalScript(...);
}
-keep class com.lingcang.immersivereading.RustWebViewClient,com.lingcang.immersivereading.RustWebChromeClient {
    public <init>(...);
}
-keep class com.lingcang.immersivereading.Ipc {
    public <init>(...);
    @android.webkit.JavascriptInterface public <methods>;
}

# In-app Kotlin plugin: Rust registers it by name
# (register_android_plugin("com.lingcang.immersivereading", "ContentReaderPlugin"))
# and Jackson deserializes its @InvokeArg payloads through field reflection.
-keep class com.lingcang.immersivereading.ContentReaderPlugin {
    public <init>(android.app.Activity);
}
-keep @app.tauri.annotation.InvokeArg class * { *; }
-keep @app.tauri.annotation.TauriPlugin public class * {
    @app.tauri.annotation.Command public <methods>;
    @app.tauri.annotation.PermissionCallback <methods>;
    @app.tauri.annotation.ActivityCallback <methods>;
    public <init>(...);
}

# WebView JavascriptInterface methods are resolved by name from the engine.
-keepclassmembers class * {
    @android.webkit.JavascriptInterface <methods>;
}

# Keep annotation metadata: Tauri/Jackson reflection matches on annotations at
# runtime and some paths re-read them after shrinking.
-keepattributes *Annotation*

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile
