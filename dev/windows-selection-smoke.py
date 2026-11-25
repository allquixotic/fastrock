#!/usr/bin/env python3
"""V37/V120: Windows selection, clipboard, and rendered contrast in both themes."""
import argparse
import ast
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time
from embedded_provider_fixture import ProviderFixture, ProcessGuard

assert sys.platform == 'win32', 'Run GUI checks on Windows, never Sean\'s Mac'
root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('--output', type=Path, default=root / 'build/selection-smoke')
parser.add_argument('--scale', type=float, default=1)
args = parser.parse_args()
out = args.output.resolve()
out.mkdir(parents=True, exist_ok=True)
for folder in ('home', 'codex', 'project'):
    (out / folder).mkdir(exist_ok=True)

# Reuse the existing native Win32 menu driver without executing its old backend fixture.
source = ast.parse((root / 'dev/windows-purpose-selection-smoke.py').read_text())
function = next(n for n in source.body if isinstance(n, ast.FunctionDef) and n.name == 'drive_copy_menus')
exec(compile(ast.Module(body=[function], type_ignores=[]), '<native-menu-driver>', 'exec'))

# Resize uses physical pixels. Keep the cross-row targets inside the viewport
# at high DPI instead of accidentally testing offscreen virtual rows.
steps = [{'wait_ready': 30000}, {'resize': [2560, 1600] if args.scale == 2 else [1440, 1000]}]
expected = {}

def capture(name, text):
    expected[name] = text
    return [{'key': 'Ctrl+C'}, {'wait': 150}, {'selection': ['capture', str(out / (name + '.txt'))]}]

def drag(name, needle, start, end, expected_text, gutter=0, end_node=''):
    # Separate clicks so native TextInput's word/paragraph gestures stay intentional.
    return ([{'wait': 700}, {'selection': ['drag', needle, str(start), str(end), str(gutter), end_node]},
             {'wait': 150}] + capture(name, expected_text))

def new_thread():
    return [{'new_thread': str(out / 'project')}, {'wait_idle': 30000}]

steps += new_thread()
steps += [{'transcript': 'card:Selection fixture|Padding **selection** café 👩‍💻 with a [link](https://example.com).\n\nSecond paragraph for reverse dragging.'}, {'wait': 400}]
steps += drag('rich-gutter', 'Padding selection', 0, 17, 'Padding selection', -8)
steps += drag('rich-reverse', 'Padding selection', 17, 0, 'Padding selection')
steps += [{'selection': ['click', 'Padding selection', '0']}, {'key': 'Shift+Right'}, {'key': 'Shift+Right'}]
steps += capture('rich-keyboard', 'Pa')
steps += [{'selection': ['menu', 'Padding selection', '1']}, {'wait': 900},
          {'selection': ['capture', str(out / 'rich-menu.txt')]}]
expected['rich-menu'] = 'Pa'
steps += drag('rich-unicode', 'Padding selection', 18, 35, 'café 👩‍💻')
steps += [{'snapshot': str(out / 'rich.png')}]
steps += drag('link-drag', 'Padding selection', 0, 45,
              'Padding selection café 👩‍💻 with a li', -8)
steps += [{'selection': ['click', 'Padding selection', '44']}, {'wait': 200}]

steps += new_thread()
steps += [{'send': 'User padding selection'}, {'wait_idle': 30000}, {'wait': 400}]
steps += drag('user-gutter', 'User padding selection', 0, 12, 'User padding', -8)
steps += drag('user-glyph', 'User padding selection', 5, 12, 'padding')
steps += [{'wait': 700}, {'selection': ['click', 'User padding selection', '0']},
          {'key': 'Shift+Right'}, {'key': 'Shift+Right'}]
steps += capture('user-keyboard', 'Us')
steps += [{'selection': ['menu', 'User padding selection', '1']}, {'wait': 900},
          {'selection': ['capture', str(out / 'user-menu.txt')]}]
expected['user-menu'] = 'Us'

steps += new_thread()
steps += [{'transcript': 'card:Code fixture|```text\nCode padding selection\nsecond code line\n```'}, {'wait': 400}]
steps += drag('code-gutter', 'Code padding selection', 0, 12, 'Code padding', -8)
steps += drag('code-second-line', 'Code padding selection', 23, 29, 'second', -8)
steps += drag('code-glyph', 'Code padding selection', 5, 12, 'padding')

