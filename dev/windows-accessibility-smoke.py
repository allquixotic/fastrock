#!/usr/bin/env python3
"""V103: active Windows UI Automation while native controls are replaced/focused.
Only attaches to the disposable fixture PID started here; never probes another app.
"""
import argparse, base64, hashlib, json, os, subprocess
from pathlib import Path
from embedded_provider_fixture import ProviderFixture, ProcessGuard

assert os.name == 'nt', 'Native accessibility verification runs on Windows only'
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('binary', type=Path)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
binary, out = a.binary.resolve(), a.output.resolve()
out.mkdir(parents=True, exist_ok=False)
fixture = ProviderFixture()
env = fixture.environment(out)
env['FASTROCK_WEBHOOK_BIND'] = '127.0.0.1:0'
steps = [{'wait_ready': 30000}, {'resize': [1440, 1000]}, {'wait': 5000}]
for cycle in range(20):
    steps += [{'open_settings': 'goose'}, {'wait': 100},
              {'settings': ['profiles', 'new']}, {'wait': 150},
              {'key': 'tab'}, {'wait': 100}, {'key': 'tab'},
              {'open_settings': 'appearance'}, {'wait': 150}]
steps += [{'open_settings': 'goose'}, {'wait': 1000},
          {'snapshot': str(out/'accessible.png')}, {'wait': 2000}, {'quit': True}]
(out/'automation.json').write_text(json.dumps(steps), encoding='utf-8')
env['CODEX_GUI_AUTOMATION'] = str(out/'automation.json')
# UIA queries and focus requests run outside Slint's event thread. Polling only
# this PID's HWND activates AccessKit; destroyed controls are expected mid-query.
client = out/'probe.ps1'
client.write_text(r'''param([int]$TargetPid, [string]$Receipt)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$queries = 0; $focuses = 0; $maxNodes = 0
$focusTargets = [System.Collections.Generic.HashSet[string]]::new()
$until = [DateTime]::UtcNow.AddSeconds(90)
while ([DateTime]::UtcNow -lt $until) {
    $process = Get-Process -Id $TargetPid -ErrorAction SilentlyContinue
    if (!$process) { break }
    $handle = $process.MainWindowHandle
    if ($handle -ne 0) {
        try {
            $root = [System.Windows.Automation.AutomationElement]::FromHandle($handle)
            $nodes = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants,
                [System.Windows.Automation.Condition]::TrueCondition)
            if ($nodes.Count -gt 5) { $queries++; $maxNodes = [Math]::Max($maxNodes, $nodes.Count) }
            for ($index = $nodes.Count - 1; $index -ge 0; $index--) {
                $node = $nodes[$index]
                if ($node.Current.ControlType -eq [System.Windows.Automation.ControlType]::Edit -and
                    $node.Current.IsEnabled -and !$node.Current.IsOffscreen -and $node.Current.IsKeyboardFocusable) {
                    $node.SetFocus(); $focuses++
                    [void]$focusTargets.Add(($node.GetRuntimeId() -join ',')); break
                }
            }
        } catch [System.Windows.Automation.ElementNotAvailableException] {
            # Expected while the fixture destroys a settings page.
        }
    }
    Start-Sleep -Milliseconds 40
}
@{ queries=$queries; focuses=$focuses; focus_targets=$focusTargets.Count; max_nodes=$maxNodes } | ConvertTo-Json | Set-Content -Encoding UTF8 $Receipt
if ($queries -lt 10 -or $focuses -lt 10) { throw 'UI Automation did not exercise the fixture' }
''', encoding='utf-8')
process = probe = None
try:
    with (out/'app.log').open('w', encoding='utf-8') as log, (out/'uia.log').open('w', encoding='utf-8') as uia_log:
        process = subprocess.Popen([str(binary)], env=env, cwd=out, stdout=log, stderr=log)
        guard = ProcessGuard(process, timeout=120)
        code = f"$TargetPid={process.pid}; $Receipt='{str(out/'uia.json').replace(chr(39), chr(39)*2)}';\n" + client.read_text().split('\n', 1)[1]
        encoded = base64.b64encode(code.encode('utf-16le')).decode('ascii')
        probe = subprocess.Popen(['powershell', '-NoProfile', '-EncodedCommand', encoded], stdout=uia_log, stderr=uia_log)
        guard.wait()
        assert process.returncode == 0, 'Fixture crashed; inspect app.log'
        assert probe.wait(timeout=15) == 0, 'UI Automation probe failed; inspect uia.log'
    observed = json.loads((out/'uia.json').read_text(encoding='utf-8-sig'))
    assert observed['queries'] >= 10 and observed['focuses'] >= 10, observed
    assert observed['focus_targets'] >= 2, 'UIA must focus distinct controls across page changes'
    assert (out/'accessible.png').stat().st_size > 1000
    receipt = dict(result='PASS', binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
        peak_process_bytes=guard.peak, state_changes=40, **observed)
    (out/'acceptance.json').write_text(json.dumps(receipt, indent=2)+'\n')
    print(json.dumps(receipt), flush=True)
finally:
    for child in (probe, process):
        if child is not None and child.poll() is None:
            child.kill(); child.wait(timeout=10)
    fixture.close(); fixture.write_evidence(out)
