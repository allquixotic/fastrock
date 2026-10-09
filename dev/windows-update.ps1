param([Parameter(Mandatory=$true)][string]$Root, [Parameter(Mandatory=$true)][string]$Version)
# Root contains the new fastrock.exe, previous-fastrock.exe from a signed release
# and codex.exe built from startup-fixture.go. Run only on interactive Windows.
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path $Root).Path
$Version = $Version.TrimStart('v')
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw 'Specify a release version' }
foreach ($file in @('fastrock.exe','previous-fastrock.exe','codex.exe')) {
    if (!(Test-Path (Join-Path $Root $file))) { throw "Missing fixture $file" }
}
$target = Join-Path $Root 'localappdata\Programs\Fastrock\fastrock.exe'
if (Test-Path $target) { throw 'Use a fresh fixture root' }
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Fastrock.lnk'
$savedShortcut = Join-Path $Root 'original-shortcut.lnk'
$hadShortcut = Test-Path $shortcut
if ($hadShortcut) { Copy-Item $shortcut $savedShortcut }
Add-Type 'using System; using System.Runtime.InteropServices; public static class UpgradeWindow { [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l); }'
$env:LOCALAPPDATA = Join-Path $Root 'localappdata'
$env:FASTROCK_HOME = Join-Path $Root 'fastrock-home'
$env:CODEX_HOME = Join-Path $Root 'codex-home'
$env:PATH = "$Root;" + $env:PATH
# Keep the fixture's automatic release check off the public network. The update
# job below uses the downloaded, independently verified release executable.
$env:HTTPS_PROXY = 'http://127.0.0.1:9'
Remove-Item Env:FASTROCK_AUTOMATION -ErrorAction SilentlyContinue
'ok' | Set-Content (Join-Path $Root 'mode.txt')
$log = Join-Path $Root 'invocations.jsonl'
function Wait-ReadyWindow($process) {
    $until = (Get-Date).AddSeconds(20)
    do {
        $process.Refresh()
        if ($process.HasExited) { throw 'App exited before connecting' }
        if ($process.MainWindowHandle -ne 0 -and (Test-Path $log)) {
            $records = @(Get-Content $log | ForEach-Object { $_ | ConvertFrom-Json } | Where-Object { $_.parent -eq $process.Id })
            if (@($records | Where-Object { $_.args -contains 'app-server' }).Count) {
                if (@($records | Where-Object { !$_.visible -or $_.pixel -eq 0 -or $_.pixel -eq 0xFFFFFFFF }).Count) { throw 'Codex ran before a painted window' }
                return
            }
        }
        if ((Get-Date) -gt $until) { throw 'App did not show a window and start app-server' }
        Start-Sleep -Milliseconds 100
    } while ($true)
}
$old = $null
$helper = $null
$child = $null
try {
    New-Item -ItemType Directory -Force (Split-Path $target) | Out-Null
    Copy-Item (Join-Path $Root 'previous-fastrock.exe') $target
    $old = Start-Process $target -PassThru
    $null = $old.Handle
    Wait-ReadyWindow $old
    $stage = Join-Path $Root 'updates\release-fixture'
    $payload = Join-Path $stage 'payload\fastrock.exe'
    New-Item -ItemType Directory -Force (Split-Path $payload) | Out-Null
    Copy-Item (Join-Path $Root 'fastrock.exe') $payload
    $job = Join-Path $stage 'job.json'
    # Go's JSON decoder does not accept the BOM emitted by Windows PowerShell.
    $jobContent = @{ParentPID=$old.Id;Target=$target;Staged=$payload;Stage=$stage;Version=$Version;GOOS='windows'} | ConvertTo-Json
    [IO.File]::WriteAllText($job, $jobContent, (New-Object System.Text.UTF8Encoding($false)))
    $helper = Start-Process (Join-Path $Root 'previous-fastrock.exe') -ArgumentList @('--apply-update', ('"{0}"' -f $job)) -PassThru -RedirectStandardError (Join-Path $Root 'helper.stderr')
    $null = $helper.Handle
    if ($helper.WaitForExit(300)) { throw 'Previous updater did not wait for the live app' }
    [UpgradeWindow]::PostMessage($old.MainWindowHandle,0x10,[IntPtr]::Zero,[IntPtr]::Zero) | Out-Null
    if (!$old.WaitForExit(5000) -or $old.ExitCode -ne 0) { throw 'Previous app did not close cleanly' }
    $old.Dispose()
    $old = $null
    if (!$helper.WaitForExit(30000) -or $helper.ExitCode -ne 0) { throw "Previous updater failed: $(Get-Content (Join-Path $Root 'helper.stderr') -Raw)" }
    $helper.Dispose()
    $helper = $null
    $expected = (Get-FileHash (Join-Path $Root 'fastrock.exe') -Algorithm SHA256).Hash
    if ((Get-FileHash $target -Algorithm SHA256).Hash -ne $expected) { throw 'Updater did not replace the installed executable' }
    $candidates = @(Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $target })
    if ($candidates.Count -ne 1) { throw 'Updater did not restart exactly one app' }
    $child = Get-Process -Id $candidates[0].ProcessId
    $null = $child.Handle
    Wait-ReadyWindow $child
    $cpu = $child.CPU
    Start-Sleep -Seconds 2
    $child.Refresh()
    $cpuSeconds = $child.CPU - $cpu
    if ($cpuSeconds -gt 0.5) { throw 'Updated app spins while idle' }
    [UpgradeWindow]::PostMessage($child.MainWindowHandle,0x10,[IntPtr]::Zero,[IntPtr]::Zero) | Out-Null
    if (!$child.WaitForExit(5000) -or $child.ExitCode -ne 0) { throw 'Updated app did not close cleanly' }
    $child.Dispose()
    $child = $null
    $report = @{status='pass';version=$Version;previousUpdaterWaited=$true;replacementSHA256=$expected.ToLower();paintedBeforeCodex=$true;idleCPUSeconds=$cpuSeconds;cleanShutdown=$true}
    $report | ConvertTo-Json | Set-Content (Join-Path $Root 'update-verification.json')
    $report | ConvertTo-Json
} finally {
    foreach ($process in @($old,$helper,$child)) {
        if ($process -and !$process.HasExited) { Stop-Process -Id $process.Id -Force }
    }
    # Also cover a newly restarted child if a check failed before obtaining its handle.
    Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -eq $target } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    if ($hadShortcut) { Copy-Item $savedShortcut $shortcut -Force }
    else { Remove-Item $shortcut -ErrorAction SilentlyContinue }
}
