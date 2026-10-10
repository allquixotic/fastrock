#!/usr/bin/env python3
"""Windows-only real GUI: cached user-only purposes, debounce, renames and selection."""
import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import threading
assert sys.platform == 'win32', 'Never run GUI tests on Sean\'s Mac'
import ctypes
from ctypes import wintypes

# Native menus run a modal Win32 loop. Invoke their real accessibility default
# action from outside that loop; never inject global desktop keystrokes.
def drive_copy_menus(process, evidence, errors):
    user32 = ctypes.windll.user32
    class GUIThreadInfo(ctypes.Structure):
        _fields_ = [('size', wintypes.DWORD), ('flags', wintypes.DWORD),
                    ('active', wintypes.HWND), ('focus', wintypes.HWND),
                    ('capture', wintypes.HWND), ('menu_owner', wintypes.HWND),
                    ('move_size', wintypes.HWND), ('caret', wintypes.HWND),
                    ('caret_rect', wintypes.RECT)]
    user32.GetGUIThreadInfo.argtypes = [wintypes.DWORD, ctypes.POINTER(GUIThreadInfo)]
    user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    user32.GetClassNameW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
    user32.IsWindowVisible.argtypes = [wintypes.HWND]
    user32.OpenClipboard.argtypes = [wintypes.HWND]
    class VariantValue(ctypes.Union):
        _fields_ = [('integer', wintypes.LONG), ('pointer', ctypes.c_void_p), ('record', ctypes.c_byte * 16)]
    class Variant(ctypes.Structure):
        _fields_ = [('kind', wintypes.WORD), ('reserved1', wintypes.WORD), ('reserved2', wintypes.WORD), ('reserved3', wintypes.WORD), ('value', VariantValue)]
    ole32 = ctypes.windll.ole32
    oleacc = ctypes.windll.oleacc
    oleacc.AccessibleObjectFromWindow.argtypes = [wintypes.HWND, wintypes.DWORD, ctypes.c_void_p, ctypes.POINTER(ctypes.c_void_p)]
    oleacc.AccessibleObjectFromWindow.restype = ctypes.HRESULT
    initialized = ole32.CoInitializeEx(None, 2) >= 0
    user32.SendMessageW.restype = ctypes.c_void_p
    user32.SendMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
    user32.GetMenuItemCount.argtypes = [wintypes.HMENU]
    user32.GetMenuStringW.argtypes = [wintypes.HMENU, wintypes.UINT, wintypes.LPWSTR, ctypes.c_int, wintypes.UINT]
    user32.GetMenuState.argtypes = [wintypes.HMENU, wintypes.UINT, wintypes.UINT]
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    user32.EnumWindows.argtypes = [callback_type, wintypes.LPARAM]
    seen = set()
    try:
        while process.poll() is None:
            windows = []
            @callback_type
            def visit(hwnd, _):
                name = ctypes.create_unicode_buffer(256)
                user32.GetClassNameW(hwnd, name, len(name))
                pid = wintypes.DWORD()
                user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
                if name.value == '#32768' and pid.value == process.pid and user32.IsWindowVisible(hwnd):
                    windows.append(hwnd)
                return True
            user32.EnumWindows(visit, 0)
            for hwnd in windows:
                if hwnd in seen:
                    continue
                menu = user32.SendMessageW(hwnd, 0x01E1, 0, 0)  # MN_GETHMENU
                items = []
                for index in range(user32.GetMenuItemCount(menu)):
                    label = ctypes.create_unicode_buffer(256)
                    user32.GetMenuStringW(menu, index, label, len(label), 0x400)
                    items.append(label.value.replace('&', ''))
                if not items or items[0] != 'Copy':
                    continue
                info = GUIThreadInfo(size=ctypes.sizeof(GUIThreadInfo))
                menu_thread = user32.GetWindowThreadProcessId(hwnd, None)
                assert user32.GetGUIThreadInfo(menu_thread, ctypes.byref(info))
                owner = info.menu_owner
                owner_pid = wintypes.DWORD()
                user32.GetWindowThreadProcessId(owner, ctypes.byref(owner_pid))
                assert owner_pid.value == process.pid, 'Copy menu lost GUI ownership'
                assert user32.GetMenuState(menu, 0, 0x400) & 3 == 0, 'Copy is disabled for selected text'
                # Empty the clipboard so this cannot pass with the Ctrl+C result.
                assert user32.OpenClipboard(None)
                try:
                    assert user32.EmptyClipboard()
                finally:
                    user32.CloseClipboard()
                seen.add(hwnd)
                evidence.append(items)
                interface = ctypes.c_void_p()
                import uuid
                iid = (ctypes.c_byte * 16).from_buffer_copy(uuid.UUID('618736e0-3c3d-11cf-810c-00aa00389b71').bytes_le)
                assert oleacc.AccessibleObjectFromWindow(hwnd, 0xFFFFFFFC, ctypes.byref(iid), ctypes.byref(interface)) >= 0
                try:
                    table = ctypes.cast(interface, ctypes.POINTER(ctypes.POINTER(ctypes.c_void_p))).contents
                    invoke = ctypes.WINFUNCTYPE(ctypes.HRESULT, ctypes.c_void_p, Variant)(table[25])  # IAccessible.accDoDefaultAction
                    child = Variant(kind=3, value=VariantValue(integer=1))  # VT_I4: first menu item.
                    assert invoke(interface, child) >= 0
                finally:
                    release = ctypes.WINFUNCTYPE(wintypes.ULONG, ctypes.c_void_p)(table[2])
                    release(interface)
            if not windows:
                seen.clear()  # Win32 may reuse a popup HWND for the next menu.
            time.sleep(.05)
    except Exception as error:
        errors.append(str(error))
    finally:
        if initialized:
            ole32.CoUninitialize()
ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('--renderer', choices=['software', 'auto', 'gpu'], default='software')
parser.add_argument('--output', type=Path, default=ROOT / 'dist/purpose-selection-smoke')
options = parser.parse_args()
options.output.mkdir(parents=True, exist_ok=True)
output = options.output.resolve()
with tempfile.TemporaryDirectory(prefix='codex-gui-purpose-') as directory:
    work = Path(directory)
    home, project = work / 'home', work / 'project'
    project.mkdir()
    (project / 'AGENTS.md').write_text('PROJECT_SECRET must not enter purpose inference.\n')
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        port = listener.getsockname()[1]
    import mock_responses
    mock_responses.write_config(home, port)
    config = home / 'config.toml'
    config.write_text(config.read_text().replace('sandbox_mode = "workspace-write"', 'sandbox_mode = "danger-full-access"').replace('model = "mock-model"', 'model = "mock-model"\nmodel_reasoning_effort = "ultra"'))
    with config.open('a') as file:
        file.write(f'\n[projects.{json.dumps(str(project))}]\ntrust_level = "trusted"\n')
    catalog_path = home / 'models.json'
    catalog = json.loads(catalog_path.read_text())
    cheap = dict(catalog['models'][0], slug='gpt-6-luna', display_name='Mock Luna')
    catalog['models'].append(cheap)
    catalog_path.write_text(json.dumps(catalog))
    requests = output / 'requests.jsonl'
    with open(output / 'mock.log', 'w') as mock_log:
        mock = subprocess.Popen([sys.executable, str(ROOT / 'dev/mock_responses.py'), '--port', str(port), '--request-log', str(requests), '--fail-model', 'gpt-6-luna'], stdout=mock_log, stderr=mock_log)
        try:
            for _ in range(100):
                try:
                    with socket.create_connection(('127.0.0.1', port), timeout=.1):
                        break
                except OSError:
                    time.sleep(.1)
            else:
                raise RuntimeError('Mock did not start')
            def run(name, steps, expected_menus=0):
                script = work / f'{name}.json'
                script.write_text(json.dumps(steps))
                with open(output / f'{name}.log', 'w') as gui_log:
                    process = subprocess.Popen([str(options.binary.resolve()), '--renderer', options.renderer], env=dict(os.environ, CODEX_HOME=str(home), CODEX_GUI_AUTOMATION=str(script)), stdout=gui_log, stderr=gui_log)
                    menu_evidence, menu_errors = [], []
                    driver = threading.Thread(target=drive_copy_menus, args=(process, menu_evidence, menu_errors), daemon=True)
                    driver.start()
                    try:
                        assert process.wait(timeout=180) == 0, 'GUI process failed'
                    finally:
                        if process.poll() is None:
                            process.kill()
                            process.wait(timeout=10)
                        driver.join(timeout=5)
                    for startup_log in home.glob("log/*.log"):
                        shutil.copyfile(startup_log, output / f"{name}-{startup_log.name}")
                    assert not menu_errors, menu_errors
                    assert len(menu_evidence) == expected_menus, menu_evidence
                    (output / f'{name}-native-menus.json').write_text(json.dumps(menu_evidence))
                log = (output / f'{name}.log').read_text()
                assert 'timed out' not in log, log
                assert 'selection text not found' not in log, log
                assert 'snapshot failed' not in log, log
            run('first', [
                {'wait_ready': 60000}, {'new_thread': str(project)}, {'wait_idle': 30000},
                {'send': 'purpose-first USER_ONE'}, {'wait': 500}, {'wait_idle': 30000}, {'wait_purpose': 60000}, {'wait': 300},
                {'sidebar': ['dump', str(output / 'initial.json')]},
                {'selection': ['drag', 'Selection sample bold', '0', '16']}, {'wait': 300}, {'key': 'Ctrl+C'}, {'wait': 200},
                {'selection': ['capture', str(output / 'drag.txt')]}, {'snapshot': str(output / 'drag-selection.png')},
                {'selection': ['click', 'Selection sample bold', '0']}, {'wait': 200},
                {'key': 'Shift+Right'}, {'key': 'Shift+Right'}, {'key': 'Shift+Right'}, {'wait': 200}, {'key': 'Ctrl+C'}, {'wait': 200},
                {'selection': ['capture', str(output / 'keyboard.txt')]},
                {'selection': ['menu', 'Selection sample bold', '1']}, {'wait': 1000}, {'selection': ['capture', str(output / 'menu.txt')]},
                {'selection': ['click', 'Selection sample bold', '0']}, {'wait': 200}, {'key': 'Shift+Left'}, {'key': 'Shift+Left'}, {'wait': 200}, {'key': 'Ctrl+C'}, {'wait': 200},
                {'selection': ['capture', str(output / 'paragraph-boundary.txt')]},
                {'selection': ['drag', 'Selection sample bold', '0', '36']}, {'wait': 300},
                {'selection': ['click', 'Selection sample bold', '35']}, {'wait': 300},
                {'selection': ['drag', 'purpose-first USER_ONE', '0', '13']}, {'wait': 200}, {'key': 'Ctrl+C'}, {'wait': 200},
                {'selection': ['capture', str(output / 'user.txt')]},
                {'wait': 700},  # Separate this click from the drag: native inputs honor double-click selection.
                {'selection': ['click', 'purpose-first USER_ONE', '0']}, {'wait': 200}, {'key': 'Shift+Right'}, {'key': 'Shift+Right'}, {'wait': 200}, {'key': 'Ctrl+C'}, {'wait': 200},
                {'selection': ['capture', str(output / 'user-keyboard.txt')]},
                {'selection': ['menu', 'purpose-first USER_ONE', '1']}, {'wait': 1000}, {'selection': ['capture', str(output / 'user-menu.txt')]},
                {'log': 'RESIZE_START'}, {'sidebar': ['width', '360']}, {'wait': 5000}, {'sidebar': ['width', '280']},
                {'wait': 11000}, {'wait_purpose': 60000}, {'wait': 300}, {'sidebar': ['dump', str(output / 'resized.json')]},
                {'tab_action': 'rename'}, {'wait': 200}, {'key': 'Ctrl+A'}, {'key': 'Protected title'}, {'dialog': True}, {'wait': 500},
                {'send': 'purpose-second USER_TWO'}, {'wait': 500}, {'wait_idle': 30000}, {'wait_purpose': 60000}, {'wait': 300},
                {'sidebar': ['dump', str(output / 'renamed.json')]}, {'snapshot': str(output / 'purpose-sidebar.png')}, {'quit': True},
            ], expected_menus=2)
            rows = lambda name: json.loads((output / name).read_text())
            first, resized, renamed = rows('initial.json'), rows('resized.json'), rows('renamed.json')
            assert first['rows'][0]['tooltip'] == 'Repair authentication.', first
            assert resized['rows'][0]['tooltip'] == first['rows'][0]['tooltip'], resized
            assert resized['rows'][0]['generated'], resized
            assert len(resized['rows'][0]['title']) <= resized['maximum'], resized
            assert '…' not in resized['rows'][0]['title'], resized
            assert renamed['rows'][0]['title'] == 'Protected title', renamed
            assert renamed['rows'][0]['tooltip'] == 'Fix authentication and add regression tests.', renamed
            assert not renamed['rows'][0]['generated'], renamed
            assert (output / 'drag.txt').read_text() == 'Selection sample', (output / 'drag.txt').read_text()
            assert (output / 'keyboard.txt').read_text() == 'Sel', (output / 'keyboard.txt').read_text()
            assert (output / 'menu.txt').read_text() == 'Sel'
            assert (output / 'paragraph-boundary.txt').read_text() == '.\n'
            assert (output / 'user.txt').read_text() == 'purpose-first'
            assert (output / 'user-keyboard.txt').read_text() == 'pu'
            assert (output / 'user-menu.txt').read_text() == 'pu'
            assert (output / 'first.log').read_text().count('automation: web link activated: https://example.com') == 1, 'Link drag opened a link, or a normal click after selection stopped working'
            bodies = [json.loads(line) for line in requests.read_text().splitlines()]
            successful = [body for body in bodies if body.get('model') == 'mock-model']
            assert any('PROJECT_SECRET' in json.dumps(body) for body in successful if not mock_responses.wants_json_schema(body)), 'Privacy control failed: ordinary inference never received project instructions'
            purposes = [body for body in successful if 'short' in (((body.get('text') or {}).get('format') or {}).get('schema') or {}).get('properties', {})]
            resize_calls = [body for body in successful if 'titles' in (((body.get('text') or {}).get('format') or {}).get('schema') or {}).get('properties', {})]
            assert len(purposes) == 2, purposes
            assert len(resize_calls) == 1, resize_calls
            assert any(body.get('model') == 'gpt-6-luna' for body in bodies), 'Fast model was never tried'
            for body in purposes + resize_calls:
                assert body['reasoning']['effort'] == 'low', 'Background inference inherited expensive reasoning'
                assert 'ASSISTANT_SECRET' not in json.dumps(body), body
                assert 'PROJECT_SECRET' not in json.dumps(body), body
                assert not any(item.get('role') == 'assistant' for item in body.get('input', [])), body
            prompts = [mock_responses.last_user_text(body) for body in purposes]
            assert 'USER_ONE' in prompts[0] and 'USER_TWO' not in prompts[0]
            assert 'USER_ONE' in prompts[1] and 'USER_TWO' in prompts[1]
            assert 'USER_ONE' not in mock_responses.last_user_text(resize_calls[0])
            assert resize_calls[0]['_mock_received_at'] - purposes[0]['_mock_received_at'] >= 15, 'Resize debounce fired early'
            cache = home / 'gui-thread-summaries.json'
            shutil.copyfile(cache, output / cache.name)
            before = len(bodies)
            run('reopened', [{'wait_ready': 60000}, {'resume': renamed['rows'][0]['id']}, {'wait_idle': 30000}, {'wait': 12000}, {'sidebar': ['dump', str(output / 'reopened.json')]}, {'quit': True}])
            after = requests.read_text().splitlines()
            assert len(after) == before, 'Reopening consumed inference'
            assert rows('reopened.json')['rows'][0]['title'] == 'Protected title'
            print('Purpose/selection smoke passed: user-only input, Luna fallback, combined output, debounce, persistent cache, manual protection, actual drag and Shift/arrows, clipboard and native Copy menus for user/assistant text.')
        finally:
            mock.terminate()
            mock.wait(timeout=10)
