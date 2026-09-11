#!/usr/bin/env python3
"""Opt-in real-model smoke test with ONLY synthetic images, isolated local DB."""
from pathlib import Path
import base64,json,os,subprocess,tempfile,time,urllib.request,urllib.error,uuid
root=Path(__file__).resolve().parents[1]
settings=json.loads((root/'data/runtime/settings.json').read_text())
settings.update(task='验证合成图片和图文分析接口。图中没有真实工作画面，不要推测人的状态。',interval_seconds=60,report_hour=23)
with tempfile.TemporaryDirectory(prefix='model-',dir=root/'data/validation') as d:
 p=Path(d)/'settings.json';p.write_text(json.dumps(settings));p.chmod(0o600)
 env=dict(os.environ,FLOW_INSIGHT_PORT='17902',FLOW_INSIGHT_DATA_DIR=d)
 with (root/'logs/model-validation.log').open('w') as log:
  process=subprocess.Popen([str(root/'target/debug/flow-insight')],env=env,stdout=log,stderr=log)
  def api(path,method='GET',data=None):
   request=urllib.request.Request('http://127.0.0.1:17902'+path,method=method,headers={'Content-Type':'application/json','x-flow-insight-client':'web'},data=None if data is None else json.dumps(data).encode())
   with urllib.request.urlopen(request,timeout=65) as r:return json.load(r)
  try:
   for _ in range(80):
    try: status=api('/api/status');break
    except urllib.error.URLError:time.sleep(.1)
   assert api('/api/settings')['api_key_configured']
   assert 'api_key' not in api('/api/settings')
   api('/api/settings/test','POST',{})
   session=api('/api/sessions','POST',{})
   frame='data:image/png;base64,'+base64.b64encode((root/'tests/fixtures/synthetic-frame.png').read_bytes()).decode()
   ids=[]
   for _ in range(2):
    sample_id=str(uuid.uuid4());ids.append(sample_id)
    api('/api/captures','POST',dict(id=sample_id,session_id=session['id'],captured_at=int(time.time()*1000),screen=frame,camera=None,capture_source='合成图片验证',activity=dict(app_name='合成测试',bundle_id='test.synthetic',switches=[])))
    deadline=time.time()+70
    while time.time()<deadline:
     view=api('/api/flow?date='+status['today']+'&mode=live')
     sample=next((s['sample'] for s in view['day']['segments'] if s['sample']['id']==sample_id),None)
     if sample and sample['state']=='done':break
     if sample and sample['state']=='error':raise RuntimeError(sample['error'])
     time.sleep(.4)
    assert sample and sample['state']=='done','model analysis did not finish'
   api('/api/sessions/'+session['id']+'/stop','POST',{})
   api('/api/report','POST',{'date':status['today'],'mode':'live'})
   deadline=time.time()+70
   while time.time()<deadline:
    view=api('/api/flow?date='+status['today']+'&mode=live')
    report=view['day']['report']
    if report and report['state']!='generating':break
    time.sleep(.5)
   assert report['state']=='done',report.get('error')
   assert len(view['day']['segments'])==2
   outcome={'ok':True,'model':settings['model'],'synthetic_samples':2,'categories':[s['category'] for s in view['day']['segments']],'report_state':report['state'],'real_screen_or_camera_uploaded':False,'tested':['vision endpoint','screen-only image analysis','recent context','persistence','flow aggregation','AI report','date deletion']}
   deleted=api('/api/data/delete-day','POST',{'date':status['today'],'mode':'live'})
   assert deleted['deleted']==2
   assert not api('/api/flow?date='+status['today']+'&mode=live')['day']['segments']
   out=root/'data/validation/model-validation.json';out.parent.mkdir(parents=True,exist_ok=True)
   out.write_text(json.dumps(outcome,ensure_ascii=False,indent=2))
   print(json.dumps(outcome,ensure_ascii=False))
  finally:
   process.terminate()
   try:process.wait(timeout=10)
   except subprocess.TimeoutExpired:process.kill();process.wait()
