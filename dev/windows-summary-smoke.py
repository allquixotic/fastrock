#!/usr/bin/env python3
"""V40/V44: native embedded-Goose conversation, summary search and settings.

Run on Windows only. Every provider request goes to this local HTTP fixture.
"""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
import importlib.util
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time

assert sys.platform == 'win32', "Run native acceptance on Windows only"
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--turns', type=int, default=1)
parser.add_argument('--navigation', action='store_true')
parser.add_argument('--tray', action='store_true', help='Close while a model runs, restore through the native tray, and quit from its menu')
parser.add_argument('--windows', action='store_true', help='Exercise two in-process windows and draft transfer')
parser.add_argument('--recovery', action='store_true', help='Restart during a send, review it, and explicitly resume only queued work')
args = parser.parse_args()
assert 1 <= args.turns <= 100
assert not args.windows or args.navigation, '--windows requires --navigation'
assert not args.recovery or (args.turns == 1 and not (args.windows or args.navigation or args.tray)), '--recovery is a separate one-turn scenario'
out = args.output.resolve()
out.mkdir(parents=True, exist_ok=False)
for folder in ('home', 'codex', 'project', 'goose/config', 'local', 'roaming'):
    (out / folder).mkdir(parents=True)
(out / 'aws-empty').write_text('')
(out / 'goose/config/config.yaml').write_text(
    'GOOSE_PROVIDER: aws_bedrock\nGOOSE_MODEL: openai.gpt-6.1-sol\n'
    'BEDROCK_DISABLE_STREAMING: true\nBEDROCK_MAX_RETRIES: 0\n'
    'GOOSE_DISABLE_SESSION_NAMING: true\nextensions: {}\n', encoding='utf-8')
requests = []
release_first_turn = threading.Event()


class Provider(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        summary = 'search index sidecar' in json.dumps(body)
        requests.append({'path': self.path, 'summary': summary})
        assert self.path.startswith('/model/') and self.path.endswith('/converse'), self.path
        turn = sum(not request['summary'] for request in requests)
        if args.recovery and not summary and turn == 2:
            release_first_turn.set()
        if args.recovery and not summary and turn == 1:
            assert release_first_turn.wait(60), 'No explicit Resume reached the provider'
        if args.tray and not summary and turn == 1:
            assert release_first_turn.wait(20), 'Native window was not hidden during the active turn'
        text = ('ObsidianNeedle: compaction-length keyword fixture for searching this conversation.'
                if summary else f'Native embedded Goose answer {turn}, produced by the local fixture.')
        response = json.dumps({'output': {'message': {'role': 'assistant', 'content': [{'text': text}]}},
                              'stopReason': 'end_turn', 'usage': {'inputTokens': 3, 'outputTokens': 4, 'totalTokens': 7},
                              'metrics': {'latencyMs': 1}}).encode()
        try:
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(response)))
            self.end_headers()
            self.wfile.write(response)
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
            assert args.recovery and turn == 1, 'Unexpected provider disconnect'


server = ThreadingHTTPServer(('127.0.0.1', 0), Provider)
threading.Thread(target=server.serve_forever, daemon=True).start()
endpoint = f'http://127.0.0.1:{server.server_port}'
rally = None
if args.navigation:
    fixture_path = Path(__file__).with_name('windows-rally-smoke.py')
    spec = importlib.util.spec_from_file_location('rally_fixture', fixture_path)
    fixture_module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fixture_module)
    rally = fixture_module.RallyFixture()
    threading.Thread(target=rally.server.serve_forever, daemon=True).start()
    (out / 'home/rally.json').write_text(json.dumps({
        'rallyEndpoint': rally.base, 'rallyWorkspace': rally.workspace['_ref'],
        'rallyProject': rally.project['_ref'], 'projectChildren': True,
    }), encoding='utf-8')
steps = [{'wait_ready': 30000}, {'resize': [1440, 1000]},
         {'new_thread': str(out / 'project')}, {'wait_idle': 30000}]
