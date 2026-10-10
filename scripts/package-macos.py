#!/usr/bin/env python3
"""Developer ID signing and notarization in CI; never launch the GUI."""
import hashlib,json,os,pathlib,plistlib,shutil,subprocess,sys,tomllib
ROOT=pathlib.Path(__file__).resolve().parents[1]
IDENTITY='9A3CFFC04D3472208A62C48E707EA6D4261998A1'
TEAM='B6XDYNLMPU'
PROFILE=os.environ.get('NOTARY_PROFILE','AC_NOTARY')
BUNDLE='com.allquixotic.fastrock'

def run(*args,capture=False):
    print('+',' '.join(map(str,args)),flush=True)
    return subprocess.run(list(map(str,args)),check=True,text=True,
                          stdout=subprocess.PIPE if capture else None).stdout

def notarize(path,evidence,name):
    submission=json.loads(run('xcrun','notarytool','submit',path,'--keychain-profile',PROFILE,'--output-format','json',capture=True))
    (evidence/f'{name}-submit.json').write_text(json.dumps(submission,indent=2))
    identifier=submission['id']
    print('Notarization submission:',identifier,flush=True)
    run('xcrun','notarytool','wait',identifier,'--keychain-profile',PROFILE,'--timeout','20m')
    info=json.loads(run('xcrun','notarytool','info',identifier,'--keychain-profile',PROFILE,'--output-format','json',capture=True))
    (evidence/f'{name}-info.json').write_text(json.dumps(info,indent=2))
    log_path=evidence/f'{name}-log.json'
    run('xcrun','notarytool','log',identifier,'--keychain-profile',PROFILE,log_path)
    log=json.loads(log_path.read_text())
    assert info['status']==log['status']=='Accepted',f'Inspect {log_path}; do not resubmit unchanged'
    assert not any(issue.get('severity')=='error' for issue in log.get('issues') or [])
    return identifier

def verify_code(path):
    run('codesign','--verify','--deep','--strict','--verbose=4',path)
    result=subprocess.run(['codesign','--display','--verbose=4',str(path)],check=True,capture_output=True,text=True)
    assert f'TeamIdentifier={TEAM}' in result.stderr and 'runtime' in result.stderr

def main():
    assert sys.platform=='darwin' and os.environ.get('GITHUB_ACTIONS')=='true'
    os.chdir(ROOT)
    version=tomllib.load(open('Cargo.toml','rb'))['package']['version']
    binary=pathlib.Path(sys.argv[1]).resolve()
    assert IDENTITY in run('security','find-identity','-v','-p','codesigning',capture=True)
    run('xcrun','notarytool','history','--keychain-profile',PROFILE,'--output-format','json',capture=True)
    assert run('lipo','-archs',binary,capture=True).strip()=='arm64'
    dependencies=run('otool','-L',binary,capture=True).splitlines()[1:]
    assert all(line.strip().split(' ')[0].startswith(('/usr/lib/','/System/Library/')) for line in dependencies),dependencies
    dist=ROOT/'dist'
    stage=dist/'macos-stage'
    evidence=dist/'macos-signing-evidence'
    if stage.exists(): shutil.rmtree(stage)
    stage.mkdir(parents=True)
    evidence.mkdir(parents=True,exist_ok=True)
    run('bash','packaging/macos/bundle-app.sh',binary,stage,version,'--bundle-id',BUNDLE)
    app=stage/'Fastrock.app'
    resources=app/'Contents/Resources'
    for name in ('LICENSE','NOTICE','README.md','codex-gui-base.json'):
        shutil.copy2(ROOT/name,resources/name)
    shutil.copy2(ROOT/'docs/gui.md',resources/'USER-GUIDE.md')
    shutil.copy2(ROOT/'docs/RALLY-INVENTORY.md',resources/'RALLY-INVENTORY.md')
    shutil.copytree(ROOT/'third_party/slint/i-slint-core/LICENSES',resources/'licenses/slint')
    build_input=json.loads(binary.with_name('BUILD-INPUT.json').read_text())
    (resources/'BUILD.json').write_text(json.dumps(build_input,indent=2)+'\n')
    native=[]
    for path in app.rglob('*'):
        if path.is_file() and not path.is_symlink() and 'Mach-O' in run('file','-b',path,capture=True): native.append(path)
    assert native==[app/'Contents/MacOS/fastrock'],native
    executable=native[0]
    # This native client has no embedded runtime and needs no JIT entitlements.
    run('codesign','--force','--timestamp','--options','runtime','--sign',IDENTITY,'--identifier',BUNDLE+'.fastrock',executable)
    verify_code(executable)
    run('codesign','--force','--timestamp','--options','runtime','--sign',IDENTITY,app)
    verify_code(app)
    temporary_zip=dist/'app-notary.zip'
    run('ditto','-c','-k','--keepParent',app,temporary_zip)
    app_id=notarize(temporary_zip,evidence,'app')
    run('xcrun','stapler','staple',app)
    run('xcrun','stapler','validate',app)
    verify_code(app)
    run('spctl','--assess','--type','execute','--verbose=4',app)
    temporary_zip.unlink()
    (stage/'Applications').symlink_to('/Applications')
    shutil.copy2(ROOT/'LICENSE',stage/'LICENSE.txt')
    dmg=dist/f'fastrock-{version}-macos-arm64-fast.dmg'
    assert not dmg.exists(),'Never overwrite a final signed distribution'
    run('hdiutil','create','-volname','Fastrock','-srcfolder',stage,'-format','UDRO',dmg)
    run('codesign','--force','--timestamp','--sign',IDENTITY,dmg)
    run('codesign','--verify','--verbose=4',dmg)
    dmg_id=notarize(dmg,evidence,'dmg')
    run('xcrun','stapler','staple',dmg)
    run('xcrun','stapler','validate',dmg)
    run('codesign','--verify','--verbose=4',dmg)
    run('spctl','--assess','--type','open','--context','context:primary-signature','--verbose=4',dmg)
    mount=dist/'macos-verify-mount'
    mount.mkdir(exist_ok=True)
    run('hdiutil','attach','-readonly','-nobrowse','-mountpoint',mount,dmg)
    try:
        mounted_app=mount/app.name
        verify_code(mounted_app)
        run('xcrun','stapler','validate',mounted_app)
        run('spctl','--assess','--type','execute','--verbose=4',mounted_app)
        assert plistlib.loads((mounted_app/'Contents/Info.plist').read_bytes())['FastrockPrereleaseVersion']==version
    finally:
        run('hdiutil','detach',mount)
    checksum=hashlib.sha256(dmg.read_bytes()).hexdigest()
    dmg.with_suffix('.dmg.sha256').write_text(f'{checksum}  {dmg.name}\n')
    record={'version':version,'source_commit':os.environ['GITHUB_SHA'],'workflow_run':os.environ['GITHUB_RUN_ID'],
            'identity_sha1':IDENTITY,'team_id':TEAM,'app_submission':app_id,'dmg_submission':dmg_id,
            'status':'Accepted','stapling':'app and DMG validated','gatekeeper':'app, DMG and mounted app accepted',
            'sha256':checksum,'architectures':['arm64'],'binary_sha256':hashlib.sha256(executable.read_bytes()).hexdigest()}
    (evidence/'verification.json').write_text(json.dumps(record,indent=2)+'\n')
    print(json.dumps(record,indent=2),flush=True)

if __name__=='__main__': main()
