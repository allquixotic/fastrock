#!/usr/bin/env python3
"""V15: every supported build entrypoint favors compilation speed."""
import os, pathlib, re, sys, tomllib
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "scripts"))
root=pathlib.Path(__file__).resolve().parents[1]
data=tomllib.loads((root/'Cargo.toml').read_text())
expected={'opt-level':0,'debug':0,'split-debuginfo':'off','lto':False,
          'codegen-units':256,'incremental':True,'strip':'none','debug-assertions':False,'overflow-checks':False}
for name in ('dev','release','test'):
    for key,value in expected.items():
        assert data['profile'][name].get(key)==value,(name,key,data['profile'][name].get(key))
official={**expected, 'opt-level':1, 'debug':2, 'split-debuginfo':'packed', 'lto':'off'}
assert data['profile']['official']['inherits']=='release'
for key,value in official.items():
    assert data['profile']['official'].get(key, data['profile']['release'].get(key))==value, ('official',key)
assert 'package' not in data['profile']['official']
for name in ('dev','release','official'):
    build=data['profile'][name]['build-override']
    assert build=={'opt-level':0,'debug':0,'codegen-units':256}, (name,build)
    assert 'package' not in data['profile'][name], 'Dependency profile overrides need review'
for name,value in os.environ.items():
    if name.startswith('CARGO_PROFILE_'):
        key=next((k for k in expected if name.endswith('_'+k.upper().replace('-','_'))),None)
        if key:
            settings=official if name.startswith('CARGO_PROFILE_OFFICIAL_') else expected
            assert value.lower()==str(settings[key]).lower(),f'{name} violates declared build policy'
assert os.environ.get('CARGO_INCREMENTAL','1')!='0','Incremental compilation must remain enabled'
for value in (os.environ.get('RUSTFLAGS',''),os.environ.get('CARGO_ENCODED_RUSTFLAGS','')):
    assert not re.search(r'(?:-O\b|opt-level\s*=\s*[1-3sz]|lto\s*=\s*(?:yes|true|false|thin|fat)|debuginfo\s*=\s*[12])',value), 'Optimization/debug Rust flags violate policy'
for path in [*root.glob('.github/workflows/*'),*root.glob('scripts/*'),root/'.cargo/config.toml']:
    if not path.is_file(): continue
    text=path.read_text(errors='replace')
    assert not re.search(r'(?:-C\s*opt-level\s*=\s*[1-3sz]|CARGO_INCREMENTAL\s*[:=]\s*["\']?0\b)',text),path
    assert not re.search(r'CARGO_PROFILE_\w+_(?:OPT_LEVEL|LTO|DEBUG|CODEGEN_UNITS)\s*[:=]\s*["\']?(?:[1-3sz]|true|thin|fat|4|16)\b',text),path
print('V15/V25: fast defaults; explicit official profile opt=1, debug=2, LTO=off; no entrypoint overrides.')
