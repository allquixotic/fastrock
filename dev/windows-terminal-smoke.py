#!/usr/bin/env python3
"""Native terminal PTY, input, history, workspace import and owned process teardown."""
import argparse, ctypes, hashlib, json, os, subprocess
from pathlib import Path
from ctypes import wintypes
from embedded_provider_fixture import ProviderFixture, ProcessGuard
assert os.name == 'nt', 'Run GUI acceptance on games only'
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('binary',type=Path);p.add_argument('--output',type=Path,required=True)
a=p.parse_args();binary=a.binary.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
repo=out/'project';provider=ProviderFixture(workspace_repo=repo,stream_chunks=9000);env=provider.environment(out,streaming=True)
env.update(FASTROCK_WEBHOOK_BIND='127.0.0.1:0',PYTHONIOENCODING='utf-8')
config=out/'goose/config/config.yaml';value=json.loads(config.read_text(encoding='utf-8'))
value['extensions']={'vwork':dict(enabled=True,name='vwork',type='stdio',cmd=str(out/'absent/vwork.exe'),args=['--home',str(out/'local/vwork'),'mcp'],timeout=30)}
config.write_text(json.dumps(value),encoding='utf-8')
(repo/'src').mkdir();original=b'original terminal source\r\n';(repo/'src/main.rs').write_bytes(original)
git_env={k:v for k,v in os.environ.items() if not k.upper().startswith('GIT_')};git_env.update(GIT_CONFIG_NOSYSTEM='1',GIT_CONFIG_GLOBAL=str(out/'aws-empty'))
for command in (['init','.'],['add','src/main.rs'],['-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','Fixture']):
    subprocess.run(['git','-c','core.hooksPath=NUL','-c','core.autocrlf=false',*command],cwd=repo,env=git_env,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,check=True)
