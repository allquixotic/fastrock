#!/usr/bin/env python3
"""Native Windows Goose conversations, followed by Rally via ACP/MCP fixtures."""
import hashlib, json, os, runpy, shutil, subprocess, sys
from pathlib import Path

assert os.name == 'nt', 'Execute GUI acceptance only on Windows'
root=Path(__file__).resolve().parent.parent
binary=Path(sys.argv[1]).resolve()
out=root/'build'/'goose-chat-smoke'
if out.exists(): shutil.rmtree(out)
out.mkdir(parents=True)
for folder in ('home','codex-home','project'): (out/folder).mkdir()
script=[{'wait_ready':30000},{'new_thread':str(out/'project')},{'wait_idle':30000},
        {'send':'hold first'},{'wait':300},{'new_thread':str(out/'project')},{'wait_idle':30000},
        {'send':'second conversation'},{'wait_idle':30000},
        {'transcript':'export-complete:'+str(out/'second.md')},{'snapshot':str(out/'second.png')},
        {'select_tab':0},{'shortcut':'escape'},{'wait_idle':10000},
        {'transcript':'export-complete:'+str(out/'first.md')},{'wait':300},
        {'click_text':'Goose · fixture-model  ▾'},{'wait':300},{'snapshot':str(out/'options.png')},
        {'quit':True}]
scriptpath=out/'script.json';scriptpath.write_text(json.dumps(script),encoding='utf-8')
env=dict(os.environ,FASTROCK_HOME=str(out/'home'),CODEX_HOME=str(out/'codex-home'),
         FASTROCK_GOOSE=sys.executable,FASTROCK_GOOSE_FIXTURE=str(root/'dev'/'external-goose-fixture.py'),
         CODEX_GUI_AUTOMATION=str(scriptpath),SLINT_BACKEND='winit-software')
env.pop('FASTROCK_FIXTURE_ARTIFACT',None)
env.pop('FASTROCK_RALLY_TOKEN',None)
with (out/'app.log').open('w',encoding='utf-8') as log:
    subprocess.run([str(binary),'--remote','goose'],cwd=root,env=env,stdout=log,stderr=log,timeout=90,check=True)
for name,sid in [('first','fixture-1'),('second','fixture-2')]:
    transcript=(out/(name+'.md')).read_text(encoding='utf-8')
    assert 'Started '+sid in transcript,(name,transcript)
    assert ('fixture-2' if sid=='fixture-1' else 'fixture-1') not in transcript,(name,transcript)
receipt=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),concurrent_native_tabs=True,isolated_cancel=True,native_config_options=True)
(out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n')
print('PASS: native concurrent Goose conversation tabs, isolated cancel, advertised option controls',flush=True)
sys.argv.append('--goose')
runpy.run_path(str(Path(__file__).with_name('windows-rally-smoke.py')),run_name='__main__')
