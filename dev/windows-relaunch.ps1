param([Parameter(Mandatory=$true)][string]$Root, [string]$Binary = 'fastrock.exe')
# Run in an interactive Windows session. Root must contain the release-stamped
# executable and codex.exe built from dev/startup-fixture.go. No real Codex/Rally
# state is used. The user's Start menu shortcut is restored after the check.
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path $Root).Path
if (!(Test-Path (Join-Path $Root 'codex.exe'))) { throw 'Missing local Codex startup fixture' }
$target = Join-Path $Root 'localappdata\Programs\Fastrock\fastrock.exe'
if (Test-Path $target) { throw 'Use a fresh fixture root' }
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Fastrock.lnk'
$savedShortcut = Join-Path $Root 'original-shortcut.lnk'
$hadShortcut = Test-Path $shortcut
if ($hadShortcut) { Copy-Item $shortcut $savedShortcut }
Add-Type 'using System; using System.Runtime.InteropServices; public static class RelaunchWindow { [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l); }'
$env:LOCALAPPDATA = Join-Path $Root 'localappdata'
$env:FASTROCK_HOME = Join-Path $Root 'fastrock-home'
$env:CODEX_HOME = Join-Path $Root 'codex-home'
$env:PATH = "$Root;" + $env:PATH
Remove-Item Env:FASTROCK_AUTOMATION -ErrorAction SilentlyContinue
'ok' | Set-Content (Join-Path $Root 'mode.txt')
$results = @()
$child = $null
try {
    foreach ($mode in @('first-install','existing-install')) {
        $log = Join-Path $Root 'invocations.jsonl'
        Remove-Item $log -ErrorAction SilentlyContinue
        $started = Get-Date
        $launcher = Start-Process (Join-Path $Root $Binary) -WorkingDirectory $Root -PassThru
        $null = $launcher.Handle
        if (!$launcher.WaitForExit(10000) -or $launcher.ExitCode -ne 0) { throw "$mode launcher did not hand off successfully" }
        $until = (Get-Date).AddSeconds(15)
        do {
            $candidates = @(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $target })
            if ($candidates.Count -eq 1) {
                $child = Get-Process -Id $candidates[0].ProcessId
                $null = $child.Handle
                $child.Refresh()
                if ($child.MainWindowHandle -ne 0 -and (Test-Path $log)) {
                    $records = @(Get-Content $log | ForEach-Object { $_ | ConvertFrom-Json })
                    if (@($records | Where-Object { $_.args -contains 'app-server' }).Count) { break }
                }
            }
            if ((Get-Date) -gt $until) { throw "$mode did not show a window and start app-server" }
            Start-Sleep -Milliseconds 100
        } while ($true)
        if (@($records | Where-Object { !$_.visible -or $_.pixel -eq 0 -or $_.pixel -eq 0xFFFFFFFF }).Count) { throw "$mode started Codex before a painted window" }
        $visibleMilliseconds = ((Get-Date) - $started).TotalMilliseconds
        $cpu = $child.CPU
        Start-Sleep -Seconds 2
        $child.Refresh()
        $cpuSeconds = $child.CPU - $cpu
        if ($cpuSeconds -gt 0.5) { throw "$mode consumed $cpuSeconds CPU seconds while idle for two seconds" }
        [RelaunchWindow]::PostMessage($child.MainWindowHandle,0x10,[IntPtr]::Zero,[IntPtr]::Zero) | Out-Null
        if (!$child.WaitForExit(5000) -or $child.ExitCode -ne 0) { throw "$mode did not close cleanly" }
        $results += @{mode=$mode;status='pass';visibleMilliseconds=$visibleMilliseconds;idleCPUSeconds=$cpuSeconds;codexInvocations=$records.Count}
        $child.Dispose()
        $child = $null
        $launcher.Dispose()
        $launcher = $null
    }
    $results | ConvertTo-Json | Set-Content (Join-Path $Root 'relaunch-verification.json')
    $results | ConvertTo-Json
} finally {
    if ($child -and !$child.HasExited) { Stop-Process -Id $child.Id -Force }
    if ($launcher -and !$launcher.HasExited) { Stop-Process -Id $launcher.Id -Force }
    if ($hadShortcut) { Copy-Item $savedShortcut $shortcut -Force }
    else { Remove-Item $shortcut -ErrorAction SilentlyContinue }
}
