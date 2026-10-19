import java.io.File
import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

// Release signing is opt-in: drop a keystore.properties (storeFile /
// storePassword / keyAlias / keyPassword) at the repo root or next to
// src-tauri and release builds pick it up. A missing file must not break
// configuration — release then keeps the default (debug) signing.
val keystorePropertiesFile = listOf(
    File(rootDir, "../../../../../keystore.properties"), // ImmersiveReader/keystore.properties
    File(rootDir, "../../keystore.properties"), // src-tauri/keystore.properties
).firstOrNull { it.isFile }

val keystoreProperties = Properties().apply {
    keystorePropertiesFile?.inputStream()?.use { load(it) }
}

// -PqaBuild=true produces a side-by-side QA package.
val qaBuild = project.findProperty("qaBuild")?.toString().toBoolean()

android {
    compileSdk = 36
    namespace = "com.lingcang.immersivereading"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "com.lingcang.immersivereading"
        buildConfigField("boolean", "QA_BUILD", qaBuild.toString())
        minSdk = 26
        targetSdk = 36
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
        if (qaBuild) {
            applicationIdSuffix = ".qa"
            versionNameSuffix = "-qa"
        }
    }
    if (keystorePropertiesFile != null) {
        signingConfigs {
            create("release") {
                storeFile = keystoreProperties.getProperty("storeFile")?.let { path ->
                    val storePath = File(path)
                    if (storePath.isAbsolute) storePath else File(keystorePropertiesFile.parentFile, path)
                }
                storePassword = keystoreProperties.getProperty("storePassword")
                keyAlias = keystoreProperties.getProperty("keyAlias")
                keyPassword = keystoreProperties.getProperty("keyPassword")
            }
        }
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            isMinifyEnabled = true
            // Drop resources that R8 proves unreachable (mipmap densities,
            // unused XML, etc.). Requires minifyEnabled = true above.
            isShrinkResources = true
            // Native debug symbols for the Rust `immersivereading` .so files.
            // Applies to App-Bundle style packaging (Play symbol upload); for
            // the standalone APK the NDK strip level still wins and the APK
            // stays small. "FULL" keeps the symbols usable when an AAB is
            // eventually published.
            ndk {
                debugSymbolLevel = "FULL"
            }
            proguardFiles(
                *fileTree(".") { include("**/*.pro") }
                    .plus(getDefaultProguardFile("proguard-android-optimize.txt"))
                    .toList().toTypedArray()
            )
            if (keystorePropertiesFile != null) {
                signingConfig = signingConfigs.getByName("release")
            } else {
                logger.warn(
                    "keystore.properties not found at the repo root or src-tauri; " +
                        "the release build keeps the default signing configuration",
                )
            }
        }
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
    buildFeatures {
        buildConfig = true
    }
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    // Cold-start splash: Theme.SplashScreen + installSplashScreen(). 1.0.1 is
    // the latest stable of the line (Java-only, kotlin-stdlib 1.6.21) so it
    // stays compatible with the project's Kotlin 1.9.25 toolchain; 1.2.x
    // pulls kotlin-stdlib 2.0.21 into the app classpath.
    implementation("androidx.core:core-splashscreen:1.0.1")
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("androidx.documentfile:documentfile:1.1.0")
    implementation("com.google.android.material:material:1.12.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = "tauri.build.gradle.kts")
