#!/usr/bin/env python3
"""V77: native permanent/batch deletion, drafts, durable queue and cleanup recovery."""
import argparse, base64, hashlib, json, os, sqlite3, subprocess, threading, time
from pathlib import Path
from embedded_provider_fixture import ProviderFixture, ProcessGuard
assert os.name=='nt','Run GUI acceptance on games only'
p=argparse.ArgumentParser(description=__doc__);p.add_argument('binary',type=Path);p.add_argument('--output',type=Path,required=True)
a=p.parse_args();binary=a.binary.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
provider=ProviderFixture();env=provider.environment(out);env['FASTROCK_WEBHOOK_BIND']='127.0.0.1:0'
repo=out/'project';(repo/'keep.txt').write_text('Preserve repository source\n',encoding='utf-8')
def dump(name):return [{'sidebar':['dump',str(out/(name+'.json'))]},{'wait':250}]
def read(name):return json.loads((out/(name+'.json')).read_text(encoding='utf-8'))
def action(id,verb):return [{'sidebar':['thread',id,verb]},{'wait':350}]
def run(name,steps):
 script=out/(name+'-steps.json');script.write_text(json.dumps(steps),encoding='utf-8')
 with (out/(name+'.log')).open('w',encoding='utf-8') as log:
  process=subprocess.Popen([str(binary)],env=dict(env,CODEX_GUI_AUTOMATION=str(script)),stdout=log,stderr=log)
  return ProcessGuard(process,timeout=150).wait()
def database():return next((out/'goose').rglob('sessions.db'))
def stored():
 with sqlite3.connect(database()) as db:return {r[0] for r in db.execute('SELECT id FROM sessions')}
