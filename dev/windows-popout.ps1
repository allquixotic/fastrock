param([string]$Root = $PSScriptRoot)
$ErrorActionPreference='Stop'
Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -like "$Root\*" -and ($_.Name -eq 'fastrock.exe' -or $_.Name -like 'mock-*.exe' -or $_.Name -eq 'codex.exe') } | ForEach-Object {Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue}
Start-Sleep -Milliseconds 1200
New-Item -ItemType Directory -Force "$Root\fastrock-home","$Root\codex-home" | Out-Null
Remove-Item "$Root\popout.json.result","$Root\fastrock-home\session*.json" -Force -ErrorAction SilentlyContinue
$env:PATH="$Root;"+$env:PATH
$env:CODEX_HOME="$Root\codex-home"
$env:FASTROCK_HOME="$Root\fastrock-home"
$env:FASTROCK_RALLY_TOKEN='mock-token'
@'
model = "gpt-6.1-sol"
model_provider = "mock"
approval_policy = "never"
sandbox_mode = "read-only"
[model_providers.mock]
name = "Mock Responses API"
base_url = "http://127.0.0.1:18080/v1"
wire_api = "responses"
requires_openai_auth = false
request_max_retries = 0
stream_max_retries = 0
'@ | Set-Content "$Root\codex-home\config.toml"
@{theme='dark';fontSize=13;sidebar=$true;info=$true;enterSends=$true;rallyEndpoint='http://127.0.0.1:18081';workingDirectory=$Root} | ConvertTo-Json | Set-Content "$Root\fastrock-home\settings.json"
$steps=@(@{action='wait';milliseconds=4000},@{action='rally';value='teamboard'},@{action='wait';milliseconds=1800})
for($i=0;$i -lt 20;$i++){$steps+=@(@{action='load_next'},@{action='wait';milliseconds=150})}
$steps+=@(
 @{action='assert_window_bound'},
 @{action='refresh'},@{action='wait';milliseconds=1000},
 @{action='filter';value='US1001'},@{action='wait';milliseconds=700},
 @{action='item';value='US1001'},@{action='wait';milliseconds=800},
 @{action='description';value='Unsaved pop-out draft'},@{action='format_description'},
 @{action='popout'},@{action='wait';milliseconds=5000},@{action='assert_tab_count';value='1'},
 @{action='new_chat';value=$Root},@{action='wait';milliseconds=2000},
 @{action='send';value='hello'},@{action='wait';milliseconds=1500},@{action='draft';value='Unsent chat draft'},
 @{action='restart'},@{action='wait';milliseconds=4000},@{action='assert_draft';value='Unsent chat draft'},
 @{action='popout'},@{action='wait';milliseconds=7000},@{action='assert_tab_count';value='1'},
 @{action='assert_memory';value='480'},@{action='draw_stats'},@{action='quit'}
)
$steps | ConvertTo-Json | Set-Content "$Root\popout.json"
$env:FASTROCK_AUTOMATION="$Root\popout.json"
Start-Process "$Root\mock-rally.exe" -ArgumentList '--stories','10000' -WindowStyle Hidden -RedirectStandardOutput "$Root\rally.log" -RedirectStandardError "$Root\rally.err"
Start-Process "$Root\mock-model.exe" -WindowStyle Hidden -RedirectStandardOutput "$Root\model.log" -RedirectStandardError "$Root\model.err"
Start-Process "$Root\fastrock.exe" -WorkingDirectory $Root -RedirectStandardOutput "$Root\ui.log" -RedirectStandardError "$Root\ui.err"
