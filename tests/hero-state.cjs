const assert=require('node:assert/strict');
const {test}=require('node:test');
const fs=require('node:fs');
const vm=require('node:vm');
const path=require('node:path');
const source=fs.readFileSync(path.join(__dirname,'../src/web/app.js'),'utf8');
// Load the actual presentation rules without browser handlers or network IO.
const context=vm.createContext({});
vm.runInContext(source.split('\nfunction render(){')[0],context);
const hero=vm.runInContext('heroState',context);
const cameraTile=vm.runInContext('cameraTileState',context);
const now=Date.parse('2026-09-09T10:00:00+08:00');
function view(category='work'){
 return {date:'2026-09-09',status:{today:'2026-09-09',recorder:{running:true,session:{id:'current'},scheduling:{reason:'waiting'}}},metrics:{current_run_seconds:120},day:{segments:[{category,start:now-120000,end:now,sample:{id:'sample',session_id:'current',captured_at:now-60000,state:'done',mode:'live',analysis:{screen_activity:'正在编写代码。',summary:'正在编写代码并核对测试结果。'}}}]}};
}
test('current work shows the approved work character and estimate',()=>{
 const state=hero(view(),now);assert.equal(state.character,'work');assert.equal(state.tone,'observing');assert.equal(state.runSeconds,120);assert.match(state.evidence,/画面 · 活动性质估算/);
});
test('pausing replaces the old work headline and hides current duration',()=>{
 const v=view();v.status.recorder.running=false;v.status.recorder.error='old failure';const state=hero(v,now);assert.equal(state.title,'暂停');assert.equal(state.character,'paused');assert.equal(state.tone,'paused');assert.equal(state.runSeconds,0);assert.match(state.evidence,/上次状态：工作中/);
});
test('a new recording session cannot inherit the previous session state',()=>{
 const v=view();v.status.recorder.session.id='new-session';const state=hero(v,now);assert.equal(state.character,'unknown');assert.equal(state.runSeconds,0);
});
test('stale or future records do not claim a current work state',()=>{
 for(const end of [now-15000,now+60000]){const v=view();v.day.segments[0].end=end;assert.equal(hero(v,now).character,'unknown');}
});
test('away scheduling immediately supersedes an older work estimate but keeps observation active',()=>{
 const v=view();v.status.recorder.scheduling={reason:'away',away:true};const state=hero(v,now);assert.equal(state.character,'away');assert.equal(state.title,'离开');assert.equal(state.tone,'observing');assert.equal(state.runSeconds,0);
});
test('waiting for a model does not mean recording is paused',()=>{
 const v=view('unknown');v.day.segments[0].sample.state='analyzing';v.status.recorder.scheduling.reason='model_busy';const state=hero(v,now);assert.equal(state.character,'unknown');assert.equal(state.title,'分析中');assert.equal(state.tone,'observing');
});
test('a valid estimate remains visible while analysis is pending',()=>{
 const v=view();v.day.segments[0].estimated_from=v.day.segments[0].sample;v.day.segments[0].sample={...v.day.segments[0].sample,state:'analyzing'};v.status.recorder.scheduling.reason='model_busy';const state=hero(v,now);assert.equal(state.character,'work');assert.match(state.evidence,/沿用/);
});
test('permission, capture and model faults have a distinct card and hide the estimate',()=>{
 for(const reason of ['screen_permission','capture_failed','model_unavailable','unavailable']){const v=view();v.status.recorder.scheduling.reason=reason;const state=hero(v,now);assert.equal(state.character,'fault');assert.equal(state.tone,'fault');assert.equal(state.runSeconds,0);}
});
test('camera uncertainty alone is not treated as absence or a recording fault',()=>{
 const v=view();v.status.recorder.presence={state:'unknown',reason:'无法确定'};assert.equal(hero(v,now).character,'work');
});
test('locked and excluded applications stop current work presentation, not local observation',()=>{
 for(const reason of ['locked','excluded']){const v=view();v.status.recorder.scheduling.reason=reason;const state=hero(v,now);assert.equal(state.tone,'observing');assert.equal(state.character,'paused');assert.equal(state.runSeconds,0);}
});
test('historical records are explicitly labeled and have no live duration',()=>{
 const v=view();v.date='2026-09-08';const state=hero(v,now);assert.equal(state.tone,'history');assert.match(state.evidence,/历史记录 · 2026-09-08/);assert.equal(state.runSeconds,0);
});
test('empty history does not advertise a work estimate',()=>{
 const v=view();v.date='2026-09-08';v.day.segments=[];const state=hero(v,now);assert.equal(state.title,'当天暂无记录');assert.equal(state.runSeconds,0);
});
test('loss of connection never leaves an apparently live green work card',()=>{
 const v=view();v.connectionLost=true;const state=hero(v,now);assert.equal(state.tone,'fault');assert.equal(state.title,'无法连接后台');assert.equal(state.character,'fault');assert.equal(state.runSeconds,0);assert.match(state.summary,/无法确认/);
});
function cameraView(extra={}){
 const v=view();Object.assign(v.status.recorder,{camera:true,camera_active:true,camera_device:'测试摄像头',presence:{state:'present',confidence:.9,observed_at:now-3000,reason:'本地检测到人脸或上半身；不代表专注'},presence_regions:[{kind:'face',x:.1,y:.2,w:.3,h:.4,confidence:.9}]},extra);return v;
}
test('the viewfinder only appears while the recorder runs and the backend is reachable',()=>{
 assert.equal(cameraTile(cameraView(),now).visible,true);
 assert.equal(cameraTile(cameraView({running:false}),now).visible,false);
 const lost=cameraView();lost.connectionLost=true;assert.equal(cameraTile(lost,now).visible,false);
 const history=cameraView();history.date='2026-09-08';assert.equal(cameraTile(history,now).visible,true);
});
test('a camera left unchecked reads as off, not as a fault',()=>{
 const state=cameraTile(cameraView({camera:false}),now);assert.equal(state.state,'off');assert.match(state.label,/未启用/);assert.equal(state.boxes.length,0);assert.equal(state.preview,false);
});
test('a detected person is drawn mirrored in viewfinder units and carries no image',()=>{
 const state=cameraTile(cameraView(),now);assert.equal(state.state,'present');assert.match(state.meta,/上次检测 3 秒前/);assert.equal(state.preview,true);assert.deepEqual(JSON.parse(JSON.stringify(state.boxes)),[{kind:'face',x:96,y:24,w:48,h:48}]);assert.match(state.title,/测试摄像头 · .*置信度 90%/);assert.ok(!Object.keys(state).some(k=>/image|frame|src/.test(k)));
});
test('an enabled camera the helper could not run is a camera fault',()=>{
 const state=cameraTile(cameraView({camera_active:false,camera_device:'',presence:{state:'unknown',observed_at:now-2000,reason:'摄像头或本地检测暂不可用；继续应用与屏幕观察'},presence_regions:[]}),now);assert.equal(state.state,'fault');assert.match(state.label,/不可用/);assert.equal(state.preview,false);
});
test('lock screen and excluded apps pause detection instead of faulting',()=>{
 for(const reason of ['locked','excluded']){const state=cameraTile(cameraView({scheduling:{reason},camera_active:false}),now);assert.equal(state.state,'paused');assert.equal(state.boxes.length,0);assert.equal(state.preview,false);}
});
test('stale or missing results wait rather than posing as live',()=>{
 const stale=cameraView();stale.status.recorder.presence.observed_at=now-25000;assert.equal(cameraTile(stale,now).state,'waiting');assert.equal(cameraTile(stale,now).preview,true);
 assert.equal(cameraTile(cameraView({presence:{state:'unknown',observed_at:0,reason:''},camera_active:false}),now).state,'waiting');
});
test('poor lighting is unclear and nobody in frame is absence',()=>{
 const dark=cameraTile(cameraView({presence:{state:'unknown',observed_at:now-1000,reason:'光线不足、过曝或镜头被遮挡，无法判断'},presence_regions:[]}),now);assert.equal(dark.state,'unclear');assert.match(dark.meta,/光线/);
 const empty=cameraTile(cameraView({presence:{state:'not_detected',observed_at:now-1000,reason:'本帧未检测到人'},presence_regions:[]}),now);assert.equal(empty.state,'absent');
});
test('all result characters resolve to local white-line SVG symbols',()=>{
 const html=fs.readFileSync(path.join(__dirname,'../src/web/index.html'),'utf8');
 for(const name of ['work','distracted','away','unknown','paused','fault'])assert.match(html,new RegExp(`<symbol id="character-${name}"[^>]*><g fill="none" stroke="#ffffff"`));
});

