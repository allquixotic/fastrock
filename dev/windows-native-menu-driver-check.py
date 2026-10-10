#!/usr/bin/env python3
"""Windows-only V9: verify the native-menu driver without a compiled GUI."""
import ast, ctypes, os, sys, threading, time
from ctypes import wintypes
from pathlib import Path
assert sys.platform == 'win32'
source_path = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).with_name('windows-purpose-selection-smoke.py')
source = ast.parse(source_path.read_text())
function = next(node for node in source.body if isinstance(node, ast.FunctionDef) and node.name == 'drive_copy_menus')
exec(compile(ast.Module(body=[function], type_ignores=[]), '<native-menu-driver>', 'exec'))
user32 = ctypes.windll.user32
kernel32 = ctypes.windll.kernel32
kernel32.GetModuleHandleW.restype = wintypes.HINSTANCE
user32.CreateWindowExW.restype = wintypes.HWND
user32.CreateWindowExW.argtypes = [wintypes.DWORD, wintypes.LPCWSTR, wintypes.LPCWSTR, wintypes.DWORD, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int, wintypes.HWND, wintypes.HMENU, wintypes.HINSTANCE, ctypes.c_void_p]
user32.CreatePopupMenu.restype = wintypes.HMENU
user32.AppendMenuW.argtypes = [wintypes.HMENU, wintypes.UINT, wintypes.WPARAM, wintypes.LPCWSTR]
user32.TrackPopupMenu.argtypes = [wintypes.HMENU, wintypes.UINT, ctypes.c_int, ctypes.c_int, ctypes.c_int, wintypes.HWND, ctypes.c_void_p]
user32.SetForegroundWindow.argtypes = [wintypes.HWND]
user32.DestroyWindow.argtypes = [wintypes.HWND]
user32.DestroyMenu.argtypes = [wintypes.HMENU]
user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
hwnd = user32.CreateWindowExW(0, 'STATIC', 'Codex GUI native-menu test', 0x10cf0000, 50, 50, 320, 180, None, None, kernel32.GetModuleHandleW(None), None)
assert hwnd, ctypes.GetLastError()
menu = user32.CreatePopupMenu()
user32.AppendMenuW(menu, 0, 11, 'Copy')
user32.AppendMenuW(menu, 0, 12, 'Select All')
class Process:
    pid = os.getpid()
    exited = False
    def poll(self): return 0 if self.exited else None
process = Process()
evidence, errors = [], []
driver = threading.Thread(target=drive_copy_menus, args=(process, evidence, errors), daemon=True)
driver.start()
watchdog = threading.Timer(10, lambda: user32.PostMessageW(hwnd, 0x1f, 0, 0))
watchdog.daemon = True
watchdog.start()
try:
    user32.SetForegroundWindow(hwnd)
    result = user32.TrackPopupMenu(menu, 0x182, 80, 90, 0, hwnd, None)
    assert result == 11, (result, evidence, errors)
    assert len(evidence) == 1 and not errors, (evidence, errors)
    print('Native Copy menu test driver passed: real popup, ownership, enabled item, clipboard clearing, accessibility activation.')
finally:
    process.exited = True
    watchdog.cancel()
    driver.join(timeout=2)
    user32.DestroyMenu(menu)
    user32.DestroyWindow(hwnd)
