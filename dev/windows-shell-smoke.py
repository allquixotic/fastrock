#!/usr/bin/env python3
"""V73: explicit native shell commands, durable history and owned crash/quit cleanup."""
import argparse, ctypes, hashlib, json, os, subprocess, time
from pathlib import Path
from embedded_provider_fixture import ProviderFixture, ProcessGuard
assert os.name=='nt','Run GUI acceptance on games only'
p=argparse.ArgumentParser(description=__doc__);p.add_argument('binary',type=Path);p.add_argument('--output',type=Path,required=True)
a=p.parse_args();binary=a.binary.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
repo=out/'project';provider=ProviderFixture(workspace_repo=repo);env=provider.environment(out);env['FASTROCK_WEBHOOK_BIND']='127.0.0.1:0'
config=out/'goose/config/config.yaml';value=json.loads(config.read_text(encoding='utf-8'))
value['extensions']={'vwork':dict(enabled=True,name='vwork',type='stdio',cmd=str(out/'absent/vwork.exe'),args=['--home',str(out/'local/vwork'),'mcp'],timeout=30)};config.write_text(json.dumps(value),encoding='utf-8')
(repo/'src').mkdir();original=b'original shell source\r\n';(repo/'src/main.rs').write_bytes(original)
git_env={k:v for k,v in os.environ.items() if not k.upper().startswith('GIT_')};git_env.update(GIT_CONFIG_NOSYSTEM='1',GIT_CONFIG_GLOBAL=str(out/'aws-empty'))
for args in (['init','-b','main','.'],['add','.'],['-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-m','Shell fixture']):
 subprocess.run(['git','-c','core.hooksPath=NUL','-c','core.autocrlf=false',*args],cwd=repo,env=git_env,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,check=True)
helper=out/'shell helper.py'
helper.write_text('''import sys,subprocess,time\nfrom pathlib import Path\nmode=sys.argv[1]\nif mode=='output':\n print('SHELL_BEGIN',flush=True)\n print('x'*400000,flush=True)\nelif mode=='child':\n child=subprocess.Popen([sys.executable,'-c','import time;time.sleep(120)'])\n Path(sys.argv[2]+'.pid').write_text(str(child.pid))\n with Path(sys.argv[2]+'.count').open('a') as f:f.write('launched\\n')\n print('CHILD_RUNNING',flush=True)\n time.sleep(120)\n''',encoding='utf-8')
def command(mode,*args):return '"'+os.sys.executable+'" "'+str(helper)+'" '+mode+' '+ ' '.join('"'+str(v)+'"' for v in args)
def shell(text,confirm=False):
 steps=[{'composer':['type','!'+text]},{'composer':['send']},{'wait':200}]
 if confirm:steps += [{'dialog':True},{'wait':300}]
 return steps
def finished():return [{'wait':350},{'wait_idle':30000}]
def export(name):return [{'transcript':'export-complete:'+str(out/(name+'.md'))},{'wait':500}]
def read(name):return (out/(name+'.md')).read_text(encoding='utf-8')
def run(name,steps,crash=None):
 script=out/(name+'-steps.json');script.write_text(json.dumps(steps),encoding='utf-8')
 with (out/(name+'.log')).open('w',encoding='utf-8') as log:
  process=subprocess.Popen([str(binary)],env=dict(env,CODEX_GUI_AUTOMATION=str(script)),stdout=log,stderr=log)
  guard=ProcessGuard(process,timeout=120)
  if crash is None:return guard.wait()
  try:
   while process.poll() is None and not crash.exists():guard.sample();time.sleep(.1)
   assert crash.exists(),'Command did not start before crash'
   time.sleep(.3);guard.sample();process.kill();process.wait(timeout=10)
  finally:
   if process.poll() is None:process.kill();process.wait(timeout=10)
  return guard.peak
def alive(pid):
 kernel=ctypes.WinDLL('kernel32',use_last_error=True);kernel.OpenProcess.restype=ctypes.c_void_p;kernel.OpenProcess.argtypes=[ctypes.c_ulong,ctypes.c_int,ctypes.c_ulong]
 handle=kernel.OpenProcess(0x1000|0x100000,False,int(pid))
 if not handle:return False
 try:
  kernel.WaitForSingleObject.argtypes=[ctypes.c_void_p,ctypes.c_ulong];return kernel.WaitForSingleObject(handle,0)==258
 finally:kernel.CloseHandle.argtypes=[ctypes.c_void_p];kernel.CloseHandle(handle)