if args.recovery:
    steps += [{'send': 'Uncertain send: retain me for review'}, {'wait': 1200},
              {'composer': ['type', 'Queued recovery message é']}, {'composer': ['send']}, {'wait': 500},
              {'restart_server': True}, {'wait': 1200}, {'wait_ready': 30000}, {'wait_idle': 30000},
              {'click_text': 'Review interrupted sends (1)'}, {'wait': 300},
              {'click_text': 'Copy recovery records'}, {'wait': 300}, {'selection': ['capture', str(out / 'recovery-records.json')]},
              {'snapshot': str(out / 'recovery-paused.png')},
              {'transcript': 'export:' + str(out / 'recovery-visible.md')},
              {'sidebar': ['dump', str(out / 'recovery-paused.json')]}, {'wait': 2500},
              {'click_text': 'Resume'}, {'wait': 500}, {'wait_idle': 30000}, {'wait': 2000}]
else:
    for turn in range(1, args.turns + 1):
        steps += [{'send': f'Please give a short fixture answer {turn}'}, {'wait_idle': 30000}, {'wait': 2000}]
steps += [{'transcript': 'export-complete:' + str(out / 'complete.md')}, {'wait': 300},
         {'sidebar': ['search', 'OBSIDIANNEEDLE']}, {'wait': 1500},
         {'sidebar': ['dump', str(out / 'matches.json')]}, {'snapshot': str(out / 'search.png')},
         {'sidebar': ['search', 'absent-very-unique-keyword']}, {'wait': 1000},
         {'sidebar': ['dump', str(out / 'empty.json')]},
         {'sidebar': ['search', '']}, {'wait': 500}]
if args.navigation:
    steps += [{'server': 'fixture-failed'}, {'wait': 200}, {'click_text': 'Rally'}, {'wait_rally': {'rows': 1, 'tabs': 1, 'timeout_ms': 15000}},
              {'rally': ['item', rally.story['_ref']]},
              {'wait_rally': {'detail': True, 'timeout_ms': 15000}},
              {'rally': ['edit', 'Name', 'Keep this unsaved draft']},
              {'rally': ['open', 'tasks']}, {'wait_rally': {'rows': 1, 'tabs': 2, 'timeout_ms': 15000}},
              {'sidebar': ['dump', str(out / 'rally-two.json')]}, {'snapshot': str(out / 'rally.png')},
              {'tab_action': 'close-others'}, {'wait': 200},
              {'sidebar': ['dump', str(out / 'rally-bulk-blocked.json')]},
              {'select_tab': 1}, {'wait': 100},
              {'rally': ['dump', str(out / 'rally-kept-draft.json')]},
              {'rally': ['action', 'detail-back']}, {'rally': ['dirty-choice', 'discard']},
              {'select_tab': 2}, {'wait': 100},
              {'tab_action': 'close-others'}, {'wait': 200},
              {'sidebar': ['dump', str(out / 'rally-bulk-closed.json')]},
              {'rally': ['open', 'teamboard']}, {'wait_rally': {'rows': 1, 'tabs': 2, 'timeout_ms': 15000}},
              {'sidebar': ['dump', str(out / 'rally-two-final.json')]},
              {'select_tab': 0}, {'wait': 300},
              {'sidebar': ['dump', str(out / 'conversation-nav.json')]},
              {'snapshot': str(out / 'conversation.png')},
              {'click_text': 'Rally'}, {'wait': 300},
              {'sidebar': ['dump', str(out / 'rally-return.json')]},
              {'rally': ['close']}, {'wait': 100}, {'rally': ['close']}, {'wait': 100},
              {'sidebar': ['dump', str(out / 'rally-closed.json')]},
              {'click_text': 'Rally'}, {'wait_rally': {'rows': 1, 'tabs': 1, 'timeout_ms': 15000}},
              {'sidebar': ['dump', str(out / 'rally-reopened.json')]},
              {'rally': ['close']}, {'wait': 100}, {'server': 'fixture-ready'}]
