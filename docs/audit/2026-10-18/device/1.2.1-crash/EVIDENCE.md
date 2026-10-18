# 1.2.1 Android 启动崩溃真机复现（P0-1 实证）

- 设备：OPPO PLJ110（Find X9），Android 17 / SDK 37，无线调试序列号 `192.168.2.178:41031`
- 安装包：`release/ImmersiveReader_1.2.1_android-universal-release.apk`
  - 16,635,119 B，SHA-256 `F560BC74E620CFCC759BCE0AF095A56909D2A6BCCC5A739928C34BB681E1638C`（与 1.2.1 QA 报告一致）
  - `adb install -r -d` → **Success**；`pm path` 存在；`versionName=1.2.1`，`primaryCpuAbi=arm64-v8a`
- 冷启动：`am start -W -n com.lingcang.immersivereading/.MainActivity`
  - `Status: ok`，`LaunchState: COLD`，`Activity: com.lingcang.immersivereading/.MainActivity`
  - `TotalTime: 154 ms`，`WaitTime: 161 ms`（崩溃发生在首帧之前）
- 启动后 6 s：`pidof com.lingcang.immersivereading` 无输出；`dumpsys meminfo` → **No process found for: com.lingcang.immersivereading**
- logcat（过滤摘录 `crash-lines.txt`；原始 8277 行完整 log 已移除）：

```
10-18 19:10:32.198 E AndroidRuntime: FATAL EXCEPTION: main
10-18 19:10:32.198 E AndroidRuntime: Process: com.lingcang.immersivereading, PID: 26759
10-18 19:10:32.198 E AndroidRuntime: java.lang.RuntimeException: java.lang.reflect.InvocationTargetException
10-18 19:10:32.198 E AndroidRuntime: Caused by: java.lang.ClassNotFoundException: com.lingcang.immersivereading.SecretsPlugin
10-18 19:10:32.198 E AndroidRuntime: 	at java.lang.Class.classForName(Native Method)
10-18 19:10:32.198 E AndroidRuntime: 	at com.lingcang.immersivereading.WryActivity.getAppClass(SourceFile:5)
10-18 19:10:32.207 W ActivityTaskManager:   Force finishing activity com.lingcang.immersivereading/.MainActivity
10-18 19:10:32.248 I ActivityManager: Process com.lingcang.immersivereading (pid 26759) has died: vis  +99 TOP
```

- 观察到崩溃循环：系统/启动器重试拉起，PID 26759 → 27028 → 27201 均以同一
  `ClassNotFoundException: com.lingcang.immersivereading.SecretsPlugin` 崩溃，
  `Force finishing activity` 后进程死亡（`Displayed ... +154ms/+172ms` 仅为首帧前空窗）。
- 截图：`01-after-crash.png`（1,719,040 B，崩溃后回到桌面/系统界面）。
- 对应根因（性能审计 §2.1）：Tauri `register_android_plugin("com.lingcang.immersivereading","SecretsPlugin")`
  注册了 Kotlin 侧不存在的类；1.2.1 APK dex 中 `SecretsPlugin` 出现 0 次、
  `ContentReaderPlugin` 出现 6 次（静态复核见 `PERF-AUDIT.md`）。

结论：**1.2.1 Android 打开即闪退的根因在真机上得到完整实证**（ClassNotFoundException →
`Force finishing` → 进程死亡）。1.3.0 修复后的验收标准：同一设备冷启动无 FATAL/ANR、
进程存活、可进入书架并打开 md/epub。

## 本目录文件

- `apk.txt`（APK 哈希/大小）、`install.txt`（安装输出）、`cold-start.txt`（am start -W）
- `crash-lines.txt`（FATAL/AndroidRuntime 过滤摘录；原始完整 logcat 含设备系统日志，已按隐私要求移除）
- `meminfo-after-crash.txt`、`01-after-crash.png`
