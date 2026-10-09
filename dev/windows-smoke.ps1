param([string]$Root = $PSScriptRoot, [string]$Binary = 'fastrock.exe', [switch]$Layout, [switch]$Rows)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition 'using System.Runtime.InteropServices; public static class SmokeCursor { [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y); }'
[SmokeCursor]::SetCursorPos(1800, 900) | Out-Null
Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -like "$Root\*" -and ($_.Name -like 'fastrock*.exe' -or $_.Name -like 'mock-*.exe') } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
Start-Sleep -Milliseconds 1200
Remove-Item "$Root\smoke.json.result", "$Root\0*.png" -Force -ErrorAction SilentlyContinue
Remove-Item "$Root\fastrock-home\session*.json*" -Force -ErrorAction SilentlyContinue
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
    @{action='rally_navigation'; value='hidden'},
    @{action='wait'; milliseconds=500},
    @{action='snapshot'; path="$Root\02-board-collapsed.png"},
    @{action='rally_navigation'; value='shown'},
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
    @{action='action'; value='new-tab'},
    @{action='wait'; milliseconds=500},
    @{action='snapshot'; path="$Root\06-new-tab.png"},
    @{action='wait'; milliseconds=1000},
    @{action='quit'}
)
$steps | ConvertTo-Json | Set-Content "$Root\smoke.json"
if ($Layout) {
    Remove-Item "$Root\layout-*.ready", "$Root\layout-*.done" -Force -ErrorAction SilentlyContinue
    $steps = @(@{action='wait'; milliseconds=3500}, @{action='rally'; value='teamboard'}, @{action='rally_navigation'; value='shown'}, @{action='wait'; milliseconds=2000})
    foreach ($check in @(@('hide', 'assert_rally_row', 'hidden'), @('show', 'assert_rally_row', 'shown'))) {
        $steps += @(@{action='signal'; path="$Root\layout-$($check[0]).ready"}, @{action='wait_file'; path="$Root\layout-$($check[0]).done"}, @{action='wait'; milliseconds=1200}, @{action=$check[1]; value='sections'; path=$check[2]})
    }
    foreach ($page in @('backlog','userstories','teamplan','workviews','iterationstatus','tasks','defects','testcases','portfolioitemstreegrid','timeline','reports')) {
        $steps += @{action='rally'; value=$page}
    }
    $steps += @(@{action='action'; value='new-tab'}, @{action='wait'; milliseconds=300}, @{action='rally'; value='teamboard'}, @{action='wait'; milliseconds=800}, @{action='assert_tab_scroll'; value='start'})
    foreach ($check in @(@('right', 'middle'), @('last', 'end'), @('left', 'middle'), @('first', 'start'))) {
        $steps += @(@{action='signal'; path="$Root\layout-$($check[0]).ready"}, @{action='wait_file'; path="$Root\layout-$($check[0]).done"}, @{action='wait'; milliseconds=1200}, @{action='assert_tab_scroll'; value=$check[1]})
        $steps += @{action='snapshot'; path="$Root\07-tabs-$($check[0]).png"}
    }
    $steps += @{action='quit'}
    $steps | ConvertTo-Json | Set-Content "$Root\smoke.json"
}
if ($Rows) {
    $steps = @(
        @{action='wait'; milliseconds=3500},
        @{action='rally'; value='teamboard'},
        @{action='wait'; milliseconds=2000},
        @{action='theme'; value='light'},
        @{action='wait'; milliseconds=600},
        @{action='snapshot'; path="$Root\08-rows-expanded.png"},
        @{action='rally_rows'; value='sections,pages,project,freshness,saved-view,timeboxes,view-actions,modes,density,search'},
        @{action='wait'; milliseconds=600},
        @{action='snapshot'; path="$Root\09-rows-subset.png"},
        @{action='rally_rows'; value='all'},
        @{action='wait'; milliseconds=600},
        @{action='snapshot'; path="$Root\10-rows-hidden.png"},
        @{action='theme'; value='dark'},
        @{action='wait'; milliseconds=600},
        @{action='snapshot'; path="$Root\11-rows-hidden-dark.png"},
        @{action='rally_rows'; value=''},
        @{action='wait'; milliseconds=600},
        @{action='snapshot'; path="$Root\12-rows-restored.png"},
        @{action='quit'}
    )
    $steps | ConvertTo-Json | Set-Content "$Root\smoke.json"
}
$env:FASTROCK_AUTOMATION = "$Root\smoke.json"
Start-Process "$Root\mock-rally.exe" -WindowStyle Hidden -RedirectStandardOutput "$Root\rally.log" -RedirectStandardError "$Root\rally.err" -PassThru | Select-Object Id,ProcessName
Start-Process "$Root\mock-model.exe" -WindowStyle Hidden -RedirectStandardOutput "$Root\model.log" -RedirectStandardError "$Root\model.err" -PassThru | Select-Object Id,ProcessName
Start-Process "$Root\$Binary" -WorkingDirectory $Root -RedirectStandardOutput "$Root\ui.log" -RedirectStandardError "$Root\ui.err" -PassThru | Select-Object Id,ProcessName
if ($Layout) {
    Start-Process 'C:\Users\SeanMcNamara\dev\tools\ahk\AutoHotkey64.exe' -ArgumentList "`"$Root\windows-layout-input.ahk`"" -PassThru | Select-Object Id,ProcessName
}