if args.windows:
    child_steps = [{'wait_ready': 30000}, {'resize': [1200, 850]},
                   {'wait_rally': {'detail': True, 'tabs': 1, 'timeout_ms': 15000}},
                   {'wait': 1500}, {'rally': ['dump', str(out / 'child.json')]},
                   {'snapshot': str(out / 'child.png')},
                   {'new_thread': str(out / 'project')}, {'wait_idle': 30000},
                   {'send': 'Please answer from the second native window'},
                   {'wait_idle': 30000}, {'wait': 2000},
                   {'transcript': 'export-complete:' + str(out / 'child.md')},
                   {'sidebar': ['search', 'OBSIDIANNEEDLE']}, {'wait': 1500},
                   {'sidebar': ['dump', str(out / 'child-search.json')]},
                   {'select_tab': 0}, {'wait': 300}, {'rally': ['transfer-first']},
                   {'wait_rally': {'tabs': 0, 'timeout_ms': 15000}}]
    (out / 'child-script.json').write_text(json.dumps(child_steps), encoding='utf-8')
    steps += [{'click_text': 'Rally'}, {'wait_rally': {'rows': 1, 'tabs': 1, 'timeout_ms': 15000}},
              {'rally': ['item', rally.story['_ref']]},
              {'wait_rally': {'detail': True, 'timeout_ms': 15000}},
              {'rally': ['edit', 'Name', 'Draft across native windows']},
              {'rally': ['set', 'filter-value', 'Unapplied window filter']}, {'wait': 1500},
              {'click_text': 'New window'}, {'wait_rally': {'tabs': 0, 'timeout_ms': 15000}},
              {'wait_rally': {'detail': True, 'tabs': 1, 'timeout_ms': 30000}},
              {'rally': ['dump', str(out / 'returned.json')]},
              {'snapshot': str(out / 'returned.png')},
              {'rally': ['action', 'detail-back']}, {'rally': ['dirty-choice', 'discard']},
              {'rally': ['close']}, {'wait': 500}]
steps += [{'open_settings': 'goose'}, {'wait': 1000}, {'snapshot': str(out / 'settings.png')},
         {'click_text': 'Save cheap profile'}, {'wait': 500}, {'quit': True}]
if args.tray:
    steps[-1:] = [{'sidebar': ['dump', str(out / 'tray-finish.json')]}, {'wait': 60000}]
(out / 'script.json').write_text(json.dumps(steps), encoding='utf-8')
env = dict(os.environ)
for key in ('AWS_PROFILE', 'AWS_SESSION_TOKEN', 'AWS_BEARER_TOKEN_BEDROCK', 'GOOSE_PROVIDER',
            'GOOSE_MODEL', 'FASTROCK_GOOSE_CONFIG', 'FASTROCK_GOOSE', 'FASTROCK_GOOSE_FIXTURE',
            'FASTROCK_FIXTURE_ARTIFACT', 'FASTROCK_RALLY_TOKEN', 'FASTROCK_RALLY_CHILD_AUTOMATION'):
    env.pop(key, None)
env.update(FASTROCK_HOME=str(out / 'home'), CODEX_HOME=str(out / 'codex'),
           GOOSE_PATH_ROOT=str(out / 'goose'), GOOSE_DISABLE_KEYRING='1',
           LOCALAPPDATA=str(out / 'local'), APPDATA=str(out / 'roaming'),
           AWS_CONFIG_FILE=str(out / 'aws-empty'), AWS_SHARED_CREDENTIALS_FILE=str(out / 'aws-empty'),
           AWS_REGION='us-east-1', AWS_ACCESS_KEY_ID='fixture-access', AWS_SECRET_ACCESS_KEY='fixture-secret',
           AWS_EC2_METADATA_DISABLED='true', AWS_ENDPOINT_URL_BEDROCK_RUNTIME=endpoint,
           AWS_ENDPOINT_URL_BEDROCK=endpoint, AWS_ENDPOINT_URL=endpoint,
           CODEX_GUI_AUTOMATION=str(out / 'script.json'), SLINT_BACKEND='winit-software')

if args.windows:
    env['FASTROCK_RALLY_CHILD_AUTOMATION'] = str(out / 'child-script.json')

if rally is not None:
    env['FASTROCK_RALLY_TOKEN'] = fixture_module.TOKEN


