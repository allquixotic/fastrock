#!/usr/bin/env python3
"""Local ACP fixture, including concurrent prompts and Fastrock's loopback MCP tools.
No inference, credentials, external network calls, or real Rally writes.
"""
import json, os, sys, threading, time, urllib.request

assert 'FASTROCK_RALLY_TOKEN' not in os.environ, 'Rally token leaked to Goose'
out_lock = threading.Lock()
sessions = {}
permissions = {}
serial = 0

def send(value):
    with out_lock:
        print(json.dumps(dict(jsonrpc='2.0', **value)), flush=True)

def reply(req, result): send(dict(id=req['id'], result=result))
def update(sid, value): send(dict(method='session/update', params=dict(sessionId=sid, update=value)))
def config():
    return [dict(id=i, name=n, type='select', currentValue=values[0], options=[dict(value=v, name=v) for v in values])
            for i,n,values in [('provider','Provider',['fixture','other-fixture']),('model','Model',['fixture-model','future-model']),
                               ('mode','Mode',['approve','auto']),('thinking_effort','Thinking effort',['medium','future-effort'])]]
def info(sid):
    s=sessions[sid]
    return dict(sessionId=sid,cwd=s['cwd'],title=s.get('title','Fixture '+sid),updatedAt='2026-10-25T12:00:00Z',
                _meta=dict(createdAt='2026-10-24T12:00:00Z',lastMessageAt='2026-10-25T12:00:00Z',providerId='fixture',modelId='fixture-model'))
def rpc_http(url,method,params):
    assert url.startswith('http://127.0.0.1:'), 'non-loopback MCP URL'
    req=urllib.request.Request(url,json.dumps(dict(jsonrpc='2.0',id=1,method=method,params=params)).encode(),
                               headers={'Content-Type':'application/json','Accept':'application/json, text/event-stream'})
    with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(req,timeout=30) as r:
        return json.load(r)

def prompt(req):
    sid=req['params']['sessionId']; s=sessions[sid]
    text=' '.join(c.get('text','') for c in req['params']['prompt'])
    cancel=threading.Event(); s['cancel']=cancel
    update(sid,dict(sessionUpdate='session_info_update',_meta={'goose':{'activeRunId':'run-'+sid}}))
    if text=='disconnect': os._exit(0)
    if text == 'benchmark-tools':
        for n in range(6):
            tid = f'bench-{sid}-{n}'
            update(sid, dict(sessionUpdate='tool_call', toolCallId=tid, title=f'Fixture tool {n}', status='in_progress', rawInput={'fixture': n}))
            output = '\n'.join(f'Output {n}/{i}: line of tool text with preserved source order' for i in range(200))
            update(sid, dict(sessionUpdate='tool_call_update', toolCallId=tid, status='completed', content=[dict(type='content', content=dict(type='text', text=output))]))
    if text == 'selection-tools':
        tid = 'selection-' + sid
        update(sid, dict(sessionUpdate='tool_call', toolCallId=tid, title='Selection tool', status='in_progress',
                         rawInput={'command': 'echo ' + 'argument-é ' * 25 + 'ARGUMENT_END'}))
        output = '\n'.join(f'Selection output {i:02d}: **literal** <tag> é 👩‍💻' for i in range(24))
        update(sid, dict(sessionUpdate='tool_call_update', toolCallId=tid, status='completed',
                         content=[dict(type='content', content=dict(type='text', text=output))]))
    if text == 'benchmark-stream':
        for n in range(1000):
            if cancel.wait(.02): break
            update(sid,dict(sessionUpdate='agent_message_chunk',messageId='stream-'+sid,
                            content=dict(type='text',text=f'Streamed sentence {n} with **formatted** text.\n\n')))
    if 'permission' in text:
        pid='permission-'+sid; wait=threading.Event(); permissions[pid]=(wait,{})
        send(dict(id=pid,method='session/request_permission',params=dict(sessionId=sid,
            toolCall=dict(toolCallId='tool-'+sid,title='Read fixture file',rawInput={'path':'fixture.txt'}),
            options=[dict(optionId='allow-'+sid,name='Allow once',kind='allow_once'),dict(optionId='deny-'+sid,name='Reject',kind='reject_once')])))
        while not wait.wait(.01) and not cancel.is_set(): pass
        chosen=permissions.pop(pid)[1]
        update(sid,dict(sessionUpdate='agent_message_chunk',messageId='permission-result-'+sid,content=dict(type='text',text=json.dumps(chosen))))
    if text=='mcp':
        server=s['mcpServers'][0]
        tools=rpc_http(server['url'],'tools/list',{})['result']['tools']
        assert any(t['name']=='fixture__echo' for t in tools),tools
        result=rpc_http(server['url'],'tools/call',dict(name='fixture__echo',arguments={'value':sid}))
        assert result['result']==dict(content=[dict(type='text',text='tool accepted')],isError=False),result
    artifact=os.environ.get('FASTROCK_FIXTURE_ARTIFACT')
    if artifact:
        for server in s['mcpServers']:
            rpc_http(server['url'],'initialize',{})
            tools=rpc_http(server['url'],'tools/list',{})['result']['tools']
            if any(t['name']=='rally_propose' for t in tools):
                result=rpc_http(server['url'],'tools/call',dict(name='rally_propose',arguments=dict(summary='Fixture suggestion requires review',
                    changes=[dict(operation='update',kind='HierarchicalRequirement',ref=artifact,fields={'Name':'AI reviewed title'})])))
                assert 'result' in result and not result['result']['isError'],result
    update(sid,dict(sessionUpdate='agent_message_chunk',messageId='a-'+sid,content=dict(type='text',text='Started '+sid+' ')))
    if text.startswith('hold'):
        cancel.wait(30)
    else:
        cancel.wait(.15)
    update(sid,dict(sessionUpdate='agent_message_chunk',messageId='a-'+sid,content=dict(type='text',text='finished')))
    update(sid,dict(sessionUpdate='session_info_update',_meta={'goose':{'activeRunId':None}}))
    reply(req,dict(stopReason='cancelled' if cancel.is_set() else 'end_turn'))