steps += new_thread()
steps += [{'send': 'selection-tools'}, {'wait_idle': 30000}, {'transcript': 'expand-all'},
          {'transcript': 'scroll-top'}, {'wait': 600}]
steps += drag('tool-arguments', 'Arguments:', 0, 10, 'Arguments:', -8)
line = lambda i: f'Selection output {i:02d}: **literal** <tag> é 👩‍💻'
steps += drag('tool-result', line(0), 0, len(line(0).encode()), line(0), -8)
steps += [{'selection': ['menu', line(0), '1']}, {'wait': 900},
          {'selection': ['capture', str(out / 'tool-menu.txt')]}]
expected['tool-menu'] = line(0)
# Tool chunks hold sixteen lines including the title and arguments. Cross the
# actual ListView row boundary, preserving literal markup and UTF-8 bytes.
steps += drag('tool-cross-chunk', line(12), 0, len(line(16).encode()),
              '\n'.join(line(i) for i in range(12, 17)), -8, line(16))
steps += [{'snapshot': str(out / 'tools.png')}, {'click_text': 'Copy All'}, {'wait': 250},
          {'selection': ['capture', str(out / 'tool-all.txt')]}]

selection_colors = {
    'dark': dict(read_only='#2871e5', editable='#00847c', surface='#2c2c2c'),
    'light': dict(read_only='#164fae', editable='#006b61', surface='#ebebed'),
}
contrast_screenshots = {}


def selection_snapshot(theme, control, role):
    name = f'{theme}-{control}.png'
    contrast_screenshots[name] = selection_colors[theme][role]
    return [{'wait': 250}, {'snapshot': str(out / name)}]


for theme in selection_colors:
    steps += [{'theme': theme}, *new_thread()]
    steps += [{'composer': ['type', 'Editable composer selection']},
              {'click_text': 'Editable composer selection'}, {'key': 'Ctrl+A'}]
    steps += selection_snapshot(theme, 'composer', 'editable')
    steps += [{'key': 'Ctrl+Shift+P'}, {'wait': 250}, {'palette': ['query', 'provider profiles']},
              {'key': 'Ctrl+A'}]
    steps += selection_snapshot(theme, 'line-edit', 'editable')
    steps += [{'key': 'escape'}, {'forward': 'Editable message selection'}, {'wait': 250},
              {'click_text': 'Editable message selection'}, {'key': 'Ctrl+A'}]
    steps += selection_snapshot(theme, 'text-edit', 'editable')
    steps += [{'forward_submit': 'cancel'},
              {'transcript': 'card:Color fixture|Rich **selection color** sample.\n\n```text\nPlain selection color sample.\n```'},
              {'wait': 500}, {'selection': ['drag', 'Rich selection color', '0', '26', '-8']}]
    steps += selection_snapshot(theme, 'rich-transcript', 'read_only')
    steps += [{'wait': 500}, {'selection': ['drag', 'Plain selection color', '0', '27']}]
    steps += selection_snapshot(theme, 'plain-transcript', 'read_only')
    steps += [{'file_command': 'text:Color fixture|Selectable file viewer contents'},
              {'wait': 500}, {'click_text': 'Selectable file viewer contents'}, {'key': 'Ctrl+A'}]
    steps += selection_snapshot(theme, 'file-viewer', 'read_only')
steps += [{'quit': True}]