class Memory(ctypes.Structure):
    _fields_ = [('cb', wintypes.DWORD), ('PageFaultCount', wintypes.DWORD)] + [
        (name, ctypes.c_size_t) for name in ('PeakWorkingSetSize', 'WorkingSetSize', 'QuotaPeakPagedPoolUsage',
        'QuotaPagedPoolUsage', 'QuotaPeakNonPagedPoolUsage', 'QuotaNonPagedPoolUsage',
        'PagefileUsage', 'PeakPagefileUsage', 'PrivateUsage')]


memory_info = ctypes.WinDLL('psapi').GetProcessMemoryInfo
memory_info.argtypes = [wintypes.HANDLE, ctypes.POINTER(Memory), wintypes.DWORD]
memory_info.restype = wintypes.BOOL
peak = 0
peak_windows = 0
owners_seen = {}
user32 = ctypes.WinDLL('user32')
window_callback = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
user32.EnumWindows.argtypes = [window_callback, wintypes.LPARAM]
user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
user32.IsWindowVisible.argtypes = [wintypes.HWND]

def native_windows(pid):
    windows = []
    @window_callback
    def collect(hwnd, _):
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        name = ctypes.create_unicode_buffer(80)
        user32.GetClassNameW(hwnd, name, len(name))
        # Winit marks its zero-size, transparent event target WS_VISIBLE.
        # Closing it destroys delivery of backend callbacks to the UI.
        if owner.value == pid and user32.IsWindowVisible(hwnd) and name.value != 'Winit Thread Event Target':
            windows.append(hwnd)
        return True
    user32.EnumWindows(collect, 0)
    return windows

def visible_windows(pid):
    return len(native_windows(pid))

user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
user32.FindWindowExW.argtypes = [wintypes.HWND, wintypes.HWND, wintypes.LPCWSTR, wintypes.LPCWSTR]
user32.FindWindowExW.restype = wintypes.HWND
user32.GetClassNameW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
user32.GetForegroundWindow.restype = wintypes.HWND
user32.SetForegroundWindow.argtypes = [wintypes.HWND]

def tray_window(pid):
    after = None
    while True:
        hwnd = user32.FindWindowExW(wintypes.HWND(-3), after, 'SlintSystemTrayWindow', None)
        if not hwnd:
            return None
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid:
            return hwnd
        after = hwnd

def is_menu(hwnd):
    name = ctypes.create_unicode_buffer(80)
    user32.GetClassNameW(hwnd, name, len(name))
    return name.value == '#32768'