for line in sys.stdin:
    req=json.loads(line)
    if 'method' not in req:
        if req.get('id') in permissions:
            event,answer=permissions[req['id']]; answer.update(req.get('result',{})); event.set()
        continue
    method=req['method']; p=req.get('params',{})
    if method=='initialize':
        assert p['protocolVersion']==1 and not p['clientCapabilities']['terminal']
        reply(req,dict(protocolVersion=1,agentInfo=dict(name='goose-fixture',version='1.56.0'),agentCapabilities=dict(loadSession=True,
            sessionCapabilities=dict(list={},close={},delete={}),mcpCapabilities=dict(http=True),promptCapabilities=dict(image=True))))
    elif method=='_goose/unstable/defaults/read': reply(req,dict(providerId='fixture',modelId='fixture-model'))
    elif method=='_goose/unstable/session/extensions/list':
        reply(req,dict(extensions=[dict(extensionKey='fastrock',extension=dict(type='mcp',server=s)) for s in sessions[p['sessionId']]['mcpServers']]))
    elif method=='_goose/unstable/session/system-prompt/set':
        assert p['mode']=='append' and p['text']; reply(req,{})
    elif method=='session/new':
        if '_meta' in p and 'enabledExtensions' in p['_meta']:
            assert p['_meta']['enabledExtensions']==[dict(type='mcp',server=s) for s in p['mcpServers']]
        serial+=1; sid='fixture-'+str(serial)
        sessions[sid]=dict(cwd=p['cwd'],mcpServers=p['mcpServers'],config=config())
        reply(req,dict(sessionId=sid,configOptions=sessions[sid]['config']))
    elif method=='session/list': reply(req,dict(sessions=[info(sid) for sid in sessions],nextCursor=None))
    elif method=='_goose/unstable/session/info': reply(req,dict(session=info(p['sessionId'])))
    elif method=='session/load':
        sid=p['sessionId']
        sessions[sid]['mcpServers']=p['mcpServers']
        for role,mid,value in [('user','history-u','Previous prompt'),('agent','history-a','Previous '),('agent','history-a','answer')]:
            update(sid,dict(sessionUpdate=role+'_message_chunk',messageId=mid,content=dict(type='text',text=value)))
        reply(req,dict(configOptions=sessions[sid]['config']))
    elif method=='session/prompt': threading.Thread(target=prompt,args=(req,),daemon=True).start()
    elif method=='session/cancel':
        if sessions[p['sessionId']].get('cancel'): sessions[p['sessionId']]['cancel'].set()
    elif method=='session/set_config_option':
        s=sessions[p['sessionId']]
        for option in s['config']:
            if option['id']==p['configId']:
                assert any(v['value']==p['value'] for v in option['options']); option['currentValue']=p['value']
        update(p['sessionId'],dict(sessionUpdate='config_option_update',configOptions=s['config']))
        reply(req,dict(configOptions=s['config']))
    elif method=='_goose/unstable/session/steer':
        assert p['expectedRunId']=='run-'+p['sessionId']
        reply(req,dict(runId=p['expectedRunId'],messageId='steered-'+p['sessionId']))
    elif method in ('session/close','session/delete'):
        if sessions[p['sessionId']].get('cancel'): sessions[p['sessionId']]['cancel'].set()
        if method=='session/delete': del sessions[p['sessionId']]
        reply(req,{})
    elif method=='_goose/unstable/session/rename':
        sessions[p['sessionId']]['title']=p['title']; reply(req,{})
    elif 'id' in req: send(dict(id=req['id'],error=dict(code=-32601,message='Fixture unsupported: '+method)))
