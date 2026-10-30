#!/usr/bin/env python3
"""Concurrent native chats/cancellation through embedded Goose and local Bedrock."""
import argparse, hashlib, json, os, subprocess, sys
from pathlib import Path
from embedded_provider_fixture import ProviderFixture, ProcessGuard

assert os.name == 'nt', 'Execute GUI acceptance only on Windows'
root=Path(__file__).resolve().parent.parent
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('--output', type=Path, default=root/'build'/'goose-chat-smoke')
parser.add_argument('--ci', action='store_true')
args=parser.parse_args()
binary=args.binary.resolve()
out=args.output.resolve()
out.mkdir(parents=True, exist_ok=False)
provider=ProviderFixture()
env=provider.environment(out, streaming=True)
script=[{'wait_ready':30000},{'new_thread':str(out/'project')},{'wait_idle':30000},
        {'send':'hold first'},{'wait':1000},{'new_thread':str(out/'project')},{'wait_idle':30000},
        {'send':'second conversation'},{'wait_idle':30000},{'wait':300},
        {'transcript':'export-complete:'+str(out/'second.md')},{'snapshot':str(out/'second.png')},
        {'select_tab':0},{'shortcut':'escape'},{'wait_idle':10000},
        {'transcript':'export-complete:'+str(out/'first.md')},{'wait':300},
        {'open_settings':'goose'},{'wait':300},{'snapshot':str(out/'options.png')},
        {'quit':True}]
scriptpath=out/'script.json';scriptpath.write_text(json.dumps(script),encoding='utf-8')
env['CODEX_GUI_AUTOMATION']=str(scriptpath)
try:
    with (out/'app.log').open('w',encoding='utf-8') as log:
        process=subprocess.Popen([str(binary),'--remote','goose'],cwd=root,env=env,stdout=log,stderr=log)
        peak=ProcessGuard(process, timeout=90).wait()
    first=(out/'first.md').read_text(encoding='utf-8')
    second=(out/'second.md').read_text(encoding='utf-8')
    assert 'Holding hold first' in first and 'second conversation' not in first,first
    assert 'Fixture answer for second conversation finished' in second and 'hold first' not in second,second
    assert sum(not request['summary'] for request in provider.requests)==2,provider.requests
    assert all(request['path'].endswith('/converse-stream') for request in provider.requests if not request['summary'])
    receipt=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                 backend='embedded-goose',peak_process_bytes=peak,concurrent_native_tabs=True,isolated_cancel=True)
    (out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n',encoding='utf-8')
    print('PASS: embedded Goose, native concurrent conversations and isolated stream cancellation',flush=True)
finally:
    provider.close()
    provider.write_evidence(out)