def verify_contrast():
    def luminance(color):
        channels = [int(color[i:i + 2], 16) / 255 for i in (1, 3, 5)]
        linear = [v / 12.92 if v <= .04045 else ((v + .055) / 1.055) ** 2.4 for v in channels]
        return sum(v * w for v, w in zip(linear, (.2126, .7152, .0722)))

    def contrast(first, second):
        low, high = sorted(map(luminance, (first, second)))
        return (high + .05) / (low + .05)

    for colors in selection_colors.values():
        for role in ('read_only', 'editable'):
            assert contrast(colors[role], '#ffffff') >= 4.5, colors
            assert contrast(colors[role], colors['surface']) >= 3, colors

    # System.Drawing is available in Windows PowerShell on both the VM and the
    # hosted runner. Count actual opaque highlight pixels from Slint's snapshots.
    inputs = out / 'selection-colors.json'
    inputs.write_text(json.dumps(contrast_screenshots), encoding='utf-8')
    check = r"""
$ErrorActionPreference = 'Stop'
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System.Drawing;
public static class SelectionPixels {
    public static int[] Count(string path, string hex) {
        using (var bitmap = new Bitmap(path)) {
            int expected = ColorTranslator.FromHtml(hex).ToArgb();
            int count = 0;
            int left = bitmap.Width, top = bitmap.Height, right = -1, bottom = -1;
            for (int y = 0; y < bitmap.Height; y++)
                for (int x = 0; x < bitmap.Width; x++)
                    if (bitmap.GetPixel(x, y).ToArgb() == expected) {
                        count++;
                        left = System.Math.Min(left, x);
                        top = System.Math.Min(top, y);
                        right = System.Math.Max(right, x);
                        bottom = System.Math.Max(bottom, y);
                    }
            int lightGlyphPixels = 0;
            for (int y = top; y <= bottom; y++)
                for (int x = left; x <= right; x++) {
                    Color pixel = bitmap.GetPixel(x, y);
                    if (pixel.R >= 235 && pixel.G >= 235 && pixel.B >= 235)
                        lightGlyphPixels++;
                }
            return new[] { count, lightGlyphPixels };
        }
    }
}
'@
$inputs = Get-Content -Raw $env:FASTROCK_SELECTION_COLOR_FIXTURE | ConvertFrom-Json
$results = @{}
foreach ($image in $inputs.PSObject.Properties) {
    $path = Join-Path (Split-Path $env:FASTROCK_SELECTION_COLOR_FIXTURE) $image.Name
    $results[$image.Name] = [SelectionPixels]::Count($path, $image.Value)
}
$results | ConvertTo-Json -Compress
"""
    counts = json.loads(subprocess.check_output(
        ['powershell.exe', '-NoProfile', '-NonInteractive', '-Command', check],
        env=dict(os.environ, FASTROCK_SELECTION_COLOR_FIXTURE=str(inputs)), timeout=60,
    ))
    for name, color in contrast_screenshots.items():
        fill, glyphs = counts[name]
        assert fill >= 100 and glyphs >= 10, (name, color, counts[name])
    return {name: dict(fill=fill, light_glyphs=glyphs) for name, (fill, glyphs) in counts.items()}

script = out / 'steps.json'
script.write_text(json.dumps(steps), encoding='utf-8')
provider = ProviderFixture(tools=True)
env = provider.environment(out)
env.update(CODEX_GUI_AUTOMATION=str(script), SLINT_SCALE_FACTOR=str(args.scale))
evidence, errors = [], []
with (out / 'app.log').open('w', encoding='utf-8') as log:
    process = subprocess.Popen([str(args.binary.resolve()), '--remote', 'goose'], cwd=root, env=env, stdout=log, stderr=log)
    driver = threading.Thread(target=drive_copy_menus, args=(process, evidence, errors), daemon=True)
    driver.start()
    try:
        peak = ProcessGuard(process, timeout=120).wait()
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        driver.join(timeout=5)
        provider.close()
        provider.write_evidence(out)
log = (out / 'app.log').read_text(encoding='utf-8')
assert not errors, errors
assert len(evidence) == 3, evidence
for failure in ('timed out', 'selection text not found', 'selection end text not found', 'snapshot failed'):
    assert failure not in log, (failure, log[-4000:])
assert log.count('automation: web link activated: https://example.com') == 1, 'Dragging activated a link, or clicking did not'
for name, value in expected.items():
    actual = (out / (name + '.txt')).read_text(encoding='utf-8')
    assert actual == value, (name, repr(actual), repr(value))
complete = (out / 'tool-all.txt').read_text(encoding='utf-8')
assert 'selection tool' in complete.lower() and 'ARGUMENT_END' in complete, complete
assert [call['name'] for call in provider.tool_calls] == ['selection_tool'], provider.tool_calls
assert all(line(i) in complete for i in range(24)), complete
receipt = dict(result='PASS', binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
               backend='embedded-goose', peak_process_bytes=peak, scale=args.scale, clipboard_cases=list(expected), native_menus=evidence,
               complete_tool_copy=True, cross_virtual_row_selection=True,
               selection_contrast_pixels=verify_contrast())
(out / 'acceptance.json').write_text(json.dumps(receipt, indent=2) + '\n')
print(json.dumps(receipt, indent=2))