def activate_quit_menu(hwnd, pid):
    """Invoke the actual native menu item, scoped to this fixture's process.

    Windows' modal menu loop does not reliably accept synthetic key events.
    Use the same accessibility action as the existing native Copy fixture.
    """
    import uuid
    class GUIThreadInfo(ctypes.Structure):
        _fields_ = [('size', wintypes.DWORD), ('flags', wintypes.DWORD)] + [
            (name, wintypes.HWND) for name in ('active', 'focus', 'capture', 'menu_owner', 'move_size', 'caret')
        ] + [('caret_rect', wintypes.RECT)]
    class VariantValue(ctypes.Union):
        _fields_ = [('integer', wintypes.LONG), ('pointer', ctypes.c_void_p), ('record', ctypes.c_byte * 16)]
    class Variant(ctypes.Structure):
        _fields_ = [('kind', wintypes.WORD), ('r1', wintypes.WORD), ('r2', wintypes.WORD), ('r3', wintypes.WORD), ('value', VariantValue)]
    user32.SendMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
    user32.SendMessageW.restype = wintypes.HMENU
    user32.GetMenuItemCount.argtypes = [wintypes.HMENU]
    user32.GetMenuStringW.argtypes = [wintypes.HMENU, wintypes.UINT, wintypes.LPWSTR, ctypes.c_int, wintypes.UINT]
    user32.GetMenuState.argtypes = [wintypes.HMENU, wintypes.UINT, wintypes.UINT]
    user32.GetGUIThreadInfo.argtypes = [wintypes.DWORD, ctypes.POINTER(GUIThreadInfo)]
    owner = wintypes.DWORD()
    thread = user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
    assert owner.value == pid, 'Refusing another process menu'
    info = GUIThreadInfo(size=ctypes.sizeof(GUIThreadInfo))
    assert user32.GetGUIThreadInfo(thread, ctypes.byref(info))
    user32.GetWindowThreadProcessId(info.menu_owner, ctypes.byref(owner))
    assert owner.value == pid, 'Native menu lost fixture ownership'
    menu = user32.SendMessageW(hwnd, 0x01E1, 0, 0)
    labels = []
    for index in range(user32.GetMenuItemCount(menu)):
        label = ctypes.create_unicode_buffer(256)
        user32.GetMenuStringW(menu, index, label, len(label), 0x400)
        labels.append(label.value.replace('&', ''))
    index = labels.index('Quit Fastrock')
    assert user32.GetMenuState(menu, index, 0x400) & 3 == 0, 'Quit is disabled'
    ole32, oleacc = ctypes.WinDLL('ole32'), ctypes.WinDLL('oleacc')
    oleacc.AccessibleObjectFromWindow.argtypes = [wintypes.HWND, wintypes.DWORD, ctypes.c_void_p, ctypes.POINTER(ctypes.c_void_p)]
    oleacc.AccessibleObjectFromWindow.restype = ctypes.HRESULT
    initialized = ole32.CoInitializeEx(None, 2) >= 0
    interface = ctypes.c_void_p()
    iid = (ctypes.c_byte * 16).from_buffer_copy(uuid.UUID('618736e0-3c3d-11cf-810c-00aa00389b71').bytes_le)
    try:
        assert oleacc.AccessibleObjectFromWindow(hwnd, 0xFFFFFFFC, ctypes.byref(iid), ctypes.byref(interface)) >= 0
        table = ctypes.cast(interface, ctypes.POINTER(ctypes.POINTER(ctypes.c_void_p))).contents
        invoke = ctypes.WINFUNCTYPE(ctypes.HRESULT, ctypes.c_void_p, Variant)(table[25])
        try:
            assert invoke(interface, Variant(kind=3, value=VariantValue(integer=index + 1))) >= 0
        finally:
            ctypes.WINFUNCTYPE(wintypes.ULONG, ctypes.c_void_p)(table[2])(interface)
    finally:
        if initialized:
            ole32.CoUninitialize()
    return labels


tray_state = dict(close_requested=False, turn_completed_hidden=False, restore_requested=False,
                 restored=False, quit_menu_requested=False, quit_selected=False)
menu_driver = None
menu_errors = []
menu_labels = []
observed_recovery_pause = False

def quit_from_menu(hwnd, pid):
    try:
        menu_labels.extend(activate_quit_menu(hwnd, pid))
        tray_state['quit_selected'] = True
    except Exception as error:
        menu_errors.append(repr(error))

