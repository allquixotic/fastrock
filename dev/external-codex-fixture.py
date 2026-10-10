#!/usr/bin/env python3
"""Local stdio fixture. No inference, credentials, or network access."""
import json, os, sys, time
from pathlib import Path

def send(value):
    print(json.dumps(value), flush=True)

def thread(cwd, ephemeral=True):
    return dict(id='fixture-rally-thread', sessionId='fixture-session', preview='', ephemeral=ephemeral,
                modelProvider='openai', model='fixture-model', createdAt=1, updatedAt=1,
                status={'type':'idle'}, cwd=cwd, cliVersion='0.162.0', source='appServer', turns=[])

if '--version' in sys.argv:
    print('codex-cli 0.162.0'); sys.exit()
assert 'FASTROCK_RALLY_TOKEN' not in os.environ, 'Rally token leaked to external Codex'
for line in sys.stdin:
    request=json.loads(line)
    if 'method' not in request or 'id' not in request: continue
    method=request['method']; params=request.get('params') or {}; result={}
    if method=='initialize': result=dict(userAgent='codex-cli/0.162.0',codexHome=os.environ['CODEX_HOME'],platformFamily='windows',platformOs='windows')
    elif method=='config/read': result={'config':{'model':'fixture-model','model_provider':'openai'},'origins':{},'layers':[]}
    elif method=='model/list': result={'data':[dict(id='fixture-model',model='fixture-model',displayName='Fixture model',description='Local testing only',hidden=False,supportedReasoningEfforts=[{'reasoningEffort':'medium','description':'Fixture'}],defaultReasoningEffort='medium',isDefault=True)],'nextCursor':None}
    elif method=='account/read': result={'account':None,'requiresOpenaiAuth':False}
    elif method=='account/rateLimits/read': result={'rateLimits':None,'rateLimitsByLimitId':{}}
    elif method in ('thread/list','thread/loaded/list'): result={'data':[], 'nextCursor':None}
    elif method in ('project/list','skills/list','mcpServerStatus/list','experimentalFeature/list'): result={'data':[],'nextCursor':None}
    elif method=='thread/start': result=dict(thread=thread(params.get('cwd') or os.getcwd()), model='fixture-model',modelProvider='openai',cwd=params.get('cwd') or os.getcwd(),approvalPolicy='never',approvalsReviewer='user',sandbox={'type':'readOnly'},reasoningEffort='medium')
    elif method=='turn/start':
        result={'turn':{'id':'fixture-turn','items':[],'status':'inProgress','error':None}}
        send({'id':request['id'],'result':result})
        # Send a real dynamic tool request to exercise owner routing and proposal review.
        send({'id':'fixture-tool-call','method':'item/tool/call','params':{'threadId':'fixture-rally-thread','turnId':'fixture-turn','callId':'fixture-call','tool':'rally_propose','arguments':{'summary':'Fixture suggestion requires review','changes':[{'operation':'update','kind':'HierarchicalRequirement','ref':os.environ['FASTROCK_FIXTURE_ARTIFACT'],'fields':{'Name':'AI reviewed title'}}]}}})
        send({'method':'item/agentMessage/delta','params':{'threadId':'fixture-rally-thread','turnId':'fixture-turn','itemId':'fixture-message','delta':'I prepared a change for your review.'}})
        send({'method':'turn/completed','params':{'threadId':'fixture-rally-thread','turn':{'id':'fixture-turn','items':[],'status':'completed','error':None}}})
        continue
    send({'id':request['id'],'result':result})
