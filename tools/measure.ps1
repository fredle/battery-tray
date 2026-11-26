<#
.SYNOPSIS
  Measures what Battery Tray costs while it sits in the tray.

.DESCRIPTION
  Starts the given exe with --tray against a scratch LOCALAPPDATA (so the real
  settings, devices and history are never touched), either with the simulated
  source (BT_FAKE_DEVICES, the default) or with the real Windows sources, and
  records:

  - cold start: time until the hidden tray window exists and until the first
    tray icon is registered with the shell (Shell_NotifyIconGetRect);
  - after settling: private bytes (= commit charge), private working set,
    working set, threads, handles, GDI and USER objects, for every
    battery_tray process (the resident one and any UI process) and in total;
  - over the idle period: CPU time, context switches (every thread wake-up is
    one), write I/O operations, and file changes in the data folder;
  - tray click to flyout visible and to flyout painted (the flyout's
    background colour on screen), for several clicks in a row, and memory
    while the flyout is open and again once it has been closed for a while;
  - exe size, and the installed size Velopack would make of it (estimated:
    exe + its compressed copy in packages\ + Update.exe and the stub, from
    another installed Velopack app if one is found).

  Nothing else may be running as battery_tray.exe. The script kills every
  battery_tray process when it ends.

.EXAMPLE
  pwsh tools\measure.ps1 -Exe target\release\battery_tray.exe -Source fake -IdleSeconds 600
#>
param(
    [Parameter(Mandatory)] [string] $Exe,
    [ValidateSet('fake', 'real')] [string] $Source = 'fake',
    [int] $SettleSeconds = 30,
    [int] $IdleSeconds = 600,
    [int] $Clicks = 6,
    # How long to wait after the flyout closes before measuring again.
    [int] $AfterCloseSeconds = 45,
    [string] $Out
)

$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path

