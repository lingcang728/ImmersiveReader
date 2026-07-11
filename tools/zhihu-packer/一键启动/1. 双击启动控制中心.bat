@echo off
chcp 936 >nul
title Zhihu Packer 控制中心
cd /d "%~dp0.."

echo ==========================================
echo   正在启动 Zhihu Packer Web 控制中心...
echo ==========================================
echo   - 启动后会自动打开浏览器： http://localhost:3000
echo   - 停止服务：直接关闭本窗口即可
echo ==========================================
echo.

rem 首次使用或代码更新后若缺少依赖，自动安装（tsx 直接运行源码，无需额外编译）
if not exist "node_modules" (
    echo 检测到尚未安装依赖，正在执行 npm install，请稍候...
    call npm install
    echo.
)

rem 延迟几秒后自动打开浏览器（此时 Web 服务已就绪），不阻塞服务本身
start "" cmd /c "ping -n 5 127.0.0.1 >nul & start http://localhost:3000"

echo 正在启动 Web 服务（保持本窗口开启；关闭窗口即停止服务）...
echo.
call npm run web

echo.
echo 服务已停止。按任意键关闭本窗口。
pause >nul
