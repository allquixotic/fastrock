#!/usr/bin/env python3
"""Native Windows Slint acceptance using ONLY loopback HTTP and stdio fixtures.
Retains scripts, snapshots, request log and state dumps in build/rally-smoke.
"""
import base64, hashlib, json, os, re, shutil, subprocess, sys, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit, parse_qs

ROOT=Path(__file__).resolve().parent.parent
OUT=ROOT/'build'/'rally-smoke'
OUT.mkdir(parents=True,exist_ok=True)
TOKEN='fixture-token-not-a-secret'

class RallyFixture:
    def __init__(self):
        self.requests=[]; self.objects={}; self.serial=1000; self.fail_write=False
        owner=self
        class Handler(BaseHTTPRequestHandler):
            def log_message(self,*args): pass
            def do_GET(self): owner.handle(self)
            def do_POST(self): owner.handle(self)
            def do_DELETE(self): owner.handle(self)
        self.server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
        self.base=f'http://127.0.0.1:{self.server.server_port}/slm/webservice/v2.0/'
        self.seed()
    def ref(self,kind,n): return self.base+kind.lower()+'/'+str(n)
    def obj(self,kind,n,name,**fields):
        o=dict(_ref=self.ref(kind,n),_type=kind,ObjectID=n,Name=name,_refObjectName=name,LastUpdateDate='2026-10-10T00:00:00Z',VersionId='1',**fields)
        self.objects[o['_ref']]=o; return o
    def seed(self):
        self.user=self.obj('User',11,'Fixture User',UserName='fixture',DisplayName='Fixture User')
        self.other_user=self.obj('User',17,'Other User',UserName='other',DisplayName='Other User')
        self.workspace=self.obj('Workspace',1,'Fixture Workspace')
        self.project=self.obj('Project',2,'Fixture Team',Workspace=self.workspace)
        self.iteration=self.obj('Iteration',3,'Current Sprint',Workspace=self.workspace,Project=self.project,StartDate='2026-01-01T00:00:00Z',EndDate='2027-01-01T00:00:00Z')
        self.release=self.obj('Release',4,'FY2027 Q1',Workspace=self.workspace,Project=self.project)
        self.tag=self.obj('Tag',5,'Fixture Tag',Workspace=self.workspace)
        self.milestone=self.obj('Milestone',6,'Fixture Milestone',Workspace=self.workspace)
        for i in range(1,301):
            n=100+i
            self.obj('HierarchicalRequirement',n,f'Fixture story {i}',FormattedID=f'US{i}',Workspace=self.workspace,Project=self.project,Owner=self.user if i%2 else self.other_user,Iteration=self.iteration,Release=self.release,ScheduleState=['Defined','In-Progress','Completed','Accepted'][(i-1)%4],PlanEstimate=i%8+1,TaskEstimateTotal=3,TaskRemainingTotal=2,Blocked=False,Ready=True,Description='<h2>Preserved heading</h2><p>Hello <b>世界</b> &amp; Rally</p>',Notes='',Tags={'_ref':self.ref('hierarchicalrequirement',n)+'/Tags','Count':1},Milestones={'_ref':self.ref('hierarchicalrequirement',n)+'/Milestones','Count':1},Tasks={'_ref':self.ref('hierarchicalrequirement',n)+'/Tasks','Count':1},Children={'_ref':self.ref('hierarchicalrequirement',n)+'/Children','Count':0},Attachments={'_ref':self.ref('hierarchicalrequirement',n)+'/Attachments','Count':1},RevisionHistory={'_ref':self.ref('RevisionHistory',7)},DisplayColor='#4678aa')
        self.story=self.objects[self.ref('HierarchicalRequirement',101)]
        for kind,n,fid in [('Defect',14,'D1'),('TestSet',15,'TS1'),('DefectSuite',16,'DS1')]:
            self.obj(kind,n,f'Fixture {kind}',FormattedID=fid,Workspace=self.workspace,Project=self.project,Owner=self.user,ScheduleState='Defined',State='Submitted',PlanEstimate=1)

        self.task=self.obj('Task',8,'Fixture task',FormattedID='TA1',Workspace=self.workspace,Project=self.project,WorkProduct=self.story,State='Defined',Estimate=3,ToDo=2,Actuals=1)
        self.comment=self.obj('ConversationPost',9,'Discussion',Artifact=self.story,Text='Fixture comment',CreationDate='2026-10-10T00:00:00Z')
        self.content=self.obj('AttachmentContent',10,'Content',Content=base64.b64encode(b'fixture attachment').decode())
        self.attachment=self.obj('Attachment',12,'fixture.txt',Artifact=self.story,Content=self.content,Size=18,ContentType='text/plain')
        self.revision=self.obj('Revision',13,'Revision',Description='Initial fixture')
        self.history=self.obj('RevisionHistory',7,'History',Revisions={'_ref':self.ref('RevisionHistory',7)+'/Revisions','Count':1})
    def schema(self,kind):
        names=[('FormattedID','STRING',True,''),('Name','STRING',False,''),('ObjectID','INTEGER',True,''),('Description','TEXT',False,''),('Notes','TEXT',False,''),('Project','OBJECT',False,'Project'),('Workspace','OBJECT',True,'Workspace'),('Owner','OBJECT',False,'User'),('Iteration','OBJECT',False,'Iteration'),('Release','OBJECT',False,'Release'),('ScheduleState','STRING',False,''),('State','STRING',False,''),('PlanEstimate','DECIMAL',False,''),('Estimate','DECIMAL',False,''),('ToDo','DECIMAL',False,''),('Actuals','DECIMAL',False,''),('Blocked','BOOLEAN',False,''),('Ready','BOOLEAN',False,''),('Tags','COLLECTION',False,'Tag'),('Milestones','COLLECTION',False,'Milestone'),('DisplayColor','STRING',False,''),('Parent','OBJECT',False,'HierarchicalRequirement'),('WorkProduct','OBJECT',False,'Artifact'),('Text','TEXT',False,'')]
        result=[]
        for i,(name,typ,ro,refkind) in enumerate(names):
            o=dict(ElementName=name,Name=name,AttributeType=typ,ReadOnly=ro,Required=name=='Name',AllowedValueType={'TypePath':refkind})
            if name in ('ScheduleState','State'): o['AllowedValues']={'_ref':self.base+'typedefinition/allowed/'+name}
            result.append(o)
        return result
    def handle(self,h):
        parsed=urlsplit(h.path); path=parsed.path.split('/v2.0/')[-1].strip('/'); q={k:v[0] for k,v in parse_qs(parsed.query).items()}
        body=json.loads(h.rfile.read(int(h.headers.get('Content-Length','0'))) or b'{}')
        self.requests.append(dict(method=h.command,path=path,query=q,body=body))
        if h.headers.get('ZSESSIONID')!=TOKEN:
            return self.respond(h,{'QueryResult':{'Errors':['Fixture token missing']}},401)
        ref=self.base+path
        if h.command=='GET':
            if path=='user' and 'pagesize' not in q: return self.respond(h,{'User':self.user})
            if ref in self.objects: return self.respond(h,{self.objects[ref]['_type'].split('/')[-1]:self.objects[ref]})
            rows=[]; kind=path.lower(); expr=q.get('query','')
            if path=='typedefinition': rows=[{'TypePath':'HierarchicalRequirement','Attributes':{'_ref':self.base+'typedefinition/attributes'}}]
            elif path=='typedefinition/attributes': rows=self.schema('HierarchicalRequirement')
            elif path.startswith('typedefinition/allowed/'): rows=[{'StringValue':v} for v in ['Defined','In-Progress','Completed','Accepted']]
            elif path.endswith('/Tags'): rows=[self.tag]
            elif path.endswith('/Milestones'): rows=[self.milestone]
            elif path.endswith('/Tasks'): rows=[self.task]
            elif path.endswith('/Attachments'): rows=[self.attachment]
            elif path.endswith('/Revisions'): rows=[self.revision]
            else:
                rows=[o for o in self.objects.values() if o['_type'].lower()==kind or (kind=='artifact' and o['_type'] in q.get('types','HierarchicalRequirement,Defect,TestSet,DefectSuite').split(','))]
                if 'ObjectID' in expr:
                    m=re.search(r'ObjectID\s*=\s*"?(\d+)',expr)
                    if m: rows=[o for o in rows if o['ObjectID']==int(m[1])]
                # The selected real server scope is checked by assertions on request query.
                for field in ('FormattedID','Name'):
                    m=re.search(field+r'\s*(?:=|contains)\s*"([^"]+)"',expr)
                    if m: rows=[o for o in rows if m[1].lower() in str(o.get(field,'')).lower()]
            start=int(q.get('start',1)); size=int(q.get('pagesize',200)); total=len(rows)
            return self.respond(h,{'QueryResult':{'Errors':[],'Warnings':[],'Results':rows[start-1:start-1+size],'TotalResultCount':total,'StartIndex':start,'PageSize':size}})
        if self.fail_write: return self.respond(h,{'OperationResult':{'Errors':['Fixture write failure']}},503)
        if h.command=='DELETE':
            self.objects.pop(ref,None); return self.respond(h,{'OperationResult':{'Errors':[]}})
        fields=next(iter(body.values()),{})
        if path.endswith('/create'):
            self.serial+=1; kind=path[:-7]; o=self.obj(kind,self.serial,fields.get('Name','Created'),**{k:v for k,v in fields.items() if k!='Name'}); o['FormattedID']='USNEW'
        else:
            o=self.objects[ref]; o.update(fields); o['VersionId']=str(int(o['VersionId'])+1); o['LastUpdateDate']=f'2026-10-10T00:00:{int(o["VersionId"]):02}Z'
        return self.respond(h,{'OperationResult':{'Errors':[],'Object':o}})
    @staticmethod
    def respond(h,value,status=200):
        data=json.dumps(value).encode(); h.send_response(status);h.send_header('Content-Type','application/json');h.send_header('Content-Length',str(len(data)));h.end_headers();h.wfile.write(data)