try:
 peak=run('commands',[{'wait_ready':30000},{'resize':[1440,1000]},{'new_thread':str(repo)},{'wait_idle':30000},
  *shell('echo canceled> never.txt'),{'dialog':False},{'wait':250},{'palette':['dump',str(out/'canceled.json')]},{'wait':200},
  *shell('echo SHELL_NATIVE & echo quoted> "named file.txt" & type "named file.txt" & exit /b 7',True),*finished(),*export('local'),
  *shell(command('output')),*finished(),*export('bounded'),
  {'open_folder':str(repo)},{'wait':1000},{'wait_idle':30000},*shell('type src\\main.rs & echo shell-recovery-edit> src\\main.rs'),*finished(),*export('virtual'),
  *shell(command('child',out/'stop-child')),{'wait':900},*shell('echo forbidden> busy-must-not-run.txt'),{'palette':['dump',str(out/'busy-draft.json')]},{'wait':200},{'snapshot':str(out/'running.png')},{'shortcut':'escape'},*finished(),*export('stopped'),*shell('echo forbidden> paused-must-not-run.txt'),{'wait':400},{'palette':['dump',str(out/'paused-draft.json')]},{'wait':200},
  {'click_text':'Resume'},{'wait':300},*shell(command('child',out/'quit-child')),{'wait':900},{'quit':True}])
 assert not (repo/'never.txt').exists()
 assert json.loads((out/'canceled.json').read_text(encoding='utf-8'))['draft']=='!echo canceled> never.txt'
 assert 'SHELL_NATIVE' in read('local') and 'quoted' in read('local'),read('local')
 assert (repo/'named file.txt').read_text().strip()=='quoted'
 assert 'Output truncated after 256 KiB' in read('bounded') and len(read('bounded'))<350000
 assert 'original shell source' in read('virtual') and 'preserved in workspace' in read('virtual'),read('virtual')
 assert (repo/'src/main.rs').read_bytes()==original
 assert 'Command stopped' in read('stopped'),read('stopped')
 assert json.loads((out/'busy-draft.json').read_text(encoding='utf-8'))['draft']=='!echo forbidden> busy-must-not-run.txt'
 assert json.loads((out/'paused-draft.json').read_text(encoding='utf-8'))['draft']=='!echo forbidden> paused-must-not-run.txt'
 assert not list((out/'local/vwork').rglob('*must-not-run.txt'))
 assert not provider.requests,'Executing commands called an LLM'
 for name in ('stop-child','quit-child'):assert not alive((out/(name+'.pid')).read_text()),name+' survived'
 peak=max(peak,run('restore',[{'wait_ready':30000},{'wait':1300},*export('restored'),{'new_thread':str(repo)},{'wait_idle':30000},
  *shell('echo HISTORY_CONTEXT',True),*finished(),{'send':'Check the saved user shell output'},{'wait':300},{'wait_idle':30000},{'quit':True}]))
 assert 'User-requested shell command result' in read('restored') and 'Command stopped' in read('restored'),read('restored')
 ordinary=[r for r in provider.requests if not r['summary']]
 assert len(ordinary)==1,ordinary
 assert 'HISTORY_CONTEXT' in json.dumps(ordinary[0]['body']) and 'User-requested shell command result' in json.dumps(ordinary[0]['body'])
 peak=max(peak,run('crash',[{'wait_ready':30000},{'new_thread':str(repo)},{'wait_idle':30000},*shell(command('child',out/'crash-child'),True),{'wait':120000}],out/'crash-child.pid'))
 assert not alive((out/'crash-child.pid').read_text()),'Crash left descendant alive'
 before=len(provider.requests)
 peak=max(peak,run('crash-restart',[{'wait_ready':30000},{'wait':1500},*export('uncertain'),{'quit':True}]))
 assert (out/'crash-child.count').read_text().splitlines()==['launched'],'Restart replayed command'
 assert not [r for r in provider.requests[before:] if not r['summary']],'Restart called provider'
 assert not provider.errors,provider.errors
 receipt=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_process_bytes=peak,native_confirmation_and_cancel=True,quoted_shell_syntax=True,bounded_output=True,public_goose_history=True,next_turn_context=True,no_model_for_command=True,virtual_source_recovery=True,source_unchanged=True,stop_quit_crash_descendants=True,no_restart_replay=True)
 (out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n');print('PASS: native commands, history, bounds, vwork recovery, Stop/Quit/crash and no replay',flush=True)
finally:
 provider.close();provider.write_evidence(out)