helper=out/'terminal_helper.py'
helper.write_text('''import os,sys,time,subprocess\nfrom pathlib import Path\nmode=sys.argv[1]\nif mode=='edit':\n Path('src/main.rs').write_bytes(b'terminal imported\\r\\n')\n Path('unimported.txt').write_bytes(b'keep recovery\\r\\n')\n print('\\x1b[32mTERMINAL_EDIT_DONE\\x1b[0m',flush=True)\nelif mode=='history':\n for i in range(150): print(f'HISTORY_{i:03d}')\n print('\\x1b[31mCOLOR_RED\\x1b[0m wide \\u754c',flush=True)\n deadline=time.monotonic()+8\n while not Path(sys.argv[2]).exists() and time.monotonic()<deadline: time.sleep(.03)\n for i in range(20): print(f'LATER_{i:03d}',flush=True)\nelif mode=='child':\n child=subprocess.Popen([sys.executable,'-c','import time;time.sleep(120)'])\n Path(sys.argv[2]).write_text(str(child.pid))\n print('OWNED_CHILD_STARTED',flush=True)\n time.sleep(120)\n''',encoding='utf-8')
def term(*args):return {'terminal':list(args)}
def dump(name):return [term('dump',str(out/(name+'.json'))),{'wait':200}]
def read(name):return json.loads((out/(name+'.json')).read_text(encoding='utf-8'))
def command(mode,*args):return term('input','"'+os.sys.executable+'" "'+str(helper)+'" '+mode+' '+ ' '.join('"'+str(v)+'"' for v in args)+'\r')
def virtual(*args):return {'virtual_workspaces':list(args)}
steps=[{'wait_ready':30000},{'resize':[1440,1000]},{'new_thread':str(repo)},{'wait_idle':30000},
 {'send':'vwork-seed'},{'wait':300},{'wait_idle':30000},{'wait':300},virtual('action','virtual-refresh'),{'wait':800},
 virtual('action','virtual-catalog-filter:/ task'),virtual('select','0'),{'wait':400},virtual('action','virtual-chat'),{'wait':800},{'wait_idle':30000},
 {'composer':['type','Terminal preserves my unsent draft']},{'click_text':'Terminal'},{'wait':1500},*dump('opened'),
 term('input','chcp 65001 >nul\r'),term('clipboard','echo PASTED_NATIVE\r'),{'click_text':'Paste'},{'wait':400},*dump('pasted'),command('history',out/'scrolled.json'),{'wait':900},*dump('history'),
 {'snapshot':str(out/'terminal.png')},{'click_text':'Copy'},{'wait':200},{'selection':['capture',str(out/'clipboard.txt')]},term('scroll','20'),{'wait':200},*dump('scrolled'),{'wait':400},*dump('anchored'),term('scroll','-5000'),{'wait':200},
 {'key':'e'},{'key':'c'},{'key':'h'},{'key':'o'},{'key':' '},{'key':'N'},{'key':'A'},{'key':'T'},{'key':'I'},{'key':'V'},{'key':'E'},{'key':'return'},{'wait':400},*dump('typed'),
 {'send':'hold terminal escape fixture'},{'wait':500},term('input','echo PRESERVE_STREAM\r'),{'wait':300},{'key':'ctrl+shift+P'},{'wait':250},{'key':'escape'},{'wait':250},*dump('before-escape'),{'key':'escape'},{'wait':300},{'palette':['dump',str(out/'escape-stream.json')]},{'wait':150},{'shortcut':'escape'},{'wait_idle':10000},term('resize','8','62'),{'wait':300},*dump('resized'),term('resize','12','100'),{'wait':300},
 command('edit'),{'wait':700},*dump('edited'),term('paths','src/main.rs'),{'click_text':'Import paths and close'},{'wait':1500},*dump('imported'),term('scroll','20'),{'wait':300},*dump('closed-scroll'),
 {'palette':['dump',str(out/'draft.json')]},{'wait':200},term('close'),{'click_text':'Terminal'},{'wait':1300},
 command('child',out/'interrupt-child.pid'),{'wait':700},{'key':'ctrl+c'},{'wait':500},term('input','echo AFTER_CTRL_C\r'),{'wait':500},*dump('interrupted'),command('child',out/'child.pid'),{'wait':900},*dump('child'),{'click_text':'Close'},{'wait':1500},*dump('closed'),
 virtual('action','virtual-refresh'),{'wait':500},virtual('action','virtual-catalog-filter:/ task'),virtual('select','0'),{'wait':300},virtual('file','0'),{'wait':400},virtual('dump',str(out/'workspace.json')),{'wait':200},
 term('close'),{'click_text':'Terminal'},{'wait':1300},command('child',out/'chat-child.pid'),{'wait':700},*dump('chat-child'),{'shortcut':'close-tab'},{'wait':1000},virtual('action','virtual-chat'),{'wait':700},{'wait_idle':30000},{'click_text':'Terminal'},{'wait':1300},command('child',out/'quit-child.pid'),{'wait':700},*dump('quit-child'),{'quit':True}]
script=out/'steps.json';script.write_text(json.dumps(steps),encoding='utf-8')
def alive(pid):
    kernel=ctypes.WinDLL('kernel32',use_last_error=True);kernel.OpenProcess.restype=ctypes.c_void_p;kernel.OpenProcess.argtypes=[ctypes.c_ulong,ctypes.c_int,ctypes.c_ulong]
    handle=kernel.OpenProcess(0x1000|0x100000,False,int(pid))
    if not handle:return False
    try:
        kernel.WaitForSingleObject.argtypes=[ctypes.c_void_p,ctypes.c_ulong]
        return kernel.WaitForSingleObject(handle,0)==258
    finally:kernel.CloseHandle.argtypes=[ctypes.c_void_p];kernel.CloseHandle(handle)