with (out / 'app.log').open('w', encoding='utf-8') as log:
    process = subprocess.Popen([str(args.binary.resolve()), '--remote', 'goose'], cwd=out,
                               env=env, stdout=log, stderr=log)
    try:
        deadline = time.monotonic() + 90
        while process.poll() is None:
            memory = Memory()
            memory.cb = ctypes.sizeof(memory)
            if not memory_info(int(process._handle), ctypes.byref(memory), memory.cb):
                if process.poll() is not None:
                    break
                raise OSError('Could not monitor fixture memory')
            peak = max(peak, memory.PeakWorkingSetSize, memory.PeakPagefileUsage, memory.PrivateUsage)
            # This one-conversation smoke needs far less than the full 32 GB
            # benchmark allowance; leave additional headroom on the VM host.
            if peak >= 2_000_000_000 or time.monotonic() > deadline:
                raise RuntimeError('Fixture memory/time limit exceeded')
            if args.recovery and not observed_recovery_pause and (out / 'recovery-paused.json').exists():
                assert len(requests) == 1, f'Work ran before Resume: {requests}'
                observed_recovery_pause = True
            if args.tray:
                windows = native_windows(process.pid)
                tray = tray_window(process.pid)
                if requests and not tray_state['close_requested'] and tray:
                    for hwnd in windows:
                        assert user32.PostMessageW(hwnd, 0x10, 0, 0)  # real WM_CLOSE
                    tray_state['close_requested'] = True
                if tray_state['close_requested'] and not windows and not tray_state['restore_requested']:
                    release_first_turn.set()
                    if any(request['summary'] for request in requests):
                        tray_state['turn_completed_hidden'] = True
                        # The same Shell_NotifyIcon callback Windows sends on a left click.
                        assert user32.PostMessageW(tray, 0x8001, 1, 0x202)
                        tray_state['restore_requested'] = True
                if tray_state['restore_requested'] and windows:
                    tray_state['restored'] = True
                if tray_state['restored'] and (out / 'tray-finish.json').exists() and not tray_state['quit_menu_requested']:
                    user32.SetForegroundWindow(windows[0])
                    assert user32.PostMessageW(tray, 0x8001, 1, 0x205)  # native tray context menu
                    tray_state['quit_menu_requested'] = True
                menus = [hwnd for hwnd in windows if is_menu(hwnd)]
                if tray_state['quit_menu_requested'] and menus and menu_driver is None:
                    menu_driver = threading.Thread(target=quit_from_menu, args=(menus[0], process.pid), daemon=True)
                    menu_driver.start()
                assert not menu_errors, menu_errors
            if args.windows:
                peak_windows = max(peak_windows, visible_windows(process.pid))
                for path in (out / 'home/windows').glob('*/owner.json'):
                    try:
                        owner = json.loads(path.read_text())
                        owners_seen[owner['id']] = owner['pid']
                    except (OSError, ValueError):
                        pass  # An atomic retirement may race this read.
            time.sleep(0.1)
        assert process.returncode == 0, f'App exited {process.returncode}; see app.log'
    finally:
        if menu_driver is not None:
            menu_driver.join(timeout=2)
        if args.tray:
            (out / 'tray-diagnostics.json').write_text(json.dumps(dict(tray_state=tray_state, provider_requests=requests, peak_process_bytes=peak), indent=2), encoding='utf-8')
        if process.poll() is None:
            process.kill()
            process.wait(timeout=10)
        release_first_turn.set()
        server.shutdown()
        if rally is not None:
            rally.server.shutdown()

matches = json.loads((out / 'matches.json').read_text())['rows']
assert len(matches) == 1 and matches[0]['id'].startswith('goose:'), matches
assert json.loads((out / 'empty.json').read_text())['rows'] == []
expected_requests = 3 if args.recovery else 2 * (args.turns + int(args.windows))
assert len(requests) == expected_requests, f'Unexpected model calls: {requests}'
paired_requests = requests[1:] if args.recovery else requests
for ordinary, summary in zip(paired_requests[::2], paired_requests[1::2]):
    assert ordinary['summary'] is False and 'openai.gpt-6.1-sol' in ordinary['path']
    assert summary['summary'] is True and 'openai.gpt-6-luna' in summary['path']
transcript = (out / 'complete.md').read_text(encoding='utf-8')
if args.recovery:
    assert observed_recovery_pause
    visible = (out / 'recovery-visible.md').read_text(encoding='utf-8')
    assert visible.count('Uncertain send: retain me for review') == 1, visible
    records = json.loads((out / 'recovery-records.json').read_text(encoding='utf-8'))
    assert len(records) == 1 and records[0]['input'][0]['text'] == 'Uncertain send: retain me for review', records
    assert transcript.count('Queued recovery message é') == 1, transcript
    assert transcript.count('Native embedded Goose answer 2,') == 1, transcript
    assert 'Native embedded Goose answer 1,' not in transcript, transcript
    states = []
    for path in (out / 'home').rglob('*.json'):
        value = json.loads(path.read_text(encoding='utf-8'))
        if isinstance(value, dict) and value.get('version') == 1 and 'uncertain' in value:
            states.append(value)
    assert len(states) == 1, states
    assert states[0]['pending'] == [] and states[0]['dispatch'] is None, states
    # Explicit Quit closes the session. Unreviewed evidence remains paused
    # for the next launch, even though pending work completed after Resume.
    assert len(states[0]['uncertain']) == 1 and states[0]['paused'] is True, states
else:
    for turn in range(1, args.turns + 1):
        assert transcript.count(f'Please give a short fixture answer {turn}') == 1, transcript
        assert transcript.count(f'Native embedded Goose answer {turn},') == 1, transcript
