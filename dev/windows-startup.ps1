param([string]$Root, [string]$Binary = 'fastrock-startup.exe', [string]$Baseline = '')
$ErrorActionPreference = 'Stop'
Add-Type 'using System; using System.Runtime.InteropServices; public static class StartupWindow { [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l); [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h); }'
New-Item -ItemType Directory -Force "$Root\codex-home" | Out-Null
$env:CODEX_HOME = "$Root\codex-home"
$env:PATH = "$Root;" + $env:PATH
$results = @()
foreach ($mode in @('blocked','incompatible','slow','missing','baseline')) {
    $elapsed = $null
    if ($mode -eq 'baseline' -and -not $Baseline) { continue }
    $env:FASTROCK_HOME = "$Root\home-$mode"
    New-Item -ItemType Directory -Force $env:FASTROCK_HOME | Out-Null
    @{theme='dark';fontSize=13;sidebar=$true;info=$false} | ConvertTo-Json | Set-Content "$env:FASTROCK_HOME\settings.json"
    Remove-Item "$Root\invocations.jsonl", "$Root\retry.ready", "$Root\retry.done" -ErrorAction SilentlyContinue
    $mode | Set-Content "$Root\mode.txt"
    $steps = @(@{action='wait';milliseconds=1200})
    if ($mode -in @('blocked','incompatible')) {
        $steps += @(@{action='assert_codex';value='unavailable'},@{action='snapshot';path="$Root\startup-$mode.png"},@{action='action';value='toggle-sidebar'},@{action='signal';path="$Root\retry.ready"},@{action='wait_file';path="$Root\retry.done"},@{action='retry_codex'},@{action='wait';milliseconds=1800},@{action='assert_codex';value='connected'},@{action='quit'})
    } elseif ($mode -eq 'missing') {
        $steps += @(@{action='assert_codex';value='unavailable'},@{action='snapshot';path="$Root\startup-missing.png"},@{action='action';value='toggle-sidebar'},@{action='quit'})
    } elseif ($mode -eq 'slow') {
        $steps = @(@{action='wait';milliseconds=120000})
    } else { $steps += @{action='quit'} }
    $job = "$Root\$mode.json"
    $steps | ConvertTo-Json | Set-Content $job
    $env:FASTROCK_AUTOMATION = $job
    $selected = $Binary
    if ($mode -eq 'baseline') { $selected = $Baseline; 'ok' | Set-Content "$Root\mode.txt" }
    $priorPath = $env:PATH
    if ($mode -eq 'missing') { $env:PATH = "$Root\empty-path" }
    $process = Start-Process "$Root\$selected" -WorkingDirectory $Root -PassThru -RedirectStandardOutput "$Root\$mode.stdout" -RedirectStandardError "$Root\$mode.stderr"
    # Windows PowerShell must retain the handle before a short-lived process exits.
    $null = $process.Handle
    $env:PATH = $priorPath
    try {
        for ($attempt = 0; $attempt -lt 20; $attempt++) {
            $process.Refresh()
            if ($process.MainWindowHandle -ne 0) {
                [StartupWindow]::SetForegroundWindow($process.MainWindowHandle) | Out-Null
                break
            }
            Start-Sleep -Milliseconds 50
        }
        if ($mode -in @('blocked','incompatible')) {
            $until = (Get-Date).AddSeconds(15)
            while (!(Test-Path "$Root\retry.ready")) {
                if ((Get-Date) -gt $until -or $process.HasExited) { throw "$mode did not reach retry" }
                Start-Sleep -Milliseconds 100
            }
            'ok' | Set-Content "$Root\mode.txt"
            '' | Set-Content "$Root\retry.done"
        }
        if ($mode -eq 'slow') {
            Start-Sleep -Milliseconds 1200
            $process.Refresh()
            if ($process.MainWindowHandle -eq 0) { throw 'slow probe hid the GUI' }
            $started = Get-Date
            [StartupWindow]::PostMessage($process.MainWindowHandle,0x10,[IntPtr]::Zero,[IntPtr]::Zero) | Out-Null
            if (!$process.WaitForExit(5000)) { throw 'closing during slow probe did not cancel startup' }
            $elapsed = ((Get-Date)-$started).TotalMilliseconds
        } elseif (!$process.WaitForExit(30000)) { throw "$mode did not exit" }
        $process.Refresh()
        if ($process.ExitCode -ne 0) { throw "$mode failed with exit $($process.ExitCode)" }
        $records = @()
        if (Test-Path "$Root\invocations.jsonl") { $records = @(Get-Content "$Root\invocations.jsonl" | ForEach-Object { $_ | ConvertFrom-Json }) }
        $records | ConvertTo-Json -Depth 5 | Set-Content "$Root\invocations-$mode.json"
        if ($mode -ne 'baseline' -and $mode -ne 'missing') {
            if (!$records.Count -or @($records | Where-Object { !$_.visible -or $_.pixel -eq 0 -or $_.pixel -eq 0xFFFFFFFF }).Count) { throw "$mode invoked Codex before a painted GUI" }
        }
        if ($mode -eq 'baseline' -and !@($records | Where-Object { !$_.visible }).Count) { throw 'baseline did not reproduce headless startup' }
        if ($mode -eq 'missing' -and $records.Count) { throw 'missing CLI unexpectedly ran a process' }
        if ($mode -ne 'slow') {
            $log = Get-Content "$job.result" -Raw
            if ($log.Contains('FAIL:')) { throw $log }
        }
        if ($mode -in @('blocked','incompatible','missing')) {
            $preferences = Get-Content "$env:FASTROCK_HOME\settings.json" -Raw | ConvertFrom-Json
            if ($preferences.sidebar) { throw "$mode lost settings changed while Codex was unavailable" }
        }
        $results += @{mode=$mode;status='pass';invocations=$records.Count;closeMilliseconds=$elapsed}
    } finally {
        if (!$process.HasExited) { Stop-Process -Id $process.Id -Force }
    }
}
$results | ConvertTo-Json | Set-Content "$Root\startup-verification.json"
$results | ConvertTo-Json