def foreground(process):
    user=ctypes.WinDLL('user32',use_last_error=True)
    callback=ctypes.WINFUNCTYPE(wintypes.BOOL,wintypes.HWND,wintypes.LPARAM)
    user.EnumWindows.argtypes=[callback,wintypes.LPARAM]
    user.GetWindowThreadProcessId.argtypes=[wintypes.HWND,ctypes.POINTER(wintypes.DWORD)]
    user.IsWindowVisible.argtypes=[wintypes.HWND];user.SetForegroundWindow.argtypes=[wintypes.HWND]
    activated=False
    def tick():
        nonlocal activated
        if activated:return
        @callback
        def visit(hwnd,_):
            nonlocal activated
            pid=wintypes.DWORD();user.GetWindowThreadProcessId(hwnd,ctypes.byref(pid))
            if pid.value==process.pid and user.IsWindowVisible(hwnd):
                activated=bool(user.SetForegroundWindow(hwnd));return not activated
            return True
        user.EnumWindows(visit,0)
    return tick
try:
    with (out/'app.log').open('w',encoding='utf-8') as log:
        proc=subprocess.Popen([str(binary)],env=dict(env,CODEX_GUI_AUTOMATION=str(script)),stdout=log,stderr=log)
        peak=ProcessGuard(proc,timeout=120).wait(tick=foreground(proc))
    assert read('opened')['virtualCopy'] and read('opened')['pid'],read('opened')
    assert read('history')['history']>0 and 'COLOR_RED' in read('history')['text'],read('history')
    assert 'PASTED_NATIVE' in read('pasted')['text'],read('pasted')
    assert 'COLOR_RED' in (out/'clipboard.txt').read_text(encoding='utf-8')
    assert read('scrolled')['scrollback']>0,read('scrolled')
    assert read('anchored')['text']==read('scrolled')['text'], (read('scrolled'),read('anchored'))
    assert 'NATIVE' in read('typed')['text'],read('typed')
    assert read('before-escape')['focused'],read('before-escape')
    assert json.loads((out/'escape-stream.json').read_text(encoding='utf-8'))['busy'], 'Terminal Escape interrupted the model'
    assert 'AFTER_CTRL_C' in read('interrupted')['text'],read('interrupted')
    assert read('resized')['rows']==8 and read('resized')['cols']==62,read('resized')
    assert read('imported')['finished'] and 'Selected source files imported' in read('imported')['status'],read('imported')
    assert 'preserved in workspace' in read('imported')['status'],read('imported')
    assert read('closed-scroll')['scrollback']>0,read('closed-scroll')
    assert not Path(read('opened')['cwd']).exists()
    assert json.loads((out/'draft.json').read_text(encoding='utf-8'))['draft']=='Terminal preserves my unsent draft'
    assert json.loads((out/'workspace.json').read_text(encoding='utf-8'))['text']=='terminal imported\r\n'
    assert (repo/'src/main.rs').read_bytes()==original
    for name in ('child','chat-child','quit-child'):
        assert not alive(read(name)['pid']),name+' shell survived'
        assert not alive((out/(name+'.pid')).read_text()),name+' descendant survived'
    restart=out/'restart-steps.json';restart.write_text(json.dumps([{'wait_ready':30000},{'wait':1000},*dump('restarted'),{'quit':True}]),encoding='utf-8')
    with (out/'restart.log').open('w',encoding='utf-8') as log:
        proc=subprocess.Popen([str(binary)],env=dict(env,CODEX_GUI_AUTOMATION=str(restart)),stdout=log,stderr=log)
        peak=max(peak,ProcessGuard(proc,timeout=45).wait())
    assert read('restarted')['terminals']==0 and not read('restarted')['visible'],read('restarted')
    receipt=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_process_bytes=peak,native_input=True,copy_paste=True,escape_preserves_stream=True,palette_returns_terminal_focus=True,closed_history_retained=True,output_keeps_scroll_anchor=True,control_c=True,chat_close_cleanup=True,restart_no_shell_replay=True,vt_history_and_resize=True,explicit_vwork_import=True,unimported_recovery=True,source_unchanged=True,draft_retained=True,owned_descendants_closed=True,quit_cleanup=True)
    (out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n',encoding='utf-8')
    print('PASS: native terminal input, history, resize, vwork import/recovery and process cleanup',flush=True)
finally:provider.close();provider.write_evidence(out)
