#!/usr/bin/env python3
"""Require the same Actions run, source, version and unsigned binary bytes."""
import hashlib,json,os,pathlib,sys,tomllib
binary=pathlib.Path(sys.argv[1])
metadata=json.loads(binary.with_name('BUILD-INPUT.json').read_text())
assert metadata['source_commit']==os.environ['GITHUB_SHA']
assert metadata['workflow_run']==os.environ['GITHUB_RUN_ID']
assert metadata['source_dirty'] is False
assert metadata['version']==tomllib.load(open('Cargo.toml','rb'))['package']['version']
assert metadata['binary_sha256'][binary.name]==hashlib.sha256(binary.read_bytes()).hexdigest()
for field,value in {'profile':'dev','opt_level':0,'debug_info':0,'lto':False,'codegen_units':256,'incremental':True}.items():
    assert metadata[field]==value,field
print('Verified exact fast-profile CI build input:',binary.name)
