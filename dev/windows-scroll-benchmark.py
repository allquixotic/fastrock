#!/usr/bin/env python3
"""Repeatable software-rendered transcript scrolling. Embedded Goose and loopback Bedrock/MCP fixtures only."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import sys
import time
import ctypes
from ctypes import wintypes
from embedded_provider_fixture import ProviderFixture, ProcessGuard

assert sys.platform == 'win32', 'Run benchmarks on Windows, never Sean\'s Mac'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--frames', type=int, default=360)
parser.add_argument('--scale', type=float, default=1.0)
parser.add_argument('--size', type=int, nargs=2, default=[1440, 1000])
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
out = args.output.resolve()
out.mkdir(parents=True, exist_ok=True)
for folder in ('home', 'codex', 'project'):
    (out / folder).mkdir(exist_ok=True)
scenarios = {
    'prose': '\n\n'.join(f'Paragraph {i}: **bold text**, a [link](https://example.com), and ordinary wrapped conversation text. ' * 3 for i in range(120)),
    'table': '| Item | Description | Status |\n|---|---|---|\n' + '\n'.join(f'| {i} | Row with **formatted** text and a [link](https://example.com/{i}) | Ready |' for i in range(400)),
    'code': '```text\n' + '\n'.join(f'line {i}: software rendering and native text selection' for i in range(1600)) + '\n```',
    'tools': None,
}
scenarios['streaming'] = scenarios['prose']
steps = [{'wait_ready': 30000}, {'resize': args.size}]
for name, content in scenarios.items():
    steps += [{'new_thread': str(out / 'project')}, {'wait_idle': 30000},
              *([{'transcript': 'card:Benchmark ' + name + '|' + content}] if content is not None else
                [{'send': 'benchmark-tools'}, {'wait_idle': 30000}, {'transcript': 'expand-all'}]), {'wait': 1000},
              {'transcript': 'scroll-top'}, {'wait': 500},
              *([{'send': 'benchmark-stream'}, {'wait': 300}, {'transcript': 'scroll-top'}, {'wait': 300}] if name == 'streaming' else []),
              {'scroll_benchmark': {'frames': args.frames, 'delta': -12, 'output': str(out / (name + '.json'))}},
              {'snapshot': str(out / (name + '.png'))}, {'wait': 200}]
    if name == 'streaming': steps += [{'shortcut': 'escape'}, {'wait_idle': 30000}]
# Recovery exercises the real native presentation path. Rust tests also corrupt
# the presentation buffer while retaining unchanged content deterministically.
steps += [{'minimized': True}, {'wait': 300}, {'minimized': False}, {'wait': 500},
          {'snapshot': str(out / 'restored.png')}, {'quit': True}]
script = out / 'steps.json'
script.write_text(json.dumps(steps), encoding='utf-8')
provider = ProviderFixture(tools=True)
env = provider.environment(out, streaming=True)
env.update(CODEX_GUI_AUTOMATION=str(script), SLINT_SCALE_FACTOR=str(args.scale))
env.pop('SLINT_DEBUG_PERFORMANCE', None)
def restore_windows(pid):
    user32 = ctypes.windll.user32
    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def visit(hwnd, _):
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid and user32.IsIconic(hwnd):
            user32.ShowWindowAsync(hwnd, 9)  # SW_RESTORE; hidden timers can pause.
        return True
    user32.EnumWindows(visit, 0)

with (out / 'app.log').open('w', encoding='utf-8') as log:
    process = subprocess.Popen([str(args.binary.resolve()), '--remote', 'goose'], env=env, cwd=root, stdout=log, stderr=log)
    try:
        peak = ProcessGuard(process, timeout=300).wait(lambda: restore_windows(process.pid))
    finally:
        provider.close()
        provider.write_evidence(out)
log = (out / 'app.log').read_text(encoding='utf-8')
assert 'timed out' not in log and 'benchmark write failed' not in log, log[-3000:]

def distribution(values):
    values = sorted(values)
    return {k: round(values[min(len(values)-1, int((len(values)-1)*q))] / 1000, 3)
            for k, q in [('p50_ms', .5), ('p95_ms', .95), ('p99_ms', .99)]}

assert len(provider.tool_calls) == 6, provider.tool_calls
assert any(request['prompt'] == 'benchmark-stream' for request in provider.requests)
receipt = dict(backend='embedded-goose', peak_process_bytes=peak, binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
               host=platform.node(), os=platform.platform(), requested_frames=args.frames,
               input='Slint native wheel events, 16.667 ms timer; tick latency includes event-loop work', scenarios={})
for name in scenarios:
    data = json.loads((out / (name + '.json')).read_text())
    frames = data['frames'][10:]  # exclude warm-up
    assert len(frames) >= args.frames * .75, (name, 'insufficient rendered frames', len(frames))
    assert abs(data['positions'][-1] - data['positions'][0]) > 1000, (name, 'pane did not scroll')
    receipt['scenarios'][name] = dict(frames=len(frames), scale=data['scale_factor'], size=data['size'],
        render=distribution([f['render_us'] for f in frames]),
        present=distribution([f['present_us'] for f in frames]),
        frame=distribution([f['total_us'] for f in frames]),
        event_loop=distribution(data['tick_us'][10:]),
        dirty_fraction=round(statistics.mean(f['dirty_pixels']/f['window_pixels'] for f in frames), 4))
(out / 'summary.json').write_text(json.dumps(receipt, indent=2) + '\n')
print(json.dumps(receipt, indent=2))
