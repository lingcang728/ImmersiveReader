# Read-only desktop affiliation check. Never moves, activates or closes a window.
function Get-WindowDesktopName {
    param([Parameter(Mandatory)][IntPtr]$Window)
    if (-not ('ImmersiveReader.WindowDesktop' -as [type])) {
        Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
namespace ImmersiveReader {
  public static class WindowDesktop {
    [ComImport, Guid("A5CD92FF-29BE-454C-8D04-D82879FB3F1B"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IManager {
      [PreserveSig] int IsWindowOnCurrentVirtualDesktop(IntPtr h, out bool current);
      [PreserveSig] int GetWindowDesktopId(IntPtr h, out Guid id);
      [PreserveSig] int MoveWindowToDesktop(IntPtr h, ref Guid id);
    }
    public static Guid Read(IntPtr h) {
      var instance = Activator.CreateInstance(Type.GetTypeFromCLSID(new Guid("AA509086-5CA9-4C25-8F95-589D3C07B48A")));
      try { Guid id; Marshal.ThrowExceptionForHR(((IManager)instance).GetWindowDesktopId(h, out id)); return id; }
      finally { Marshal.ReleaseComObject(instance); }
    }
  }
}
"@
    }
    try { $desktopId = [ImmersiveReader.WindowDesktop]::Read($Window) }
    catch { throw "无法确认窗口所属桌面，暂停安装以保护 main 中的工作。" }
    $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops\Desktops\{' + $desktopId.ToString().ToUpperInvariant() + '}'
    return [string](Get-ItemProperty -LiteralPath $key -ErrorAction Stop).Name
}
function Assert-CodingDesktopProcesses {
    param([object[]]$Processes)
    foreach ($process in $Processes) {
        # A background helper may still belong to an app on main. Resolve its parent window first.
        $owner = $process
        $visited = [Collections.Generic.HashSet[int]]::new()
        while ($owner.MainWindowHandle -eq [IntPtr]::Zero) {
            if (-not $visited.Add([int]$owner.Id)) { throw '无法确认后台进程所属桌面，暂停安装。' }
            $record = Get-CimInstance Win32_Process -Filter "ProcessId = $($owner.Id)" -ErrorAction Stop
            if (-not $record -or -not $record.ParentProcessId) { throw '无法确认后台进程所属桌面，暂停安装。' }
            $owner = Get-Process -Id $record.ParentProcessId -ErrorAction Stop
        }
        $name = Get-WindowDesktopName -Window $owner.MainWindowHandle
        if ($name -ine 'coding') { throw "安装暂缓：PID $($process.Id) 的窗口属于 '$name'，只能关闭 coding 桌面的实例。" }
    }
}