assert 'ObsidianNeedle' not in transcript, 'Search summary leaked into the source history'
saved = (out / 'goose/config/config.yaml').read_text(encoding='utf-8')
assert 'FASTROCK_CHEAP_PROFILE:' in saved and 'Bedrock Luna' in saved
assert (out / 'local/vwork/workspaces.redb').exists()
if args.navigation:
    navigation = {name: json.loads((out / (name + '.json')).read_text())['navigation']
                  for name in ('rally-two', 'rally-bulk-blocked', 'rally-bulk-closed', 'rally-two-final', 'conversation-nav', 'rally-return', 'rally-closed', 'rally-reopened')}
    assert 'server failed \"Fixture backend unavailable\"' in (out / 'app.log').read_text(encoding='utf-8')
    assert navigation['rally-two']['rallySelected'] is True, navigation
    assert navigation['conversation-nav']['rallySelected'] is False, navigation
    assert navigation['rally-bulk-blocked']['rallyCount'] == 2, navigation
    draft = json.loads((out / 'rally-kept-draft.json').read_text())
    assert draft['dirty'] and any(field['name'] == 'Name' and field['value'] == 'Keep this unsaved draft'
                                  for field in draft['fields']), draft
    assert navigation['rally-bulk-closed']['rallyCount'] == 1, navigation
    assert navigation['rally-two']['rallyCount'] == 2, navigation
    assert navigation['rally-two']['rallyResidentModels'][3:6] == [0, 0, 0], navigation['rally-two']
    assert navigation['rally-two']['documentKinds'] == ['Rally', 'Rally'], navigation
    assert navigation['conversation-nav']['activeKind'] == 'Thread', navigation
    assert navigation['conversation-nav']['documentKinds'] == [], navigation
    assert navigation['rally-return']['activeId'] == navigation['rally-two-final']['activeId'], navigation
    assert navigation['rally-closed']['rallyCount'] == 0, navigation
    assert all(size == 0 for size in navigation['rally-closed']['rallyResidentModels']), navigation['rally-closed']
    assert navigation['rally-closed']['rallyAssistantBytes'] == 0, navigation['rally-closed']
    assert navigation['rally-reopened']['rallyCount'] == 1, navigation
    assert navigation['rally-reopened']['activeKind'] == 'Rally', navigation
    assert all(request['method'] == 'GET' for request in rally.requests), rally.requests
if args.windows:
    child = json.loads((out / 'child.json').read_text())
    returned = json.loads((out / 'returned.json').read_text())
    for document in (child, returned):
        assert document['dirty'], document
        assert document['filterDraft'] == 'Unapplied window filter', document
        assert any(field['name'] == 'Name' and field['value'] == 'Draft across native windows'
                   for field in document['fields']), document
    child_transcript = (out / 'child.md').read_text(encoding='utf-8')
    assert child_transcript.count('Please answer from the second native window') == 1, child_transcript
    assert f'Native embedded Goose answer {args.turns + 1},' in child_transcript, child_transcript
    assert 'Please give a short fixture answer' not in child_transcript, child_transcript
    child_hits = json.loads((out / 'child-search.json').read_text())['rows']
    assert len(child_hits) == 2, child_hits
    assert peak_windows >= 2 and len(owners_seen) == 2, (peak_windows, owners_seen)
    assert set(owners_seen.values()) == {process.pid}, owners_seen
if args.tray:
    assert all(tray_state.values()), tray_state
    assert (out / 'codex/gui.json').exists(), 'Explicit Quit did not flush UI preferences'
receipt = dict(result='PASS', binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
               explicit_recovery_resume=args.recovery, uncertain_send_never_replayed=args.recovery,
               peak_process_bytes=peak, requests=requests, summary_only_match=True,
               exported_turns=args.turns, complete_history_preserved=True,
               native_cheap_profile_saved=True, all_provider_requests_local=True,
               independent_rally_navigation=args.navigation, backend_overlay_navigation=args.navigation,
               in_process_windows=args.windows, peak_native_windows=peak_windows, window_owner_pids=owners_seen, tray_lifecycle=tray_state if args.tray else None)
(out / 'acceptance.json').write_text(json.dumps(receipt, indent=2) + '\n', encoding='utf-8')
print(json.dumps(receipt, indent=2), flush=True)