Add-Type -Language CSharp -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Probe {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindowW(string cls, string title);
    [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetGuiResources(IntPtr process, uint flags);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, System.Text.StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr h);
    [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr h, IntPtr dc);
    [DllImport("gdi32.dll")] public static extern uint GetPixel(IntPtr dc, int x, int y);
    [DllImport("kernel32.dll")] public static extern bool GetProcessIoCounters(IntPtr p, out IO_COUNTERS c);
    [DllImport("shell32.dll")] public static extern int Shell_NotifyIconGetRect(ref NOTIFYICONIDENTIFIER id, out RECT r);
    [DllImport("ntdll.dll")] public static extern int NtQuerySystemInformation(int cls, IntPtr buf, int len, out int ret);

    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct IO_COUNTERS {
        public ulong ReadOps, WriteOps, OtherOps, ReadBytes, WriteBytes, OtherBytes;
    }
    [StructLayout(LayoutKind.Sequential)] public struct NOTIFYICONIDENTIFIER {
        public uint cbSize; public IntPtr hWnd; public uint uID; public Guid guidItem;
    }

    public static bool IconRegistered(IntPtr window, uint id) {
        var nid = new NOTIFYICONIDENTIFIER { cbSize = (uint)Marshal.SizeOf(typeof(NOTIFYICONIDENTIFIER)), hWnd = window, uID = id };
        RECT r;
        return Shell_NotifyIconGetRect(ref nid, out r) == 0;
    }

    /// Visible top-level windows of these processes titled `title`.
    public static IntPtr VisibleWindow(uint[] pids, string title) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (Array.IndexOf(pids, pid) < 0 || !IsWindowVisible(h)) return true;
            var sb = new System.Text.StringBuilder(64);
            GetWindowTextW(h, sb, 64);
            if (sb.ToString() == title) { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    /// The screen colour at a point, as 0xRRGGBB.
    public static uint ScreenPixel(int x, int y) {
        IntPtr dc = GetDC(IntPtr.Zero);
        uint c = GetPixel(dc, x, y);
        ReleaseDC(IntPtr.Zero, dc);
        return ((c & 0xff) << 16) | (c & 0xff00) | ((c >> 16) & 0xff);
    }

    /// Context switches of each thread of a process, by thread id (x64
    /// layout of SYSTEM_PROCESS_INFORMATION / SYSTEM_THREAD_INFORMATION).
    public static System.Collections.Generic.Dictionary<long, long> ContextSwitches(int pid) {
        var counts = new System.Collections.Generic.Dictionary<long, long>();
        int len = 1 << 20;
        while (true) {
            IntPtr buf = Marshal.AllocHGlobal(len);
            try {
                int ret;
                int status = NtQuerySystemInformation(5, buf, len, out ret);
                if (status == unchecked((int)0xC0000004)) { len *= 2; continue; }
                if (status != 0) return counts;
                long offset = 0;
                while (true) {
                    IntPtr p = new IntPtr(buf.ToInt64() + offset);
                    uint next = (uint)Marshal.ReadInt32(p, 0);
                    uint threads = (uint)Marshal.ReadInt32(p, 4);
                    long id = Marshal.ReadIntPtr(p, 80).ToInt64();
                    if (id == pid) {
                        for (int i = 0; i < threads; i++) {
                            long tid = Marshal.ReadIntPtr(p, 256 + i * 80 + 48).ToInt64();
                            counts[tid] = (uint)Marshal.ReadInt32(p, 256 + i * 80 + 64);
                        }
                        return counts;
                    }
                    if (next == 0) return counts;
                    offset += next;
                }
            } finally { Marshal.FreeHGlobal(buf); }
        }
    }
}
'@

$CLASS = 'BatteryTrayHiddenWindow'
$WM_TRAY = 0x8000 + 1
$WM_LBUTTONUP = 0x0202
# The flyout's background, light and dark (theme.rs).
$BACKGROUNDS = @(0xF3F3F3, 0x202020)

# Only copies of this exe: another battery_tray.exe (say, a screenshot run
# from another folder) is left alone.
function Get-Tray { Get-Process battery_tray -ErrorAction SilentlyContinue | Where-Object Path -eq $Exe }

if (Get-Tray) { throw "$Exe is already running; quit it first." }
if ([Probe]::FindWindowW('BatteryTrayHiddenWindow', [NullString]::Value) -ne [IntPtr]::Zero) {
    throw 'another Battery Tray is in the tray; quit it first.'
}

$scratch = Join-Path ([IO.Path]::GetTempPath()) ("bt-measure-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Force $scratch | Out-Null
$data = Join-Path $scratch 'BatteryTray'
New-Item -ItemType Directory -Force $data | Out-Null
$fake = Join-Path $scratch 'fake.json'
@'
[
  {"name": "WH-1000XM4", "kind": "headphones", "level": 82},
  {"name": "MX Master 3", "kind": "mouse", "level": 55, "transport": "receiver", "charging": false},
  {"name": "AirPods Pro", "kind": "headphones", "precision": "tens", "parts": [
    {"kind": "left", "level": 80}, {"kind": "right", "level": 70}, {"kind": "case", "level": 40}]},
  {"name": "Xbox Wireless Controller", "kind": "gamepad", "precision": "coarse", "level": 60},
  {"name": "Old keyboard", "kind": "keyboard"},
  {"name": "Pixel Buds", "kind": "headphones", "connected": false}
]
'@ | Set-Content -Encoding utf8 $fake

function Snapshot {
    $all = @(Get-Tray)
    $rows = foreach ($p in $all) {
        $p.Refresh()
        $io = New-Object Probe+IO_COUNTERS
        [void][Probe]::GetProcessIoCounters($p.Handle, [ref]$io)
        [pscustomobject]@{
            Pid          = $p.Id
            Private      = $p.PrivateMemorySize64
            WorkingSet   = $p.WorkingSet64
            Threads      = $p.Threads.Count
            Handles      = $p.HandleCount
            Gdi          = [Probe]::GetGuiResources($p.Handle, 0)
            User         = [Probe]::GetGuiResources($p.Handle, 1)
            CpuMs        = $p.TotalProcessorTime.TotalMilliseconds
            Switches     = ([Probe]::ContextSwitches($p.Id).Values | Measure-Object -Sum).Sum
            PerThread    = [Probe]::ContextSwitches($p.Id)
            WriteOps     = $io.WriteOps
            WriteBytes   = $io.WriteBytes
        }
    }
    $rows
}

function Sum($rows, $field) { ($rows | Measure-Object -Property $field -Sum).Sum }

$result = [ordered]@{ exe = $Exe; source = $Source; exe_bytes = (Get-Item $Exe).Length }

# Installed size: Velopack puts the exe in current\, a compressed copy in
# packages\, and Update.exe plus a stub launcher next to them.
$zip = Join-Path $scratch 'exe.zip'
Compress-Archive -Path $Exe -DestinationPath $zip -CompressionLevel Optimal
$velopack = Get-ChildItem $env:LOCALAPPDATA -Filter Update.exe -Recurse -Depth 1 -ErrorAction SilentlyContinue | Select-Object -First 1
if ($velopack) {
    $stub = Get-ChildItem $velopack.DirectoryName -Filter *.exe | Where-Object Name -ne 'Update.exe' | Select-Object -First 1
    $result.installed_bytes_estimate = $result.exe_bytes + (Get-Item $zip).Length + $velopack.Length + ($stub ? $stub.Length : 0)
}

# Start it.
$psi = New-Object Diagnostics.ProcessStartInfo $Exe
$psi.ArgumentList.Add('--tray')
$psi.UseShellExecute = $false
$psi.Environment['LOCALAPPDATA'] = $scratch
$psi.Environment['BT_QUIET'] = '1'
if ($Source -eq 'fake') { $psi.Environment['BT_FAKE_DEVICES'] = $fake } else { $psi.Environment.Remove('BT_FAKE_DEVICES') | Out-Null }

$clock = [Diagnostics.Stopwatch]::StartNew()
$proc = [Diagnostics.Process]::Start($psi)
$window = [IntPtr]::Zero
while ($clock.ElapsedMilliseconds -lt 20000) {
    $window = [Probe]::FindWindowW($CLASS, [NullString]::Value)
    if ($window -ne [IntPtr]::Zero) { break }
    Start-Sleep -Milliseconds 2
}
$result.start_to_tray_window_ms = $clock.ElapsedMilliseconds
while ($clock.ElapsedMilliseconds -lt 20000 -and -not [Probe]::IconRegistered($window, 1)) { Start-Sleep -Milliseconds 2 }
$result.start_to_tray_icon_ms = $clock.ElapsedMilliseconds
if ($window -eq [IntPtr]::Zero) { throw 'the tray window never appeared' }

try {
    Start-Sleep -Seconds $SettleSeconds
    $settled = Snapshot
    $result.settled = $settled | Select-Object * -ExcludeProperty PerThread

    # Idle: CPU, wake-ups and writes.
    $changes = [System.Collections.Concurrent.ConcurrentBag[string]]::new()
    $watcher = New-Object IO.FileSystemWatcher $data
    $watcher.IncludeSubdirectories = $true
    $watcher.NotifyFilter = [IO.NotifyFilters]'FileName, LastWrite, Size'
    $handler = { $Event.MessageData.Add("$($EventArgs.ChangeType) $($EventArgs.Name) $(Get-Date -Format HH:mm:ss)") }
    $subs = foreach ($name in 'Changed', 'Created', 'Renamed', 'Deleted') {
        Register-ObjectEvent $watcher $name -MessageData $changes -Action $handler
    }
    $watcher.EnableRaisingEvents = $true
    $before = Snapshot
    Start-Sleep -Seconds $IdleSeconds
    $after = Snapshot
    $watcher.EnableRaisingEvents = $false
    $subs | ForEach-Object { Unregister-Event -SourceIdentifier $_.Name }
    $result.idle_seconds = $IdleSeconds
    $result.idle_processes_before = @($before).Count
    $result.idle_processes_after = @($after).Count
    $result.idle_cpu_ms = [math]::Round((Sum $after CpuMs) - (Sum $before CpuMs), 1)
    # Per thread, so threads that come and go do not cancel out. A thread
    # that ended during the period takes its count with it (an undercount).
    $switches = 0
    foreach ($a in @($after)) {
        $b = @($before) | Where-Object Pid -eq $a.Pid
        foreach ($tid in $a.PerThread.Keys) {
            $was = if ($b -and $b.PerThread.ContainsKey($tid)) { $b.PerThread[$tid] } else { 0 }
            $switches += $a.PerThread[$tid] - $was
        }
    }
    $result.idle_context_switches = $switches
    $result.idle_write_ops = (Sum $after WriteOps) - (Sum $before WriteOps)
    $result.idle_file_changes = $changes.Count
    $result.idle_file_change_list = @($changes | Sort-Object | Select-Object -First 40)
    $result.idle_end = $after | Select-Object * -ExcludeProperty PerThread

    # Tray clicks: open, wait, close, several times.
    $clicksOut = @()
    for ($i = 0; $i -lt $Clicks; $i++) {
        $pids = [uint32[]]@(Get-Tray | ForEach-Object Id)
        $clock.Restart()
        [void][Probe]::PostMessageW($window, $WM_TRAY, [IntPtr]1, [IntPtr]$WM_LBUTTONUP)
        $flyout = [IntPtr]::Zero; $visibleMs = $null; $paintedMs = $null
        while ($clock.ElapsedMilliseconds -lt 10000) {
            if ($flyout -eq [IntPtr]::Zero) {
                $pids = [uint32[]]@(Get-Tray | ForEach-Object Id)
                $flyout = [Probe]::VisibleWindow($pids, 'Battery Tray')
                if ($flyout -ne [IntPtr]::Zero) { $visibleMs = $clock.ElapsedMilliseconds }
            }
            if ($flyout -ne [IntPtr]::Zero) {
                $r = New-Object Probe+RECT
                [void][Probe]::GetWindowRect($flyout, [ref]$r)
                $pixel = [Probe]::ScreenPixel($r.Left + 4, [int](($r.Top + $r.Bottom) / 2))
                if ($BACKGROUNDS -contains $pixel) { $paintedMs = $clock.ElapsedMilliseconds; break }
            }
            Start-Sleep -Milliseconds 1
        }
        Start-Sleep -Milliseconds 1500
        $open = Snapshot
        $clicksOut += [pscustomobject]@{
            click = $i + 1; visible_ms = $visibleMs; painted_ms = $paintedMs
            processes = @($open).Count; private_total = (Sum $open Private)
        }
        [void][Probe]::PostMessageW($window, $WM_TRAY, [IntPtr]1, [IntPtr]$WM_LBUTTONUP)
        Start-Sleep -Milliseconds 1200
        # Every other pair of clicks waits long enough for an idle UI process to go.
        if ($i % 2 -eq 1) { Start-Sleep -Seconds $AfterCloseSeconds }
    }
    $result.clicks = $clicksOut
    Start-Sleep -Seconds $AfterCloseSeconds
    $closed = Snapshot
    $result.after_close = $closed | Select-Object * -ExcludeProperty PerThread
    $result.alive_at_end = [bool](Get-Process -Id $proc.Id -ErrorAction SilentlyContinue)
}
finally {
    Get-Tray | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 500
    Remove-Item -Recurse -Force $scratch -ErrorAction SilentlyContinue
}

$json = $result | ConvertTo-Json -Depth 6
if ($Out) { $json | Set-Content -Encoding utf8 $Out }
$json
