param([string]$Root = $PSScriptRoot, [string]$Binary = 'fastrock.exe')
$ErrorActionPreference = 'Stop'
Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -like "$Root\*" -and ($_.Name -like 'fastrock*.exe' -or $_.Name -like 'mock-*.exe') } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
Start-Sleep -Milliseconds 1200
Remove-Item "$Root\smoke.json.result", "$Root\0*.png" -Force -ErrorAction SilentlyContinue
if (Test-Path "$Root\fastrock-home\session.json") { Remove-Item "$Root\fastrock-home\session.json" -Force }
New-Item -ItemType Directory -Force "$Root\codex-home", "$Root\fastrock-home" | Out-Null
$env:PATH = "$Root;" + $env:PATH
$env:CODEX_HOME = "$Root\codex-home"
$env:FASTROCK_HOME = "$Root\fastrock-home"
$env:FASTROCK_RALLY_TOKEN = 'mock-token'
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
@{
    theme = 'dark'; fontSize = 13; sidebar = $true; info = $true; enterSends = $true
    rallyEndpoint = 'http://127.0.0.1:18081'; workingDirectory = $Root
} | ConvertTo-Json | Set-Content "$Root\fastrock-home\settings.json"
$steps = @(
    @{action='wait'; milliseconds=3500},
    @{action='rally'; value='teamboard'},
    @{action='wait'; milliseconds=3500},
    @{action='filter'; value='US1001'},
    @{action='assert_count'; value='1'},
    @{action='filter'; value=''},
    @{action='snapshot'; path="$Root\01-board-dark.png"},
    @{action='theme'; value='light'},
    @{action='wait'; milliseconds=500},
    @{action='snapshot'; path="$Root\02-board-light.png"},
    @{action='theme'; value='dark'},
    @{action='item'; value='US1001'},
    @{action='wait'; milliseconds=1500},
    @{action='snapshot'; path="$Root\03-detail.png"},
    @{action='description'; value='Updated rich text from the native editor'},
    @{action='format_description'},
    @{action='assert_html'; value='<strong>Updated rich text from the native editor</strong>'},
    @{action='save_item'},
    @{action='wait'; milliseconds=1200},
    @{action='back'},
    @{action='item'; value='US1001'},
    @{action='wait'; milliseconds=1200},
    @{action='assert_html'; value='<strong>Updated rich text from the native editor</strong>'},
    @{action='detail_tab'; value='Tasks'},
    @{action='wait'; milliseconds=800},
    @{action='open_first_child'},
    @{action='wait'; milliseconds=1000},
    @{action='snapshot'; path="$Root\03-task-detail.png"},
    @{action='new_chat'; value=$Root},
    @{action='wait'; milliseconds=2000},
    @{action='send'; value='hello'},
    @{action='wait'; milliseconds=5000},
    @{action='assert_chat'; value='Fastrock'},
    @{action='snapshot'; path="$Root\04-conversation.png"},
    @{action='rally'; value='teamboard'},
    @{action='wait'; milliseconds=500},
    @{action='assistant'},
    @{action='ask'; value='Show blocked stories and group by owner'},
    @{action='wait'; milliseconds=7000},
    @{action='assert_assistant'; value='blocked stories'},
    @{action='snapshot'; path="$Root\05-assistant.png"},
    @{action='wait'; milliseconds=1000},
    @{action='quit'}
)
$steps | ConvertTo-Json | Set-Content "$Root\smoke.json"
$env:FASTROCK_AUTOMATION = "$Root\smoke.json"
Start-Process "$Root\mock-rally.exe" -WindowStyle Hidden -RedirectStandardOutput "$Root\rally.log" -RedirectStandardError "$Root\rally.err" -PassThru | Select-Object Id,ProcessName
Start-Process "$Root\mock-model.exe" -WindowStyle Hidden -RedirectStandardOutput "$Root\model.log" -RedirectStandardError "$Root\model.err" -PassThru | Select-Object Id,ProcessName
Start-Process "$Root\$Binary" -WorkingDirectory $Root -RedirectStandardOutput "$Root\ui.log" -RedirectStandardError "$Root\ui.err" -PassThru | Select-Object Id,ProcessName
