#!/usr/bin/env python3
"""V15: every supported build entrypoint favors compilation speed."""
import os, pathlib, re, tomllib
root=pathlib.Path(__file__).resolve().parents[1]
data=tomllib.loads((root/'Cargo.toml').read_text())
expected={'opt-level':0,'debug':0,'split-debuginfo':'off','lto':False,
          'codegen-units':256,'incremental':True,'strip':'none','debug-assertions':False,'overflow-checks':False}
for name in ('dev','release','test'):
    for key,value in expected.items():
        assert data['profile'][name].get(key)==value,(name,key,data['profile'][name].get(key))
for name in ('dev','release'):
    build=data['profile'][name]['build-override']
    assert build=={'opt-level':0,'debug':0,'codegen-units':256}, (name,build)
    assert 'package' not in data['profile'][name], 'Dependency profile overrides need review'
for name,value in os.environ.items():
    if name.startswith('CARGO_PROFILE_'):
        key=next((k for k in expected if name.endswith('_'+k.upper().replace('-','_'))),None)
        if key: assert value.lower()==str(expected[key]).lower(),f'{name} violates fast-build policy'
assert os.environ.get('CARGO_INCREMENTAL','1')!='0','Incremental compilation must remain enabled'
for value in (os.environ.get('RUSTFLAGS',''),os.environ.get('CARGO_ENCODED_RUSTFLAGS','')):
    assert not re.search(r'(?:-O\b|opt-level\s*=\s*[1-3sz]|lto\s*=\s*(?:yes|true|thin|fat)|debuginfo\s*=\s*[12])',value), 'Optimization/debug Rust flags violate policy'
for path in [*root.glob('.github/workflows/*'),*root.glob('scripts/*'),root/'.cargo/config.toml']:
    if not path.is_file(): continue
    text=path.read_text(errors='replace')
    assert not re.search(r'(?:-C\s*opt-level\s*=\s*[1-3sz]|CARGO_INCREMENTAL\s*[:=]\s*["\']?0\b)',text),path
    assert not re.search(r'CARGO_PROFILE_\w+_(?:OPT_LEVEL|LTO|DEBUG|CODEGEN_UNITS)\s*[:=]\s*["\']?(?:[1-3sz]|true|thin|fat|4|16)\b',text),path
print('V15: fast dev, release, test and build-script profiles; no optimizing entrypoint overrides.')