test('two minute expiry wins over fresh local activity and an in-flight analysis',()=>{
 for(const inherited of [false,true]){
  const v=view();const s=v.day.segments[0];s.sample.captured_at=now-120000;
  if(inherited){s.estimated_from={...s.sample};s.sample={...s.sample,captured_at:now-1000,state:'analyzing'};}
  v.status.recorder.scheduling.reason='model_busy';
  assert.equal(hero(v,now-1).character,'work');
  const expired=hero(v,now);assert.equal(expired.title,'未判断');assert.equal(expired.runSeconds,0);
 }
});
test('the displayed basis distinguishes optional task context',()=>{
 const v=view();assert.match(hero(v,now).evidence,/活动性质估算/);
 v.day.segments[0].sample.task='修复时间线';assert.match(hero(v,now).evidence,/任务相关性估算/);
});
test('an expired local segment stays unjudged when the next analysis is busy',()=>{
 const v=view('unknown');v.day.segments[0].sample={id:'local',session_id:'current',captured_at:now-5000,state:'local'};
 v.status.recorder.scheduling.reason='model_busy';
 const state=hero(v,now);assert.equal(state.title,'未判断');assert.equal(state.runSeconds,0);assert.equal(state.tone,'observing');
});

test('unknown reasons distinguish failures, pending analysis and actual expiry',()=>{
 const reason=vm.runInContext('unknownReason',context);
 const segment={start:now,sample:{captured_at:now-120000,state:'done',evidence:{basis:'screen'},analysis:{category:'work'}}};
 assert.match(reason(segment),/超过 2 分钟/);
 segment.sample={state:'error',evidence:{basis:'screen'}};assert.match(reason(segment),/分析失败/);
 segment.sample={state:'analyzing'};assert.match(reason(segment),/尚未完成分析/);
 segment.sample={state:'done',analysis:{category:'unknown'}};assert.match(reason(segment),/无法确定工作状态/);
 segment.sample={state:'local',evidence:{basis:'local',trigger:'waiting'}};
 assert.match(reason(segment),/仅有在座或应用活动记录/);assert.doesNotMatch(reason(segment),/过期|超过/);
 segment.sample.evidence.trigger='model_busy';assert.match(reason(segment),/上一项画面分析尚未完成/);
});
test('a merged unknown period retains all reasons instead of only its last record',()=>{
 const samples=['capture_failed','model_busy','model_busy'].map((trigger,i)=>({category:'unknown',start:now+i*5000,end:now+(i+1)*5000,sample:{id:String(i),session_id:'same',state:'local',activity:{},evidence:{basis:'local',trigger}}}));
 context.syntheticSegments=samples;
 vm.runInContext('app.day={segments:syntheticSegments}',context);
 const grouped=vm.runInContext('groups(true)',context);
 assert.equal(grouped.length,1);assert.equal(grouped[0].unknownReasons.length,2);
 const explain=vm.runInContext('unknownExplanation',context);
 assert.match(explain(grouped[0]),/屏幕采集失败.*上一项画面分析尚未完成/);
 assert.match(explain(grouped[0]),/不计入工作或中断时长/);
});