def main():
    assert os.name=='nt', 'GUI acceptance runs only on Windows'
    binary=Path(sys.argv[1]).resolve()
    binary_sha=hashlib.sha256(binary.read_bytes()).hexdigest()
    print("Native executable SHA-256: "+binary_sha,flush=True)
    shutil.rmtree(OUT);OUT.mkdir(parents=True)
    fixture=RallyFixture()
    threading.Thread(target=fixture.server.serve_forever,daemon=True).start()
    home=OUT/'home'; home.mkdir(exist_ok=True); codex=OUT/'codex-home';codex.mkdir(exist_ok=True)
    (home/'rally.json').write_text(json.dumps({'rallyEndpoint':fixture.base,'rallyWorkspace':fixture.workspace['_ref'],'rallyProject':fixture.project['_ref'],'projectChildren':False}),encoding='utf-8')
    def rally(*args): return {'rally':list(args)}
    def wait(rows=0,detail=None,tabs=None): return {'wait_rally':dict(rows=rows,detail=detail,tabs=tabs,timeout_ms=30000)}
    def dump(name): return [rally('dump',str(OUT/(name+'.json'))),{'wait':300}]
    def snap(name): return {'snapshot':str(OUT/(name+'.png'))}
    script=[{'wait_ready':30000},rally('open','userstories'),wait(128),*dump('list'),snap('list'),
            {'click_text':'Ask AI'},{'wait':250},snap('assistant'),{'click_text':'×'},{'wait':250},
            rally('view','mode','board'),wait(128),rally('action','next'),wait(256),rally('action','next'),wait(300),*dump('board'),snap('board'),
            rally('points',str(OUT/'drag-points.json'),'Fixture story 1','Fixture story 2'),{'wait':2500},*dump('dragged'),rally('move',fixture.story['_ref'],'In-Progress','',fixture.ref('HierarchicalRequirement',102),'below'),wait(300),rally('action','undo-move'),wait(300),rally('view','group','Owner'),wait(300),rally('view','card-field','Priority'),wait(300),*dump('swimlanes'),snap('swimlanes'),rally('view','mode','list'),rally('action','refresh'),wait(128),
            rally('item',fixture.story['_ref']),wait(detail=True),*dump('detail'),snap('detail'),
            rally('edit','Name','Native edited title'),wait(detail=True),*dump('dirty'),rally('action','detail-save'),wait(detail=True),*dump('saved'),
            rally('view','relation','Attachments'),{'wait':500},*dump('attachments'),snap('attachments'),rally('view','relation','Tasks'),{'wait':500},*dump('tasks'),rally('view','relation','Revisions'),{'wait':500},*dump('revisions'),rally('view','relation','Discussions'),{'wait':1000},*dump('discussion'),
            rally('draft','comment','Fixture posted comment'),rally('action','comment'),{'wait':1000},
            rally('view','relation','Details'),rally('rich','Description','<h2>Edited heading</h2><p>Native <b>bold</b></p>','source'),rally('action','detail-save'),wait(detail=True),
            rally('edit','Name','Cancelled native draft'),rally('action','detail-back'),{'wait':100},snap('dirty-guard'),rally('dirty-choice','cancel'),wait(detail=True),rally('action','detail-back'),rally('dirty-choice','discard'),wait(128,False),rally('item',fixture.story['_ref']),wait(detail=True),rally('edit','Name','Save then navigate'),rally('action','detail-back'),rally('dirty-choice','save'),wait(128,False),*dump('save-navigate'),
            rally('set','view-name','Native saved view'),rally('action','save-view'),{'wait':500},*dump('views'),
            rally('set','filter-value','Unapplied filter draft'),rally('set','scroll-table','-120'),rally('open','teamboard'),wait(128,False,tabs=2),rally('action','next'),wait(256),rally('action','next'),wait(303),*dump('mixed'),snap('mixed'),rally('view','create-state','Completed'),wait(detail=True),*dump('new-in-lane'),rally('edit','Name','Native created story'),rally('action','detail-save'),wait(detail=True),*dump('created'),rally('action','detail-back'),wait(303,False),rally('close'),wait(128,False,tabs=1),*dump('independent'),

            rally('hide','search'),{'wait':100},rally('restore','all'),rally('select',fixture.story['_ref']),rally('select',fixture.ref('HierarchicalRequirement',102)),rally('action','bulk'),{'wait':500},rally('bulk-edit','Name','Bulk native title'),rally('picker','Owner'),{'wait':500},{'click_text':'Fixture User · fixture'},{'wait':300},*dump('picker-chosen'),rally('action','bulk-review'),{'wait':600},snap('bulk'),rally('action','bulk-apply'),{'wait':1000},rally('action','bulk-cancel'),
            rally('action','assistant'),rally('draft','assistant','Suggest a title change'),rally('send'),{'wait':2000},*dump('proposal'),snap('proposal'),
            rally('action','apply-proposal'),{'wait':1500},*dump('applied'),
            {'resize':[900,700]},{'wait':500},snap('narrow'),{'resize':[1600,1050]},{'wait':250},
            rally('action','assistant'),rally('item',fixture.story['_ref']),wait(detail=True),rally('edit','Name','Restored draft'),rally('draft','comment','Unsent discussion'),*dump('restore-before'),
            {'wait':500},{'quit':True}]
    scriptpath=OUT/'script.json';scriptpath.write_text(json.dumps(script),encoding='utf-8')
    env=dict(os.environ,FASTROCK_HOME=str(home),CODEX_HOME=str(codex),FASTROCK_CODEX=sys.executable,FASTROCK_CODEX_FIXTURE=str(ROOT/'dev'/'external-codex-fixture.py'),FASTROCK_RALLY_TOKEN=TOKEN,FASTROCK_FIXTURE_ARTIFACT=fixture.story['_ref'],CODEX_GUI_AUTOMATION=str(scriptpath),SLINT_BACKEND='winit-software',RUST_BACKTRACE='1')
    driver_errors=[]
    def drive_drag(proc):
        if (OUT/'drag-done').exists():return
        import ctypes
        from ctypes import wintypes
        u=ctypes.windll.user32
        u.EnumWindows.argtypes=[ctypes.WINFUNCTYPE(wintypes.BOOL,wintypes.HWND,wintypes.LPARAM),wintypes.LPARAM]
        u.GetWindowThreadProcessId.argtypes=[wintypes.HWND,ctypes.POINTER(wintypes.DWORD)]
        u.IsWindowVisible.argtypes=[wintypes.HWND];u.ClientToScreen.argtypes=[wintypes.HWND,ctypes.POINTER(wintypes.POINT)]
        u.SetForegroundWindow.argtypes=[wintypes.HWND]
        end=time.time()+75
        try:
            while time.time()<end and proc.poll() is None:
                path=OUT/'drag-points.json'
                if not path.exists():time.sleep(.05);continue
                try:points=json.loads(path.read_text())
                except ValueError:time.sleep(.05);continue
                windows=[]
                @ctypes.WINFUNCTYPE(wintypes.BOOL,wintypes.HWND,wintypes.LPARAM)
                def collect(hwnd,unused):
                    pid=wintypes.DWORD();u.GetWindowThreadProcessId(hwnd,ctypes.byref(pid))
                    if pid.value==proc.pid and u.IsWindowVisible(hwnd):windows.append(hwnd)
                    return True
                u.EnumWindows(collect,0);assert windows,'Native window not found'
                hwnd=windows[0];origin=wintypes.POINT();assert u.ClientToScreen(hwnd,ctypes.byref(origin));u.SetForegroundWindow(hwnd)
                scale=points['scale'];a,b=points['from'],points['to']
                a=[int(origin.x+a[0]*scale),int(origin.y+a[1]*scale)];b=[int(origin.x+b[0]*scale),int(origin.y+b[1]*scale)]
                u.SetCursorPos(*a);time.sleep(.1);u.mouse_event(2,0,0,0,0)
                try:
                    for i in range(1,25):u.SetCursorPos(int(a[0]+(b[0]-a[0])*i/24),int(a[1]+(b[1]-a[1])*i/24));time.sleep(.025)
                    time.sleep(.15)
                finally:u.mouse_event(4,0,0,0,0)
                (OUT/'drag-done').write_text(json.dumps({'from':a,'to':b,'pid':proc.pid}));return
            raise AssertionError('Native drag coordinates not reached')
        except Exception as e:driver_errors.append(str(e))
    def run():
        with (OUT/'app.log').open('a',encoding='utf-8') as log:
            proc=subprocess.Popen([str(binary)],cwd=ROOT,env=env,stdout=log,stderr=log)
            driver=threading.Thread(target=drive_drag,args=(proc,),daemon=True);driver.start()
            try: code=proc.wait(timeout=150)
            except subprocess.TimeoutExpired: proc.kill();raise AssertionError('Native automation timeout; inspect app.log')
            driver.join(timeout=2);assert not driver_errors,driver_errors
            assert code==0, f'Native app failed ({code}); inspect app.log'
    try:
        run()
        for name in ('list','board','detail','dirty','saved','views','proposal','applied'):
            assert (OUT/(name+'.json')).exists(),f'Missing {name} evidence; inspect app.log'
        load=lambda name: json.loads((OUT/(name+'.json')).read_text(encoding='utf-8'))
        for name in ('list','board','swimlanes','detail','saved','mixed','independent','picker-chosen','applied','restored','returned'):
            if (OUT/(name+'.json')).exists():assert not load(name)['error'],(name,load(name)['error'])
        assert not load('picker-chosen')['pickerOpen'],'Choosing a User left the picker open'
        for name in ('attachments','tasks','revisions'):assert load(name)['relations']==1,(name,load(name)['relations'])
        assert len(load('list')['rows'])==128
        assert any(cell['field']=='ScheduleState' and cell['text']=='In-Progress' for cell in next(row for row in load('dragged')['rows'] if row['ref']==fixture.story['_ref'])['cells']), 'Real native drag did not change the card state'
        assert any(r['method']=='POST' and r['body'].get('HierarchicalRequirement',{}).get('ScheduleState')=='In-Progress' for r in fixture.requests[:next(i for i,r in enumerate(fixture.requests) if r['body'].get('HierarchicalRequirement',{}).get('Name')=='Native edited title')]),'Native drag produced no workflow write'
        assert any(f['name']=='ScheduleState' and f['value']=='Completed' for f in load('new-in-lane')['fields']),'Lane creation lost workflow default'
        assert any(r['path']=='hierarchicalrequirement/create' and r['body']['HierarchicalRequirement']['Name']=='Native created story' for r in fixture.requests),'Native create did not write'
        assert len(load('board')['rows'])==300,'Board paging skipped records'
        assert len(load('swimlanes')['rows'])==300,'Changing board grouping lost the loaded working set'
        assert set(row['kind'] for row in load('mixed')['rows'])=={'HierarchicalRequirement','Defect','TestSet','DefectSuite'},'Mixed board lost concrete types'
        assert load('independent')['filterDraft']=='Unapplied filter draft','Switching documents lost unapplied filter'
        assert not load('save-navigate')['detail'],'Save and continue did not leave editor'
        assert any(r['body'].get('HierarchicalRequirement',{}).get('Name')=='Save then navigate' for r in fixture.requests),'Save and continue did not write'

        assert load('dirty')['dirty'] and not load('saved')['dirty'],(load('dirty')['dirty'],load('saved')['dirty'])
        assert fixture.story['Description']=='<h2>Edited heading</h2><p>Native <b>bold</b></p>'
        assert 'Native saved view' in load('views')['views']
        assert load('proposal')['proposal']==1, 'Dynamic tool proposal did not reach native dock'
        assert fixture.story['Name']=='AI reviewed title','Reviewed proposal not applied'
        # Restart same real binary with persisted native document/session drafts.
        scriptpath.write_text(json.dumps([{'wait_ready':30000},wait(detail=True),*dump('restored'),snap('restored'),{'quit':True}]),encoding='utf-8')
        run(); restored=load('restored')
        assert restored['dirty'] and any(f['name']=='Name' and f['value']=='Restored draft' for f in restored['fields'])
        childscript=OUT/'child-script.json'
        childscript.write_text(json.dumps([{'wait_ready':30000},wait(detail=True,tabs=1),{'wait':1500},*dump('child'),snap('child'),rally('transfer-first'),wait(tabs=0),{'wait':1500},{'quit':True}]),encoding='utf-8')
        env['FASTROCK_RALLY_CHILD_AUTOMATION']=str(childscript)
        scriptpath.write_text(json.dumps([{'wait_ready':30000},wait(detail=True,tabs=1),{'resize':[1600,1050]},{'wait':1500},{'click_text':'New window'},wait(tabs=0),wait(detail=True,tabs=1),*dump('returned'),snap('returned'),{'quit':True}]),encoding='utf-8')
        run();returned=load('returned');child=load('child')
        assert child['dirty'] and returned['dirty']
        assert child['filterDraft']==returned['filterDraft']=='Unapplied filter draft','Window transfer lost filter draft'

        assert any(f['name']=='Name' and f['value']=='Restored draft' for f in returned['fields']), 'Window transfer lost draft'
        writes=[r for r in fixture.requests if r['method']!='GET']
        assert any('rankBelow' in r['query'] for r in writes) and any('rankAbove' in r['query'] for r in writes),'Relative ranking/Undo missing'
        assert any(r['path']=='conversationpost/create' for r in writes),'Discussion not posted'
        assert all(r['query'].get('workspace')==fixture.workspace['_ref'] for r in fixture.requests if r['path']=='hierarchicalrequirement' and r['method']=='GET'), 'Scope lost'
        receipt={'binary_sha256':binary_sha,'requests':len(fixture.requests),'explicit_writes':len(writes),'result':'PASS','native_mouse_drag':True,'native_user_picker_click':True}
        (OUT/'acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n',encoding='utf-8')
        print(f'PASS: native Slint list/board/drag/rank/undo/swimlanes/detail/create/relations/rich/discussion/views/bulk/assistant/restart/window-transfer; {len(fixture.requests)} fixture requests, {len(writes)} explicit writes')
    finally:
        (OUT/'binary.sha256').write_text(binary_sha+'\n');(OUT/'requests.json').write_text(json.dumps(fixture.requests,indent=2),encoding='utf-8');fixture.server.shutdown()

if __name__=='__main__': main()
