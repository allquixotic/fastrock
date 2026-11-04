#!/usr/bin/env python3
"""V94: old history opens with a locked store; relaunch restores its tray owner."""
import argparse, ctypes, hashlib, json, msvcrt, os, subprocess, time
from ctypes import wintypes
from pathlib import Path
from embedded_provider_fixture import ProviderFixture, ProcessGuard

assert os.name == 'nt'
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('binary', type=Path)
p.add_argument('--output', type=Path, required=True)
a = p.parse_args()
binary, out = a.binary.resolve(), a.output.resolve()
out.mkdir(parents=True, exist_ok=False)
provider = ProviderFixture()
env = provider.environment(out)
config_path = out/'goose/config/config.yaml'
config = json.loads(config_path.read_text())
home = out/'local/vwork'
config['extensions']['vwork'] = dict(enabled=True, name='vwork', type='stdio',
    cmd=str(out/'not-installed/vwork.exe'), args=['--home',str(home),'mcp'], timeout=30)
config_path.write_text(json.dumps(config), encoding='utf-8')

def start(name, steps, *args):
    script = out/(name+'-steps.json')
    script.write_text(json.dumps(steps), encoding='utf-8')
    log = (out/(name+'.log')).open('w', encoding='utf-8')
    process = subprocess.Popen([str(binary), *args], env=dict(env,CODEX_GUI_AUTOMATION=str(script)), stdout=log, stderr=log)
    log.close()
    return process

user32 = ctypes.WinDLL('user32', use_last_error=True)
callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
user32.EnumWindows.argtypes = [callback_type, wintypes.LPARAM]
user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
user32.IsWindowVisible.argtypes = [wintypes.HWND]
user32.GetWindowTextLengthW.argtypes = [wintypes.HWND]
user32.PostMessageW.argtypes = [wintypes.HWND,wintypes.UINT,wintypes.WPARAM,wintypes.LPARAM]
def windows(pid):
    found = []
    @callback_type
    def collect(hwnd, _):
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid and user32.IsWindowVisible(hwnd) and user32.GetWindowTextLengthW(hwnd):
            found.append(hwnd)
        return True
    user32.EnumWindows(collect, 0)
    return found

try:
    prepare = start('prepare', [{'wait_ready':30000},{'new_thread':str(out/'project')},
        {'wait_idle':30000},{'send':'Remember the old history fixture'}, {'wait_idle':30000},
        {'wait_summaries':30000},{'wait':1000},{'quit':True}])
    peak = ProcessGuard(prepare, timeout=90).wait()
    db = home/'workspaces.fjall/lock'
    assert db.exists(), 'Preparation did not create a database'
    # Real byte-range lock, as held by another Fjall owner. Never user data.
    with db.open('r+b') as locked:
        msvcrt.locking(locked.fileno(), msvcrt.LK_NBLCK, 1)
        history = out/'restored.md'
        owner = start('restore', [{'wait_ready':30000},{'wait':1500},{'wait_idle':30000},
            {'transcript':'export-complete:'+str(history)}, {'wait':15000},
            {'click_text':'Resume'}, {'wait':1000}, {'wait_idle':30000},
            {'transcript':'export-complete:'+str(out/'resumed.md')}, {'wait':750}, {'quit':True}])
        guard = ProcessGuard(owner, timeout=90)
        secondary = None
        try:
            until = time.monotonic()+35
            while not history.exists() and time.monotonic()<until:
                assert owner.poll() is None, 'Owner exited during restoration'
                guard.sample(); time.sleep(.1)
            assert history.exists(), 'Old thread did not load with locked workspace storage'
            assert 'Remember the old history fixture' in history.read_text(encoding='utf-8')
            visible = windows(owner.pid)
            assert len(visible)==1, visible
            hwnd = visible[0]
            user32.PostMessageW(hwnd, 0x0010, 0, 0)  # WM_CLOSE -> tray
            until = time.monotonic()+5
            while user32.IsWindowVisible(hwnd) and time.monotonic()<until: time.sleep(.05)
            assert owner.poll() is None and not user32.IsWindowVisible(hwnd), 'Close did not preserve tray owner'
            started = time.monotonic()
            secondary = start('relaunch', [{'wait_ready':30000},{'wait':30000},{'quit':True}])
            assert secondary.wait(timeout=12)==0, 'Relaunch did not forward to owner'
            elapsed = time.monotonic()-started
            until = time.monotonic()+5
            while not user32.IsWindowVisible(hwnd) and time.monotonic()<until: time.sleep(.05)
            assert user32.IsWindowVisible(hwnd) and owner.poll() is None, 'Relaunch failed to restore original window'
            msvcrt.locking(locked.fileno(), msvcrt.LK_UNLCK, 1)
            locked.close()
            peak = max(peak,guard.wait())
            resumed = (out/'resumed.md').read_text(encoding='utf-8')
            assert 'Remember the old history fixture' in resumed and 'recovery could not be checked' not in resumed, resumed
        finally:
            for process in (secondary,owner):
                if process and process.poll() is None: process.kill(); process.wait(timeout=10)
            if not locked.closed: msvcrt.locking(locked.fileno(), msvcrt.LK_UNLCK, 1)
    # Explicit Quit releases profile ownership; a subsequent real startup runs.
    after = out/'after.md'
    reopened = start('after-quit', [{'wait_ready':30000},{'wait':1000},{'wait_idle':30000},
        {'transcript':'export-complete:'+str(after)}, {'wait':750}, {'quit':True}])
    peak = max(peak,ProcessGuard(reopened,timeout=60).wait())
    assert 'Remember the old history fixture' in after.read_text(encoding='utf-8')
    assert len([r for r in provider.requests if not r['summary']]) == 1, 'History restoration replayed inference'
    (out/'summary.json').write_text(json.dumps(dict(result='PASS', peak_bytes=peak,
        binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), relaunch_seconds=elapsed,
        checks=['old thread while store locked','close to tray','relaunch restores original window',
                'secondary process exits','Resume rechecks storage and restores normal history',
                'explicit quit releases ownership']),indent=2), encoding='utf-8')
finally:
    provider.close()
print('PASS: old history and tray relaunch ownership',flush=True)
