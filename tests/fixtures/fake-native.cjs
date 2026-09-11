#!/usr/bin/env node
// Test-only IPC worker. Does not call OS capture APIs or request permissions.
const fs=require('node:fs'),path=require('node:path'),readline=require('node:readline');
const controlPath=process.env.FLOW_INSIGHT_FAKE_CAPTURE_CONTROL;
const frame='data:image/png;base64,'+fs.readFileSync(path.join(__dirname,'synthetic-frame.png')).toString('base64');
const readControl=()=>controlPath&&fs.existsSync(controlPath)?JSON.parse(fs.readFileSync(controlPath)):{};
function permissions(c){return {ok:true,screen_permission:c.screen_permission??true,camera_permission:c.camera_permission??'authorized',displays:c.displays||[{id:1,name:'内置测试屏幕',primary:true},{id:5,name:'外接测试屏幕',primary:false}],engine:'synthetic test worker'};}
const input=readline.createInterface({input:process.stdin});
input.on('line',async line=>{const request=JSON.parse(line),c=readControl();let response;
 if(request.command==='permissions')response=permissions(c);
 else if(request.command==='request_permission'){if(request.kind==='screen')c.screen_permission=true;else c.camera_permission='authorized';fs.writeFileSync(controlPath,JSON.stringify(c));response=permissions(c);}
 else if(request.command==='observe'){
  // camera_inactive simulates an occupied or disconnected camera; regions are synthetic geometry, no frame exists.
  const cameraOn=!!request.camera&&!c.camera_inactive&&!c.blocked_reason;
  const presence=!request.camera?{state:'disabled',reason:'未启用本地在座检测'}:c.blocked_reason?{state:'unknown',reason:'采集暂停'}:c.camera_inactive?{state:'unknown',reason:'摄像头或本地检测暂不可用；继续应用与屏幕观察'}:{state:c.presence||'present',reason:c.presence_reason||'合成本地在座检测'};
  const regions=cameraOn&&presence.state!=='not_detected'?(c.regions||[{kind:'face',x:.36,y:.2,w:.26,h:.36,confidence:.92},{kind:'body',x:.22,y:.16,w:.54,h:.84,confidence:.8}]):[];
  response=c.error?{ok:false,error:c.error}:{ok:true,captured_at:Date.now(),activity:{app_name:c.app_name||'测试编辑器',bundle_id:c.bundle_id||'test.editor',window_title:c.window_title||'工作文档',idle_seconds:c.idle_seconds||0,switches:[]},presence:{confidence:cameraOn?0.9:0,observed_at:Date.now(),...presence},presence_regions:regions,camera_active:cameraOn,camera_device:cameraOn?'合成测试摄像头':'',blocked_reason:c.blocked_reason||null};}
 else if(request.command==='sample'){if(c.delay_ms)await new Promise(r=>setTimeout(r,c.delay_ms));response=c.error?{ok:false,error:c.error}:{ok:true,screens:(request.displays||[]).map(d=>({display_id:d.id,display_name:d.name,captured_at:Date.now(),...((c.failed_displays||[]).includes(d.id)?{error:'测试显示器已断开'}:{image:frame})})),activity:{app_name:'测试编辑器',bundle_id:'test.editor',window_title:'工作文档',foreground_display_id:c.foreground_display_id??1,switches:[]},captured_at:Date.now(),capture_source:'受控多屏采集（合成画面）'};}
 else if(request.command==='preview')response=c.camera_inactive?{ok:true,image:null,reason:'摄像头未运行或暂无新画面'}:{ok:true,image:frame,captured_at:Date.now()};
 else response={ok:true};
 process.stdout.write(JSON.stringify(response)+'\n');
});
input.on('close',()=>process.exit(0));
