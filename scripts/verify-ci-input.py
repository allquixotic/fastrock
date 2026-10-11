#!/usr/bin/env python3
"""Require the same Actions run, source, version and unsigned binary bytes."""
import hashlib,json,os,pathlib,sys,tomllib
from release_config import build_plan
from debug_symbols import verify_windows,verify_macos,unpack_dsym
import tempfile
binary=pathlib.Path(sys.argv[1])
metadata=json.loads(binary.with_name('BUILD-INPUT.json').read_text())
assert metadata['source_commit']==os.environ['GITHUB_SHA']
assert metadata['workflow_run']==os.environ['GITHUB_RUN_ID']
assert metadata['release_tag']==os.environ['GITHUB_REF_NAME']
assert metadata['source_dirty'] is False
assert metadata['version']==tomllib.load(open('Cargo.toml','rb'))['package']['version']
assert metadata['binary_sha256'][binary.name]==hashlib.sha256(binary.read_bytes()).hexdigest()
plan=build_plan()
for field,value in plan.items():
    assert metadata[field]==value,field
if not plan['prerelease']:
    name='fastrock.pdb' if binary.suffix=='.exe' else 'fastrock.dSYM.zip'
    symbols=binary.with_name(name)
    assert metadata['binary_sha256'][name]==hashlib.sha256(symbols.read_bytes()).hexdigest()
    if binary.suffix=='.exe':
        assert verify_windows(binary,symbols)==metadata['symbols']['windows']
    else:
        with tempfile.TemporaryDirectory() as directory:
            assert verify_macos(binary,unpack_dsym(symbols,directory))==metadata['symbols']['macos']
print('Verified exact CI build input and debug symbols:',binary.name)
