#!/usr/bin/env python3
"""V37: native pointer/keyboard/clipboard selection with a local Goose fixture."""
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
steps += [{'send': 'selection-tools'}, {'wait_idle': 30000}, {'click_text': 'Goose.Selection tool'},
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
          {'selection': ['capture', str(out / 'tool-all.txt')]}, {'quit': True}]

script = out / 'steps.json'
script.write_text(json.dumps(steps), encoding='utf-8')
env = dict(os.environ, FASTROCK_HOME=str(out / 'home'), CODEX_HOME=str(out / 'codex'),
           FASTROCK_GOOSE=sys.executable, FASTROCK_GOOSE_FIXTURE=str(root / 'dev/external-goose-fixture.py'),
           CODEX_GUI_AUTOMATION=str(script), SLINT_BACKEND='winit-software', SLINT_SCALE_FACTOR=str(args.scale))
for key in ('FASTROCK_FIXTURE_ARTIFACT', 'FASTROCK_RALLY_TOKEN'):
    env.pop(key, None)
evidence, errors = [], []
with (out / 'app.log').open('w', encoding='utf-8') as log:
    process = subprocess.Popen([str(args.binary.resolve()), '--remote', 'goose'], cwd=root, env=env, stdout=log, stderr=log)
    driver = threading.Thread(target=drive_copy_menus, args=(process, evidence, errors), daemon=True)
    driver.start()
    try:
        assert process.wait(timeout=120) == 0, 'GUI process failed'
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        driver.join(timeout=5)
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
assert 'Goose.Selection tool' in complete and 'ARGUMENT_END' in complete, complete
assert all(line(i) in complete for i in range(24)), complete
receipt = dict(result='PASS', binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
               scale=args.scale, clipboard_cases=list(expected), native_menus=evidence,
               complete_tool_copy=True, cross_virtual_row_selection=True)
(out / 'acceptance.json').write_text(json.dumps(receipt, indent=2) + '\n')
print(json.dumps(receipt, indent=2))