def raw(id):return id.removeprefix('goose:')
def basename(id):return base64.urlsafe_b64encode(raw(id).encode()).decode().rstrip('=')
def delete(id,accept=True):return [*action(id,'delete'),{'dialog':accept},{'wait':1400},*([{'dialog':True},{'wait':300}] if accept else [])]
try:
 steps=[{'wait_ready':30000},{'resize':[1440,1100]}]
 for text in ['DraftKept conversation','DeleteOne conversation','QueueKept conversation','DeleteTwo conversation','LateDraft conversation']:
  steps += [{'new_thread':str(repo)},{'wait_idle':30000},{'send':text},{'wait':400},{'wait_idle':30000},{'wait':500}]
 peak=run('create',[*steps,{'wait_summaries':30000},*dump('created'),{'quit':True}])
 ids=[c['id'] for c in read('created')['navigation']['openChats']];assert len(ids)==5,ids
 first,second,third,fourth,late=ids
 batch_ids=ids[:4]
 queue=next(out.rglob(basename(third)+'.json'));binding=queue.with_name(basename(third)+'.binding.json')
 queue.write_text(json.dumps(dict(version=1,pending=[dict(id='unsent-fixture',input=[dict(type='text',text='Never dispatch this queue item',text_elements=[])],clientUserMessageId='fixture-pending')],dispatch=None,uncertain=[],paused=True)),encoding='utf-8')
 before=len(provider.requests)
 peak=max(peak,run('batch',[{'wait_ready':30000},{'wait':1600},{'resume':first},{'wait':500},{'composer':['type','Retain unsent draft é']},
  *action(second,'rename'),{'dialog_text':'Private manual title to delete'},{'dialog':True},{'wait':500},*action(second,'pin'),{'click_text':'Select'},{'wait':200},
  *sum([action(id,'select') for id in batch_ids],[]),{'click_text':'Delete'},{'wait':350},{'dialog':True},{'wait':3500},
  {'snapshot':str(out/'batch-results.png')},{'dialog':True},{'wait':400},*dump('after-batch'),
  {'palette':['dump',str(out/'draft.json')]},{'wait':300},{'quit':True}]))
 assert raw(first) in stored() and raw(third) in stored() and raw(second) not in stored() and raw(fourth) not in stored(),stored()
 assert read('draft')['draft']=='Retain unsent draft é',read('draft')
 assert json.loads(queue.read_text(encoding='utf-8'))['pending'][0]['id']=='unsent-fixture'
 assert not [r for r in provider.requests[before:] if not r['summary']],'Deletion or restart ran an agent'
 for id in [second,fourth]:
  for suffix in ['.binding.json','.json','.tools.json','.vwork.json','.delegates.json','.delegate-result.json','.outcome.json']:
   assert not (binding.parent/(basename(id)+suffix)).exists(),(id,suffix)
  marker=binding.parent/'deletions'/(basename(id)+'.json');assert json.loads(marker.read_text(encoding='utf-8'))['complete']
 state=json.loads((out/'codex/fastrock-sidebar.json').read_text(encoding='utf-8'));assert second not in state['entries'] and second not in state['order'],state
 cache_path=out/'codex/gui-thread-summaries.json';cache=json.loads(cache_path.read_text(encoding='utf-8'));assert second not in cache['threads'],cache
 cache['deleted']=[id for id in cache.get('deleted',[]) if id!=second];cache['threads'][second]=dict(manual_title='Private stale title after crash',tooltip='private stale summary');cache_path.write_text(json.dumps(cache))
 # Recreate an incomplete post-history cleanup marker: restart may finish metadata
 # cleanup, but an incomplete marker with existing history must await Delete.
 marker=binding.parent/'deletions'/(basename(second)+'.json');record=json.loads(marker.read_text(encoding='utf-8'));record['complete']=False;marker.write_text(json.dumps(record))
 stale=binding.parent/(basename(second)+'.tools.json');stale.write_text('["private obsolete tool"]')
 pending=dict(record,id=raw(first));pending_marker=marker.with_name(basename(first)+'.json');pending_marker.write_text(json.dumps(pending))
 peak=max(peak,run('recover',[{'wait_ready':30000},{'wait':1700},*dump('recovered'),{'quit':True}]))
 assert json.loads(marker.read_text(encoding='utf-8'))['complete'] and not stale.exists()
 assert second not in json.loads(cache_path.read_text(encoding='utf-8'))['threads'],'Restart retained a deleted cached title'
 assert not json.loads(pending_marker.read_text(encoding='utf-8'))['complete'] and raw(first) in stored(),'Restart replayed history deletion'
 # Remove the reviewed fixture queue explicitly through its native pending editor.
 peak=max(peak,run('finish',[{'wait_ready':30000},{'wait':1500},
  {'resume':first},{'wait':600},{'composer':['type','']},*delete(first,False),*dump('cancelled'),*delete(first),
  {'resume':third},{'wait':700},{'queue':['action','0','delete']},{'wait':600},
  *delete(third),*dump('finished'),{'quit':True}]))
 assert raw(first) not in stored() and raw(third) not in stored(),stored()
 assert any(r['id']==first for r in read('cancelled')['rows']),read('cancelled')
 assert not any(r['id'] in batch_ids for r in read('finished')['rows']),read('finished')
 # Hold the fixture's SQLite writer briefly after the GUI is ready. Deletion
 # waits in its worker while native composer edits continue on the UI thread.
 lock_errors=[]
 def delay_delete():
  try:
   deadline=time.monotonic()+25
   while not (out/'late-ready.json').exists():
    assert time.monotonic()<deadline,'Late-draft readiness timeout'
    time.sleep(.05)
   with sqlite3.connect(database(),timeout=10) as db:
    db.execute('BEGIN IMMEDIATE');time.sleep(4);db.rollback()
  except BaseException as e:lock_errors.append(str(e))
 locker=threading.Thread(target=delay_delete,daemon=True);locker.start()
 peak=max(peak,run('late-draft',[{'wait_ready':30000},{'wait':1500},{'resume':late},{'wait':700},*dump('late-ready'),{'wait':1000},
  *action(late,'delete'),{'dialog':True},{'wait':400},{'composer':['type','Typed while deleting — keep me']},{'wait':5500},{'dialog':True},{'wait':500},
  {'palette':['dump',str(out/'late-draft.json')]},{'wait':300},{'quit':True}]))
 locker.join(timeout=15);assert not locker.is_alive() and not lock_errors,lock_errors
 assert raw(late) not in stored(),stored()
 assert read('late-draft')['draft']=='Typed while deleting — keep me',read('late-draft')
 saved=json.loads((out/'codex/fastrock-open-chats.json').read_text(encoding='utf-8'))
 assert any(c['thread_id']!=late and c['draft']['text']=='Typed while deleting — keep me' for w in saved['windows'].values() for c in w['chats']),saved
 assert (repo/'keep.txt').read_text(encoding='utf-8')=='Preserve repository source\n'
 assert not [r for r in provider.requests[before:] if not r['summary']],'Unexpected model request'
 assert not provider.errors,provider.errors
 receipt=dict(result='PASS',binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),peak_process_bytes=peak,native_batch_partial_results=True,draft_preserved=True,typing_during_delete_preserved=True,durable_queue_guard=True,individual_confirm_and_cancel=True,history_and_owned_metadata_removed=True,repository_unchanged=True,restart_finishes_cleanup_only=True,existing_history_requires_explicit_retry=True,no_restart_dispatch=True)
 (out/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n');print('PASS: native deletion, draft/queue guards, partial batch and cleanup recovery',flush=True)
finally:
 provider.close();provider.write_evidence(out)
