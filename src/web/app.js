'use strict';
const $=s=>document.querySelector(s), $$=s=>[...document.querySelectorAll(s)];
const labels={work:'工作中',distracted:'中断',away:'离开',unknown:'未判断'};
const stateLabels={pending:'待分析',queued:'等待分析',analyzing:'AI 分析中',done:'分析完成',error:'分析失败',local:'仅本地观察',skipped:'已跳过上传'};
let app={status:null,settings:null,day:null,metrics:null,date:'',page:'overview',selected:null,permissions:null,activityView:'timeline',selectedAt:null,insightView:'daily',primeDays:7,prime:null,primeKey:'',primeLoading:false,primeError:'',primeLoadedAt:0,primeSelected:null};
const esc=s=>String(s??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const time=t=>new Date(t).toLocaleTimeString('zh-CN',{hour:'2-digit',minute:'2-digit',hour12:false});
const minuteCount=s=>s>0&&s<60?'＜1':Math.floor(s/60);
const duration=s=>s===0?'0 分钟':s<60?'不足 1 分钟':s<3600?`${Math.floor(s/60)} 分钟`:`${Math.floor(s/3600)} 小时 ${Math.floor(s%3600/60)} 分`;
const empty=(title,body)=>`<div class="empty-section"><h2>${esc(title)}</h2><p>${esc(body)}</p></div>`;
const badge=c=>`<span class="badge ${esc(c)}">${labels[c]||labels.unknown}</span>`;
let toastTimer;
function toast(s){$('#toast').textContent=s;$('#toast').hidden=false;clearTimeout(toastTimer);toastTimer=setTimeout(()=>$('#toast').hidden=true,4500)}
async function api(path,method='GET',data){const r=await fetch(path,{method,headers:method==='GET'?{}:{'Content-Type':'application/json','x-flow-insight-client':'web'},body:data===undefined?undefined:JSON.stringify(data)});const v=await r.json();if(!r.ok)throw Error(v.error||`请求失败 ${r.status}`);return v}
async function action(fn,button){if(button)button.disabled=true;try{return await fn()}catch(e){toast(e.message)}finally{if(button)button.disabled=false}}
function blankVisual(){return '<div class="empty-visual"><svg viewBox="0 0 100 70" aria-hidden="true"><rect x="5" y="4" width="71" height="48" rx="5"/><path d="M30 63h23M42 52v11M19 20h18m-18 8h41m-41 8h30"/><rect x="60" y="39" width="33" height="25" rx="5"/><circle cx="77" cy="49" r="4"/><path d="M70 60c0-8 14-8 14 0"/></svg><p>暂无屏幕采样</p></div>'}
const selectedScreens=new Map();
const hasScreen=s=>!!s&&(s.screens?.length?s.screens.some(screen=>screen.file):!!s.screen);
function visual(s,detail=false){
 if(!hasScreen(s))return s?.evidence?presenceCard(s):blankVisual();
 const screens=s.screens||[];
 if(!screens.length)return `<img loading="lazy" class="${detail?'detail-image':'main-image'}" src="/api/media/${encodeURIComponent(s.id)}/screen" alt="${esc(s.analysis?.screen_activity||'屏幕采样画面')}">`;
 const foreground=s.activity?.foreground_display_id;
 const screen=(detail&&screens.find(d=>d.display_id===selectedScreens.get(s.id)))||screens.find(d=>d.file&&d.display_id===foreground)||screens.find(d=>d.file);
 const missing=screens.filter(d=>!d.file).length;
 const picture=screen.file?`<img loading="lazy" class="${detail?'detail-image':'main-image'}" src="/api/media/${encodeURIComponent(s.id)}/screen-${screen.display_id}" alt="${esc(screen.display_name)} · 屏幕 ${screen.display_id}">`:`<div class="screen-missing" role="status"><b>${esc(screen.display_name)} 未采集到画面</b><p>${esc(screen.error||'该屏幕当前不可用')}</p><small>本次分析没有包含这块屏幕的画面。</small></div>`;
 if(!detail)return `<div class="screen-preview">${picture}<span class="screen-count">${screens.length} 块屏幕${missing?` · ${missing} 块缺失`:''}</span></div>`;
 return `<div class="screen-gallery"><div class="screen-tabs" role="group" aria-label="选择回看的屏幕">${screens.map(d=>`<button type="button" class="subtle" data-screen-id="${d.display_id}" data-screen-sample="${esc(s.id)}" aria-pressed="${d.display_id===screen.display_id}">${esc(d.display_name)}${!d.file?' · 缺失':''}</button>`).join('')}</div><p class="screen-caption">屏幕 ${screen.display_id} · ${esc(screen.display_name)} · ${new Date(screen.captured_at).toLocaleTimeString('zh-CN',{hour12:false})}${foreground===screen.display_id?' · 前台窗口所在屏幕（估计）':''}</p>${picture}<p class="fine">本组 ${screens.length} 块屏幕，共用一次状态判断。前台窗口位置不能说明你正在看哪块屏幕。</p></div>`;
}

const presenceLabels={present:'检测到有人',not_detected:'暂未检测到人',unknown:'无法确定',disabled:'未启用'};
const reasonLabels={started:'开始记录',periodic:'每分钟分析',waiting:'等待下一分钟分析',context_changed:'应用或窗口内容变化',returned:'恢复在座或操作',stable:'活动稳定，等待补查',settling:'等待切换后的内容稳定',away:'持续未检测到人且没有键鼠活动',locked:'锁屏期间暂停',excluded:'排除应用期间暂停',screen_permission:'等待屏幕授权',model_unavailable:'待配置模型接口',model_busy:'上一项分析尚未完成',capture_failed:'本次屏幕采集失败',unavailable:'本地观察暂不可用'};
const basis=s=>s?.evidence?.basis==='local'?'本地观察':'AI 采样判断';
const description=s=>s?.analysis?.screen_activity||reasonLabels[s?.evidence?.trigger]||s?.error||'等待分析';
function presenceCard(s){const p=s?.evidence?.presence;return `<div class="local-presence"><span class="person-symbol" aria-hidden="true">◉</span><div><b>${esc(s?.evidence?.away?'离开':presenceLabels[p?.state]||'无本地检测记录')}</b><p>${esc(p?.reason||'此记录来自旧版采集')}</p><small>${s?.evidence?'摄像头画面不保存、不上传':'旧版画面保留在本机，新版不再采集或上传摄像头图片'}</small></div></div>`}
const timeRange=(start,end)=>{const format=t=>new Date(t).toLocaleTimeString('zh-CN',{hour:'2-digit',minute:'2-digit',second:'2-digit',hour12:false});return end-start<60000?`${format(start)}–${format(end)}`:`${time(start)}–${time(end)}`};
const segmentKey=s=>`${s.sample.id}:${s.start}`;
const judgment=s=>s.estimated_from||s.sample;
const taskBasis=s=>s?.task?.trim()?'任务相关性':'活动性质';
const segmentBasis=s=>s.estimated_from?`沿用 ${time(s.estimated_from.captured_at)} 的截图判断`:basis(s.sample);
const shortDuration=ms=>{const seconds=Math.round(ms/1000);return seconds<60?`${seconds} 秒`:seconds%60?`${Math.floor(seconds/60)} 分 ${seconds%60} 秒`:`${seconds/60} 分钟`};
// Explain only causes present in the record; local activity alone cannot prove expiry.
function unknownReason(segment){
 const s=segment?.sample||{},trigger=s.evidence?.trigger;
 const reasons={locked:'锁屏期间暂停了屏幕采集',excluded:'使用排除应用期间暂停了屏幕采集',screen_permission:'没有屏幕录制权限，无法采集画面',capture_failed:'屏幕采集失败，没有取得画面',unavailable:'本地观察失败，没有取得有效记录',model_unavailable:'模型接口不可用，未能分析画面',model_busy:'上一项画面分析尚未完成，本轮未采集新画面'};
 if(reasons[trigger])return reasons[trigger];
 if(s.state==='error')return '画面分析失败，没有得到判断结果';
 const category=s.correction||s.analysis?.category;
 if(s.evidence&&['work','distracted'].includes(category)&&segment.start>=s.captured_at+120000)return '画面判断已超过 2 分钟有效期';
 if(s.analysis)return '已分析画面，但无法确定工作状态';
 if(['pending','queued','analyzing'].includes(s.state))return '这条画面记录尚未完成分析';
 if(s.state==='skipped')return '这条画面记录已跳过分析';
 if(s.evidence?.basis==='local'||s.state==='local')return '仅有在座或应用活动记录，没有有效的画面判断';
 return '没有可用的画面判断，记录中未注明具体原因';
}
function unknownExplanation(segment){
 const reasons=segment.unknownReasons||[unknownReason(segment)];
 return reasons.join('；')+'。这段不计入工作或中断时长。';
}
function groups(timeline=false){
 const list=[];
 for(const s of app.day?.segments||[]){
  const p=list.at(-1),limit=s.sample.evidence?10000:1500;
  const same=p&&p.category===s.category&&p.sample.session_id===s.sample.session_id&&s.start-p.end<=limit;
  const detailMatches=p&&p.sample.activity.app_name===s.sample.activity.app_name&&p.sample.evidence?.basis===s.sample.evidence?.basis&&p.sample.evidence?.trigger===s.sample.evidence?.trigger&&p.estimated_from?.id===s.estimated_from?.id;
  if(same&&(timeline||detailMatches)){
   p.end=s.end;p.duration+=s.end-s.start;p.ids.push(segmentKey(s),s.sample.id);
   if(s.category==='unknown')p.unknownReasons=[...new Set([...p.unknownReasons,unknownReason(s)])];
   p.sample=s.sample;p.estimated_from=s.estimated_from;p.selection=segmentKey(s);
   if(judgment(s).analysis)p.source=judgment(s);
  }else list.push({...s,selection:segmentKey(s),ids:[segmentKey(s),s.sample.id],duration:s.end-s.start,source:judgment(s),unknownReasons:s.category==='unknown'?[unknownReason(s)]:[]});
 }
 return list;
}
// Recorder status and estimated work state describe different things.
function heroState(view,now=Date.now()){
 const recorder=view.status?.recorder||{},last=view.day?.segments?.at(-1),sample=last?.sample;
 const categoryTitles={...labels,unknown:'分析中'};
 const category=categoryTitles[last?.category]?last.category:'unknown';
 const source=last?.estimated_from||sample;
 const previous=last?`上次状态：${labels[category]} · ${time(last.end)}`:'';
 const result={tone:'paused',character:'paused',title:'尚未开始观察',summary:'点击“开始观察”，在后台记录工作状态。',evidence:'',runSeconds:0,settings:false};
 if(view.connectionLost)return {...result,tone:'fault',character:'fault',title:'无法连接后台',summary:'暂时无法确认采集是否仍在运行，正在重新连接。',evidence:previous};
 const fromRecord=()=>({character:category,title:categoryTitles[category],summary:category==='unknown'?'暂时没有足够依据判断工作状态。':category==='away'?'本地检测显示已离开。':source?.analysis?.screen_activity||source?.analysis?.summary||categoryTitles[category],evidence:source?.analysis?`${last.estimated_from?'沿用':'依据'} ${time(source.captured_at)} 画面 · ${taskBasis(source)}估算`:sample?`${time(sample.captured_at)} · ${basis(sample)}`:''});
 if(view.date&&view.date!==view.status?.today)return {...result,...fromRecord(),tone:'history',title:last?labels[category]:'当天暂无记录',summary:last?fromRecord().summary:'这一天还没有可回看的记录。',evidence:`历史记录 · ${view.date}${last?' · '+time(last.end):''}`};
 if(!recorder.running)return {...result,title:last||recorder.session?'暂停':'尚未开始观察',summary:last||recorder.session?'当前没有采集新的屏幕和活动记录。':result.summary,evidence:previous};
 const reason=recorder.scheduling?.reason;
 const faults={screen_permission:['屏幕采集不可用','请暂停观察，重新检查屏幕录制权限。'],capture_failed:['屏幕采集失败','本次没有取得新的屏幕画面，正在重试。'],unavailable:['本地观察不可用','暂时无法取得本地活动记录，正在重试。'],model_unavailable:['等待配置模型','本地观察继续运行，配置模型后才能分析画面。']};
 const sameSession=!!recorder.session?.id&&sample?.session_id===recorder.session.id;
 const recent=last&&sameSession&&now-last.end<15000&&last.end<=now+3000;
 const fault=faults[reason]||(view.status.analysis_notice?faults.model_unavailable:recorder.error?['观察暂时受限',recorder.error]:recent&&sample?.state==='error'&&!last.estimated_from?['画面分析失败',sample.error||'等待新的画面分析结果。']:null);
 if(fault)return {...result,tone:'fault',character:'fault',title:fault[0],summary:fault[1],evidence:previous,settings:reason==='model_unavailable'||!!view.status.analysis_notice};
 const active={...result,tone:'observing'};
 if(reason==='locked'||reason==='excluded')return {...active,character:'paused',title:'暂停',summary:reason==='locked'?'锁屏期间不采集新的屏幕画面，解锁后自动恢复。':'当前应用已排除，切换到其他应用后自动恢复。'};
 if(recorder.scheduling?.away||reason==='away')return {...active,character:'away',title:'离开',summary:'本地检测显示已离开，返回后自动恢复分析。'};
 const age=source?.captured_at==null?Infinity:now-source.captured_at;
 const valid=age>=-3000&&age<(view.settings?.judgment_ttl_seconds??120)*1000;
 if(!recent||category==='unknown'||!valid){
  const expired=!!source&&(source.analysis||source.correction)&&!valid;
  const pending=!last||['pending','queued','analyzing'].includes(sample?.state);
  return {...active,character:'unknown',title:expired||!pending?'未判断':'分析中',summary:expired?'上次判断已超过 2 分钟，等待新的画面结果。':pending?'正在等待新的画面分析结果。':'暂时没有足够依据，下一轮会继续分析。',evidence:previous};
 }
 return {...active,...fromRecord(),runSeconds:category==='work'?view.metrics?.current_run_seconds||0:0};
}
// The viewfinder reports what local detection found and whether the camera runs; `preview` says a live frame may be fetched.
function cameraTileState(view,now=Date.now()){
 const r=view.status?.recorder||{};
 if(view.connectionLost||!r.running)return {visible:false,state:'off',label:'',meta:'',title:'',boxes:[],preview:false};
 const p=r.presence||{},reason=r.scheduling?.reason,age=p.observed_at?now-p.observed_at:Infinity;
 const ago=age<1500?'刚刚':age<60000?`${Math.round(age/1000)} 秒前`:'超过 1 分钟前';
 const title=[r.camera_active?r.camera_device:'',p.reason,p.state==='present'&&p.confidence?`本帧置信度 ${Math.round(p.confidence*100)}%`:''].filter(Boolean).join(' · ');
 const tile=(state,label,meta,boxes=[])=>({visible:true,state,label,meta,title,boxes,preview:['present','absent','unclear','waiting'].includes(state)});
 if(!r.camera)return tile('off','未启用在座检测','摄像头未开启');
 if(reason==='locked'||reason==='excluded'||reason==='screen_permission')return tile('paused','检测已暂停',reason==='locked'?'锁屏期间关闭摄像头':reason==='excluded'?'排除应用期间关闭摄像头':'等待屏幕授权，摄像头已关闭');
 if(reason==='unavailable'||!p.observed_at)return tile('waiting','等待检测…','正在连接摄像头');
 if(!r.camera_active)return tile('fault','摄像头不可用','可能被其他程序占用或已断开');
 if(age>20000)return tile('waiting','等待检测…',`上次结果 ${ago}`);
 // Mirror like a self view so left and right match the viewer, in 160×120 viewfinder units.
 const boxes=(r.presence_regions||[]).map(g=>({kind:g.kind==='face'?'face':'body',x:Math.round((1-g.x-g.w)*1600)/10,y:Math.round(g.y*1200)/10,w:Math.round(g.w*1600)/10,h:Math.round(g.h*1200)/10}));
 if(p.state==='present')return tile('present','检测到有人',`上次检测 ${ago}`,boxes);
 if(p.state==='not_detected')return tile('absent','暂未检测到人',`上次检测 ${ago}`);
 return tile('unclear','看不清',/光线|遮挡|过曝/.test(p.reason||'')?'光线不足或镜头被遮挡':'检测结果不确定',boxes);
}
// Live view: the helper's in-memory frame over the loopback API. The page keeps only the current blob.
let previewOn=true;try{previewOn=localStorage.getItem('cameraPreview')!=='off'}catch{}
let previewBusy=false,previewUrl='';
function clearFrame(){const img=$('#cameraFrame');if(!img.hidden){img.hidden=true;img.removeAttribute('src')}if(previewUrl){URL.revokeObjectURL(previewUrl);previewUrl=''}$('#cameraTile').dataset.frame='none'}
function showFrame(blob){const img=$('#cameraFrame'),url=URL.createObjectURL(blob);img.src=url;img.hidden=false;if(previewUrl)URL.revokeObjectURL(previewUrl);previewUrl=url;$('#cameraTile').dataset.frame='live'}
async function previewTick(){
 if(previewBusy)return;
 const t=cameraTileState(app);
 if(!t.visible||!t.preview||!previewOn||document.visibilityState!=='visible'){clearFrame();return}
 previewBusy=true;
 try{const r=await fetch('/api/camera/preview',{cache:'no-store'});if(!r.ok)throw Error('no frame');showFrame(await r.blob())}catch{clearFrame()}finally{previewBusy=false}
}
function renderCameraTile(view=app){
 const t=cameraTileState(view),tile=$('#cameraTile');
 tile.hidden=!t.visible;if(!t.visible||!t.preview||!previewOn)clearFrame();if(!t.visible)return;
 tile.dataset.state=t.state;tile.title=t.title;
 $('#cameraLabel').textContent=t.label;$('#cameraMeta').textContent=t.meta;
 $('#cameraToggle').hidden=!t.preview;$('#cameraToggle').textContent=previewOn?'隐藏画面':'显示画面';
 $('#cameraRegions').innerHTML=t.boxes.map(b=>`<rect class="${b.kind}" x="${b.x}" y="${b.y}" width="${b.w}" height="${b.h}" rx="${b.kind==='face'?7:10}"/>`).join('');
}
function renderHero(view=app){
 const state=heroState(view),hero=$('#stateHero');
 hero.dataset.tone=state.tone;hero.dataset.character=state.character;
 for(const [id,value]of Object.entries({currentTitle:state.title,currentSummary:state.summary,currentEvidence:state.evidence}))$('#'+id).textContent=value;
 $('#currentEvidence').hidden=!state.evidence;
 $('#currentCharacter use').setAttribute('href',`#character-${state.character}`);
 $('#currentRun').hidden=state.runSeconds<=0;
 $('#currentRun').innerHTML=`<strong>${minuteCount(state.runSeconds)}</strong><span>分钟连续工作 · 估算</span>`;
 $('#heroSettings').hidden=!state.settings;
 $('#recordButton').disabled=!!view.connectionLost;
 $('#recordBadge').disabled=!!view.connectionLost;
}
function render(){if(!app.day)return;const day=app.day,segs=day.segments,last=segs.at(-1),running=app.status.recorder.running;
 $('#date').value=app.date;$('#recordBadge span').textContent=running?'后台记录中':'暂停';$('#recordBadge').classList.toggle('running',running);$('#recordButton').textContent=running?'暂停 Ⅱ':'开始观察 ↗';
 const error=app.status.recorder.error||app.status.analysis_notice;
 $('#errorBanner').hidden=!error;$('#errorBanner').textContent=error?`采样提示：${error}`:'';
 renderHero();renderCameraTile();
 const latestImage=[...segs].reverse().find(s=>hasScreen(s.sample))?.sample;
 $('#latestVisual').innerHTML=visual(latestImage)+(hasScreen(latestImage)?`<span class="image-overlay">${esc(latestImage.analysis?.app_name||latestImage.activity.app_name||'等待分析')}</span>`:'');
 $('#latestCaption').textContent=latestImage?description(latestImage):'';$('#latestTime').textContent=latestImage?time(latestImage.captured_at):'—';
 renderTimeline();renderWorkRhythm();renderReview();renderInsights();renderPrime();renderRecords();
 $('#stats').innerHTML=[[minuteCount(day.work_seconds),'分钟','估算投入'],[minuteCount(day.longest_work_seconds),'分钟','最长连续投入'],[app.metrics.interruptions,'次','中断']].map(([n,unit,label])=>`<div><div class="stat-value">${n}<small>${unit}</small></div><div class="stat-label">${label}</div></div>`).join('');
 $('#coverageNote').textContent=`今日有记录 ${duration(day.observed_seconds)} · 完成 ${day.counts.done||0} 次画面分析。投入与连续时长为估算。`;
 const high=app.metrics.high_period,low=app.metrics.low_period;
 $('#teaserTitle').textContent=day.report?.state==='done'&&!day.report_stale?day.report.headline:high?`${String(high.hour).padStart(2,'0')}:00，更连贯的一个小时。`:last?'当天记录已就绪':'暂无洞察';
 $('#teaserText').textContent=day.report?.state==='done'&&!day.report_stale?(day.report.suggestions[0]||'查看这一天的完整分析。'):low?`${String(low.hour).padStart(2,'0')}:00 时段中断更多，可以点开画面了解发生了什么。`:last?'可生成当天的 AI 分析。':'记录后可生成当天的分析。';
}
function renderTimeline(){
 const segs=app.day.segments,grouped=groups(true);
 const empty=!segs.length;
 $('#timeline').classList.toggle('is-empty',empty);
 $('#timeLabels').hidden=empty;
 if(empty){
  const historical=app.date!==app.status?.today;
  const title=historical?'这一天还没有记录':app.status?.recorder?.running?'正在等待第一段记录':'今天的时间线，等待开始';
  const hint=historical?'换个日期，看看其他日子的工作状态。':app.status?.recorder?.running?'正在观察，记录会陆续出现在这里。':'点击上方「开始观察」，留下今天的工作记录。';
  $('#timeline').innerHTML=`<div class="timeline-empty"><span class="timeline-empty-icon" aria-hidden="true"><svg viewBox="0 0 64 64" fill="none"><path d="M12 33h40" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-dasharray="3 5"/><circle cx="17" cy="33" r="5" fill="#fffef8" stroke="currentColor" stroke-width="2"/><circle cx="47" cy="33" r="5" fill="#fffef8" stroke="currentColor" stroke-width="2"/><rect x="26" y="21" width="12" height="24" rx="6" fill="#daeaa5" stroke="currentColor" stroke-width="2"/><path d="M45 13v6m-3-3h6" stroke="#d97750" stroke-width="2" stroke-linecap="round"/></svg></span><div><h3>${title}</h3><p>${hint}</p></div></div>`;
  $('#timeLabels').innerHTML='';$('#timelineSelected').innerHTML='';return;
 }
 const start=segs[0].start,end=segs.at(-1).end,span=Math.max(end-start,1);
 const selected=grouped.find(s=>s.ids.includes(app.selected))||grouped.at(-1);
 $('#timeline').innerHTML=grouped.map(s=>`<button class="time-piece ${s.category} ${s===selected?'selected':''}" style="left:${(s.start-start)/span*100}%;width:${(s.end-s.start)/span*100}%" data-select="${esc(s.selection)}" aria-label="${timeRange(s.start,s.end)} ${labels[s.category]}，约 ${shortDuration(s.duration)}" aria-pressed="${s===selected}" title="${timeRange(s.start,s.end)} · ${labels[s.category]} · 约 ${shortDuration(s.duration)}">${(s.end-s.start)/span>.15?`<span>${labels[s.category]} · 约 ${shortDuration(s.duration)}</span>`:''}</button>`).join('');
 $('#timeLabels').innerHTML=[0,.25,.5,.75,1].map(n=>`<span>${time(start+span*n)}</span>`).join('');
 const s=selected,source=s.source;
 $('#timelineSelected').innerHTML=`<div>${badge(s.category)} <span>${timeRange(s.start,s.end)} · 约 ${shortDuration(s.duration)}</span><p>${esc(s.category==='unknown'?unknownExplanation(s):s.category==='away'?'本地检测显示这段时间已离开。':description(source))}</p></div><button class="text-button" data-review="${esc(s.selection)}">${s.category==='unknown'?'查看记录与原因':'查看依据'} ↗</button>`;
 $('#timelineSelected').dataset.category=s.category;
 positionTimelineDetail();
}
function positionTimelineDetail(){
 const timeline=$('#timeline'),detail=$('#timelineSelected'),selected=timeline.querySelector('.selected');
 if(!selected||!timeline.clientWidth)return;
 const axis=timeline.getBoundingClientRect(),block=selected.getBoundingClientRect(),width=detail.getBoundingClientRect().width;
 const center=block.left-axis.left+block.width/2;
 const left=Math.max(0,Math.min(center-width/2,axis.width-width));
 detail.style.marginLeft=`${left}px`;
 detail.style.setProperty('--pointer-x',`${Math.max(18,Math.min(center-left,width-18))}px`);
}
const rhythmCells=new Map();
function activityView(name){
 app.activityView=name==='rhythm'?'rhythm':'timeline';
 const rhythm=app.activityView==='rhythm';
 $('#timelinePanel').hidden=rhythm;$('#workRhythmPanel').hidden=!rhythm;$('#timelineLegend').hidden=rhythm;
 $('#activityViewTitle').textContent=rhythm?'工作节奏':'状态时间线';
 $$('[data-activity-view]').forEach(b=>b.setAttribute('aria-selected',String(b.dataset.activityView===app.activityView)));
 if(rhythm)renderWorkRhythm();else positionTimelineDetail();
}
function renderWorkRhythm(){
 if(app.activityView!=='rhythm'||!app.day)return;
 const focused=document.activeElement?.dataset.rhythmMinute;
 const day=app.day,segments=day.segments,rows=FlowRhythm.hourRows(app.date,segments),interruptions=app.metrics.interruptions;
 $('#workRhythmSource').textContent=`${app.date} · 当天记录 · 已观察 ${duration(day.observed_seconds)}`;
 $('#workRhythmNumber').textContent=interruptions;
 $('#workRhythmTitle').innerHTML=!segments.length?'这一天还没有工作记录。':day.work_seconds===0&&day.distracted_seconds===0?'还在观察，<br><em>等待足够的判断依据。</em>':interruptions>0?'投入与中断，<br><em>都在这段时间里。</em>':'这段时间，<br><em>工作正在继续。</em>';
 rhythmCells.clear();
 const selected=segments.find(s=>segmentKey(s)===app.selected||s.sample.id===app.selected);
 const selectedAt=app.selectedAt??selected?.start;
 const tabAt=selectedAt??segments[0]?.start;
 $('#workRhythmGrid').innerHTML=rows.length?rows.map(row=>`<div class="work-hour"><span class="work-hour-label">${time(row.start)}</span><div class="work-minutes">${row.cells.map(cell=>{
  rhythmCells.set(cell.start,cell);
  const colors=cell.parts.map(p=>`var(--focus-${p.category}) ${(p.start-cell.start)/600}% ${(p.end-cell.start)/600}%`).join(',');
  const description=cell.parts.map(p=>`${labels[p.category]||'未记录'} ${shortDuration(p.end-p.start)}`).join('；');
  const label=`${time(cell.start)} · ${description}`;
  return `<button type="button" class="work-minute" tabindex="${tabAt>=cell.start&&tabAt<cell.end?0:-1}" data-rhythm-minute="${cell.start}" style="--minute-fill:linear-gradient(to right,${colors})" aria-label="${esc(label)}" title="${esc(label)}" aria-pressed="${selectedAt>=cell.start&&selectedAt<cell.end}" ${cell.parts.some(p=>p.key)?'':'disabled'}></button>`;
 }).join('')}</div></div>`).join(''):empty('从一次工作记录开始','记录后，可以在这里按小时查看状态变化。');
 const group=groups(true).find(g=>g.ids.includes(app.selected));
 $('#rhythmSelected').innerHTML=group?`<div>${badge(group.category)}<span class="selection-time">${timeRange(group.start,group.end)} · 约 ${shortDuration(group.duration)}</span><p>${esc(group.category==='unknown'?unknownExplanation(group):group.category==='away'?'本地检测显示这段时间已离开。':description(group.source))}</p></div><button type="button" class="text-button" data-review="${esc(app.selected)}">查看画面与依据 ↗</button>`:'';
 if(focused)$(`[data-rhythm-minute="${focused}"]`)?.focus({preventScroll:true});
}
function insightView(name){
 app.insightView=name==='prime'?'prime':'daily';
 $('#dailyInsightsPanel').hidden=app.insightView!=='daily';$('#primeInsightsPanel').hidden=app.insightView!=='prime';
 $$('[data-insight-view]').forEach(b=>b.setAttribute('aria-selected',String(b.dataset.insightView===app.insightView)));
 if(app.insightView==='prime')loadPrime();
}
const primeQueryKey=()=>`${app.date}|${app.primeDays}`;
let primeSerial=0;
function invalidatePrime(){primeSerial++;app.prime=null;app.primeKey='';app.primeSelected=null;app.primeLoading=false;app.primeError='';}
async function loadPrime(force=false){
 if(!app.date)return;
 const key=primeQueryKey();
 if(app.primeKey===key&&(app.primeLoading||(!force&&app.prime&&Date.now()-app.primeLoadedAt<60000)))return;
 const serial=++primeSerial;
 if(app.primeKey!==key){app.prime=null;app.primeSelected=null;}
 app.primeKey=key;app.primeLoading=true;app.primeError='';renderPrime();
 try{
  const result=await api(`/api/rhythm?date=${encodeURIComponent(app.date)}&days=${app.primeDays}`);
  if(serial!==primeSerial||key!==primeQueryKey())return;
  app.prime=result;app.primeLoadedAt=Date.now();
 }catch(e){if(serial===primeSerial&&key===primeQueryKey())app.primeError=e.message;}
 finally{if(serial===primeSerial){app.primeLoading=false;renderPrime();}}
}
function renderPrime(){
 if(app.insightView!=='prime')return;
 const focused=document.activeElement?.dataset,focusDate=focused?.primeDate,focusHour=focused?.primeHour;
 const r=app.primeKey===primeQueryKey()?app.prime:null;
 $('#refreshPrime').disabled=app.primeLoading;$('#refreshPrime').textContent=app.primeLoading?'整理中…':'刷新';
 $('#primeRangeLabel').textContent=r?`${r.start_date} — ${r.end_date} · 跨日观察`:`截至 ${app.date} · 近 ${app.primeDays} 天`;
 $('#primeNotice').hidden=!app.primeError;
 $('#primeNotice').textContent=app.primeError?`读取失败：${app.primeError}${r?'。下方保留上次结果，请刷新重试。':'，请刷新重试。'}`:'';
 if(!r){$('#primeTitle').textContent=app.primeError?'暂时无法整理记录。':'正在整理工作记录。';$('#primeSummary').textContent='';$('#primeHeatmap').innerHTML='';$('#primeSelection').innerHTML='';$('#primeMethod').textContent='';return;}
 const best=r.best_period;
 $('#primeTitle').innerHTML=best?`<em>${String(best.hour).padStart(2,'0')}:00–${String(best.end_hour).padStart(2,'0')}:00</em><br>投入相对更集中。`:r.status==='similar'?'已观察的时段，<br><em>投入比例比较接近。</em>':r.recorded_days?'记录正在积累，<br><em>再多看几天。</em>':'这段时间，<br><em>还没有记录。</em>';
 $('#primeSummary').textContent=best?`${r.recorded_days} / ${r.days} 天有记录\n该时段有 ${best.eligible_days} 天可比较\n平均投入占已判断活动 ${Math.round(best.work_ratio*100)}%`:`${r.recorded_days} / ${r.days} 天有记录\n${r.status==='similar'?'暂未出现明显的时段差异。':'需要至少两个时段，各有 3 天足够的判断记录。'}`;
 const populated=r.rows.flatMap(d=>d.hours.filter(h=>h.coverage_seconds>0).map(h=>h.hour));
 const min=populated.length?Math.min(...populated):8,max=populated.length?Math.max(...populated):19;
 const columns=Array.from({length:max-min+1},(_,i)=>i+min);
 const firstRow=r.rows.find(d=>d.hours.some(h=>h.coverage_seconds>0));
 const tabCell=app.primeSelected??(firstRow?{date:firstRow.date,hour:firstRow.hours.find(h=>h.coverage_seconds>0).hour}:null);
 $('#primeHeatmap').innerHTML=`<div class="prime-table" style="--hour-count:${columns.length}"><div class="prime-row prime-axis"><span>日期 / 时</span>${columns.map(h=>`<span class="${best?.hour===h?'best-hour':''}">${String(h).padStart(2,'0')}</span>`).join('')}</div>${r.rows.map(d=>`<div class="prime-row"><span>${d.date.slice(5).replace('-','/')}</span>${columns.map(h=>{
  const cell=d.hours[h],missing=cell.coverage_seconds===0,unknown=!missing&&cell.unknown_seconds===cell.coverage_seconds,selected=app.primeSelected?.date===d.date&&app.primeSelected?.hour===h;
  const text=`${d.date} ${String(h).padStart(2,'0')}:00 · ${missing?'未记录':`估算投入 ${duration(cell.work_seconds)}，已观察 ${duration(cell.coverage_seconds)}，中断 ${duration(cell.distracted_seconds)}，离开 ${duration(cell.away_seconds)}，未判断 ${duration(cell.unknown_seconds)}`}`;
  return `<button type="button" class="prime-cell ${missing?'no-data':unknown?'unknown':''} ${best?.hour===h?'best-hour':''}" tabindex="${tabCell?.date===d.date&&tabCell?.hour===h?0:-1}" style="--heat:${Math.max(.08,Math.min(1,cell.work_seconds/3600))}" data-prime-date="${d.date}" data-prime-hour="${h}" aria-label="${esc(text)}" title="${esc(text)}" aria-pressed="${selected}" ${missing?'disabled':''}></button>`;
 }).join('')}</div>`).join('')}</div>`;
 const selection=app.primeSelected,row=r.rows.find(d=>d.date===selection?.date),cell=row?.hours[selection?.hour];
 $('#primeSelection').innerHTML=cell?`<div><strong>${row.date} · ${String(cell.hour).padStart(2,'0')}:00–${String(cell.hour+1).padStart(2,'0')}:00</strong><div class="prime-detail-value">${duration(cell.work_seconds)}<span>估算投入</span></div><p>已观察 ${duration(cell.coverage_seconds)} · 中断 ${duration(cell.distracted_seconds)} · 离开 ${duration(cell.away_seconds)} · 未判断 ${duration(cell.unknown_seconds)}</p><small>${cell.eligible?'该时段参与跨日比较。':'已判断活动不足 10 分钟，暂不参与跨日比较。'}</small></div><button type="button" class="text-button" data-prime-open="${row.date}" data-prime-hour="${cell.hour}">查看当天记录 ↗</button>`:'';
 $('#primeMethod').textContent=`颜色表示每小时的估算投入分钟数。时段比较只使用每小时已判断活动不少于 10 分钟、且至少有 3 天记录的时段；按各天投入比例的平均值比较，不把未判断或离开算作中断。${r.retention_days<r.days?` 当前保留 ${r.retention_days} 天，已清理的历史不会补齐。`:''}这是当前记录中的工作规律，不是生理状态测量。`;
 if(focusDate)$(`[data-prime-date="${focusDate}"][data-prime-hour="${focusHour}"]`)?.focus({preventScroll:true});
}
function reviewMatches(s,filter){return filter==='all'||(filter==='screen'?hasScreen(s.sample):filter==='local'?s.sample.evidence?.basis==='local':s.category===filter)}
function renderReview(){const segs=groups().filter(s=>reviewMatches(s,$('#reviewFilter').value));
 const selectedSeg=segs.find(s=>s.ids.includes(app.selected))||segs[0];
 $('#filmstrip').innerHTML=segs.map(s=>`<button class="film-card ${s.ids.includes(app.selected)?'selected':''}" data-select="${esc(s.selection)}"><div class="thumb">${visual(judgment(s))}</div><div class="film-time">${timeRange(s.start,s.end)}</div><h3>${esc(description(judgment(s)))}</h3>${badge(s.category)}<small>${segmentBasis(s)}</small></button>`).join('');
 if(!selectedSeg||!segs.length){$('#reviewDetail').innerHTML=empty('还没有这类记录','开始观察后，每个片段都会保留可查看的画面与判断依据。');return}
 const s=judgment(selectedSeg),a=s.analysis,activity=selectedSeg.sample.activity||{};
 $('#reviewDetail').innerHTML=`<article class="review-image-card"><div class="section-top"><h2>${time(s.captured_at)} 的工作画面</h2><span class="eyebrow">${segmentBasis(selectedSeg)}</span></div>${hasScreen(s)?visual(s,true):`<div class="empty-visual"><p>${a?'这条记录的屏幕图片不可用':s.evidence?.basis==='local'||s.state==='local'?'这段只有本地观察记录，未采集屏幕':'这条记录没有可查看的屏幕图片'}</p></div>`}<div class="camera-detail">${presenceCard(s)}</div>${s.capture_warning?`<p class="fine">${esc(s.capture_warning)}</p>`:''}</article><article class="evidence-card">${badge(selectedSeg.category)}${selectedSeg.category==='unknown'?`<p>${esc(unknownExplanation(selectedSeg))}</p>`:''}<h2>${esc(description(s))}</h2><p>${selectedSeg.estimated_from?`这段状态沿用 ${time(s.captured_at)} 的画面判断。`:''}${esc(a?.summary||s.error||(s.state==='local'?'仅记录在座和应用活动，不据此判断工作或中断。':'该采样的分析尚未完成。'))}</p><div class="evidence-list"><div class="evidence-item"><h3>屏幕依据</h3><p>${(a?.evidence||[s.state==='local'?'这段没有上传屏幕，未进行 AI 分类。':'暂时没有分析依据']).map(esc).join('<br>')}</p></div><div class="evidence-item"><h3>应用活跃</h3><p>${esc(activity.app_name||'暂无应用记录')}${activity.switches?.length?` · 该采样区间切换 ${Math.max(activity.switches.length-(s.evidence?0:1),0)} 次`:''}</p><p>${esc(activity.window_title||'')}</p></div><div class="evidence-item"><h3>任务描述（可选）</h3><p>${esc(s.task||'未填写：按活动性质粗略判断，不判断是否推进特定任务')}</p></div></div>${hasScreen(s)?`<button class="subtle" data-retry="${esc(s.id)}">重新分析该片段</button>`:''}<p class="fine" style="margin-top:15px">状态与时长根据间歇截图和活动信号估算；每次判断从截图时刻起最多有效 2 分钟，离席、暂停或记录断档会停止延续。</p></article>`;
}
function renderInsights(){const m=app.metrics,d=app.day;const populated=m.hourly.filter(h=>h.coverage_seconds>0);const min=populated.length?Math.max(0,populated[0].hour-1):8,max=populated.length?Math.min(23,populated.at(-1).hour+1):18;
 $('#rhythmChart').innerHTML=m.hourly.filter(h=>h.hour>=min&&h.hour<=max).map(h=>`<div class="hour-column" title="${h.hour}:00 工作中 ${duration(h.work_seconds)}，中断 ${duration(h.distracted_seconds)}">${['unknown','away','distracted','work'].map(c=>`<div class="hour-part ${c}" style="height:${h[c+'_seconds']/3600*100}%"></div>`).join('')}<span class="hour-label">${String(h.hour).padStart(2,'0')}</span></div>`).join('');
 $('#periodCards').innerHTML=[['投入较连贯',m.high_period,''],['中断相对较多',m.low_period,'low']].map(([label,h,cls])=>`<div class="period ${cls}"><p>${label}</p><strong>${h?`${String(h.hour).padStart(2,'0')}:00–${String(h.hour+1).padStart(2,'0')}:00`:'待积累'}</strong><p>${h?`投入占已判断活动 ${Math.round(h.work_ratio*100)}%`:'需要两个可对比时段'}</p></div>`).join('');
 const r=d.report;$('#generateReport').disabled=r?.state==='generating';$('#generateReport').textContent=r?.state==='generating'?'正在分析…':r?'更新洞察 ↗':'生成洞察 ↗';
 $('#reportContent').innerHTML=r?.state==='done'?`<h2>${esc(r.headline)}</h2>${d.report_stale?'<p class="fine">记录或统计规则已更新，这份洞察尚未更新。</p>':''}<ul>${r.observations.map(t=>`<li>${esc(t)}</li>`).join('')}</ul><h3>下一次，可以试试</h3><ul>${r.suggestions.map(t=>`<li>${esc(t)}</li>`).join('')}</ul>`:`<h2>${r?.state==='generating'?'正在回顾这一天。':'了解状态，也找到调整方向。'}</h2><p>${esc(r?.error||'有了工作记录，就可以让 AI 结合整天的统计与代表片段，提供有依据的发现和可尝试的建议。')}</p>`;
 $('#recoveries').innerHTML=m.recoveries.length?m.recoveries.map(r=>`<article class="recovery-card"><span class="eyebrow">中断与恢复 · ${time(r.start)}–${time(r.end)}</span><div class="pair"><div class="step"><strong>出现中断</strong><p>${time(r.start)} · 连续偏离记录</p></div><span>→</span><div class="step"><strong>重新投入</strong><p>${time(r.end)} · 回到相关活动</p></div></div><p class="fine">这段中断持续约 ${duration(r.seconds)}。对照前后画面，了解你的工作如何变化。</p><div class="form-row" style="margin-top:16px"><button class="text-button" data-review="${esc(r.interruption_id)}">中断时的画面 ↗</button><button class="text-button" data-review="${esc(r.recovered_id)}">恢复后的画面 ↗</button></div></article>`).join(''):empty('还没有可对照的恢复片段','系统会在连续中断后重新出现投入记录时，保留前后画面的对照入口。');
}
function renderRecords(){$('#recordCount').textContent=`${Object.values(app.day.counts).reduce((a,b)=>a+b,0)} 条记录`;$('#recordsBody').innerHTML=app.day.segments.map(s=>`<tr><td>${timeRange(s.start,s.end)}</td><td>${badge(s.category)}</td><td>${esc(s.estimated_from?`沿用 ${time(s.estimated_from.captured_at)} 的画面判断`:description(s.sample))}</td><td>${esc(stateLabels[s.sample.state]||s.sample.state)}</td><td><button class="text-button" data-review="${esc(segmentKey(s))}">查看 ↗</button></td></tr>`).join('')}
function page(name){if(!['overview','review','insights'].includes(name))name='overview';const changed=app.page!==name;app.page=name;if(changed)window.scrollTo({top:0});for(const n of ['overview','review','insights']){$('#'+n+'Page').hidden=n!==name;$$(`[data-page="${n}"]`).forEach(el=>el.classList.toggle('active',n===name))}$('#pageTitle').textContent={overview:'工作概览',review:'画面回放',insights:'状态洞察'}[name];if(name==='overview')activityView(app.activityView);if(name==='insights'&&app.insightView==='prime')loadPrime()}
let refreshSerial=0;
async function refresh(){
 const serial=++refreshSerial;
 const [status,settings]=await Promise.all([api('/api/status'),api('/api/settings')]);
 if(serial!==refreshSerial)return;
 app.status=status;app.settings=settings;if(!app.date)app.date=status.today;
 const date=app.date;
 const view=await api(`/api/flow?date=${encodeURIComponent(date)}`);
 if(serial!==refreshSerial||date!==app.date)return;
 app.day=view.day;app.metrics=view.metrics;
 if(!app.day.segments.some(s=>segmentKey(s)===app.selected||s.sample.id===app.selected)){
  app.selected=app.day.segments.length?segmentKey(app.day.segments.at(-1)):null;app.selectedAt=null;
 }
 render();if(app.page==='insights'&&app.insightView==='prime')loadPrime();
}
// Settings explanations open on click and stay within the visible dialog.
let activeHelp=null;
function closeSettingsHelp(restoreFocus=false){
 if(!activeHelp)return;
 const {button,popover}=activeHelp;
 popover.hidden=true;
 button.setAttribute('aria-expanded','false');
 button.removeAttribute('aria-describedby');
 activeHelp=null;
 if(restoreFocus)button.focus({preventScroll:true});
}
function positionSettingsHelp(){
 if(!activeHelp)return;
 const {button,popover}=activeHelp;
 const anchor=button.getBoundingClientRect(),dialog=$('#settingsDialog').getBoundingClientRect();
 const left=Math.max(12,dialog.left+12),right=Math.min(innerWidth-12,dialog.right-12);
 const top=Math.max(12,dialog.top+12),bottom=Math.min(innerHeight-12,dialog.bottom-12);
 if(anchor.bottom<top||anchor.top>bottom){closeSettingsHelp();return}
 popover.style.width=`${Math.min(320,right-left)}px`;
 popover.style.maxHeight=`${Math.max(0,bottom-top)}px`;
 const below=bottom-anchor.bottom-8,above=anchor.top-top-8;
 const openAbove=popover.offsetHeight>below&&above>below;
 popover.style.maxHeight=`${Math.max(0,openAbove?above:below)}px`;
 popover.style.left=`${Math.max(left,Math.min(anchor.left,right-popover.offsetWidth))}px`;
 popover.style.top=`${openAbove?anchor.top-8-popover.offsetHeight:anchor.bottom+8}px`;
}
$('#settingsDialog').addEventListener('click',e=>{
 const button=e.target.closest('[data-help]');
 if(!button)return;
 const wasOpen=activeHelp?.button===button;
 closeSettingsHelp();
 if(wasOpen)return;
 const popover=document.getElementById(button.dataset.help);
 activeHelp={button,popover};
 button.setAttribute('aria-expanded','true');
 button.setAttribute('aria-describedby',popover.id);
 popover.hidden=false;
 positionSettingsHelp();
});
document.addEventListener('click',e=>{
 if(activeHelp&&!activeHelp.button.contains(e.target)&&!activeHelp.popover.contains(e.target))closeSettingsHelp();
});
document.addEventListener('keydown',e=>{
 if(e.key==='Escape'&&activeHelp){e.preventDefault();e.stopPropagation();closeSettingsHelp(true)}
},true);
document.addEventListener('focusin',e=>{
 if(activeHelp&&!activeHelp.button.contains(e.target)&&!activeHelp.popover.contains(e.target))closeSettingsHelp();
});
$('#settingsDialog').addEventListener('close',()=>closeSettingsHelp());
$('#settingsDialog').addEventListener('toggle',e=>{
 if(activeHelp&&e.target instanceof HTMLDetailsElement&&!e.target.open&&e.target.contains(activeHelp.button))closeSettingsHelp();
},true);
$('#settingsDialog').addEventListener('scroll',positionSettingsHelp);
window.addEventListener('resize',positionSettingsHelp);
window.addEventListener('resize',positionTimelineDetail);
async function openSettings(){await refresh();const f=$('#settingsForm'),s=app.settings;for(const [key,value]of Object.entries(s)){const el=f.elements.namedItem(key);if(!el)continue;if(el.type==='checkbox')el.checked=value;else el.value=Array.isArray(value)?value.join('\n'):value}f.elements.api_key.value='';$('#modelPreset').value='';$('#modelSummary').textContent=s.model;$('#keyStatus').textContent=s.api_key_configured?'已配置密钥。':'尚未配置密钥。';$('#settingsError').textContent='';$('#testResult').textContent='';$('#settingsDialog').showModal()}
function updateDisplayCount(){const choices=$$('#displayChoices input');const count=choices.filter(el=>el.checked).length;$('#displayCount').textContent=`已选 ${count} / ${choices.length} 块`;$('#allDisplays').checked=choices.length>0&&count===choices.length;$('#allDisplays').indeterminate=count>0&&count<choices.length;$('#allDisplays').disabled=!choices.length}
$('#allDisplays').onchange=e=>{$$('#displayChoices input').forEach(el=>el.checked=e.target.checked);updateDisplayCount()};
$('#displayChoices').onchange=updateDisplayCount;
async function loadPermissions(reset=false){
 const r=await api('/api/recorder');app.permissions=r.permissions;const p=r.permissions;
 $('#permissions').innerHTML=[['screen','屏幕录制',p.screen_permission?'已授权':'未授权'],['camera','摄像头',p.camera_permission==='authorized'?'已授权':p.camera_permission==='not_determined'?'未授权':'不可用 / 未授权']].map(([kind,name,label])=>`<div class="permission-row"><div>${name} <span>${label}</span></div><button type="button" class="subtle" data-permission="${kind}">${label==='已授权'?'刷新状态':'前往授权'}</button></div>`).join('');
 const selected=new Set($$('#displayChoices input:checked').map(el=>Number(el.value)));
 $('#displayChoices').innerHTML=(p.displays||[]).map(d=>`<label class="checkbox-line"><input type="checkbox" name="display_ids" value="${d.id}" ${reset||selected.has(d.id)?'checked':''}>${esc(d.name)}${d.primary?' · 主显示器':''}</label>`).join('')||'<p class="fine">暂无可用显示器，请刷新设备状态。</p>';
 updateDisplayCount();if(p.error)$('#startError').textContent=p.error;
}

async function record(){if(app.status.recorder.running){await api('/api/recorder/stop','POST',{});toast('已暂停');await refresh();return}$('#startError').textContent='';$('#startTask').value=app.settings.task;$('#displayChoices').innerHTML='';updateDisplayCount();$('#startDialog').showModal();await loadPermissions(true)}
document.addEventListener('click',e=>{
 const b=e.target.closest('button');if(!b)return;
 if(b.dataset.activityView){activityView(b.dataset.activityView);return;}
 if(b.dataset.insightView){insightView(b.dataset.insightView);return;}
 if(b.dataset.rhythmMinute){
  const cell=rhythmCells.get(Number(b.dataset.rhythmMinute));if(!cell)return;
  const rect=b.getBoundingClientRect();
  const part=e.detail===0?cell.parts.find(p=>p.key):FlowRhythm.hit(cell,(e.clientX-rect.left)/rect.width);
  if(!part?.key){toast('这一小段没有记录。');return;}
  app.selected=part.key;app.selectedAt=part.start;
  renderTimeline();renderWorkRhythm();renderReview();return;
 }
 if(b.dataset.primeDate){app.primeSelected={date:b.dataset.primeDate,hour:Number(b.dataset.primeHour)};renderPrime();return;}
 if(b.dataset.primeOpen){
  const row=app.prime?.rows.find(d=>d.date===b.dataset.primeOpen),cell=row?.hours[Number(b.dataset.primeHour)];
  if(!cell?.sample_key)return;
  action(async()=>{
   app.date=row.date;app.selected=cell.sample_key;app.selectedAt=cell.sample_at;
   await refresh();activityView('rhythm');page('overview');location.hash='overview';
   $('#workRhythmPanel').scrollIntoView({behavior:'smooth',block:'start'});
  },b);
 }
});
document.addEventListener('keydown',e=>{
 const cell=e.target.closest('.work-minute,.prime-cell');
 if(cell&&['ArrowLeft','ArrowRight','ArrowUp','ArrowDown','Home','End'].includes(e.key)){
  const grid=cell.closest('#workRhythmGrid,#primeHeatmap'),cells=[...grid.querySelectorAll('button')];
  const columns=cell.parentElement.querySelectorAll('button').length,index=cells.indexOf(cell);
  const step=e.key==='ArrowDown'?columns:e.key==='ArrowUp'?-columns:e.key==='ArrowRight'||e.key==='Home'?1:-1;
  let next=e.key==='Home'?0:e.key==='End'?cells.length-1:index+step;
  while(next>=0&&next<cells.length&&cells[next].disabled)next+=step;
  e.preventDefault();if(cells[next]){cells[next].focus();cells[next].click();}return;
 }
 const tab=e.target.closest('.view-switch [role="tab"]');if(!tab||!['ArrowLeft','ArrowRight','Home','End'].includes(e.key))return;
 const tabs=[...tab.parentElement.querySelectorAll('[role="tab"]')],index=tabs.indexOf(tab);
 const next=e.key==='Home'?0:e.key==='End'?tabs.length-1:(index+(e.key==='ArrowRight'?1:-1)+tabs.length)%tabs.length;
 e.preventDefault();tabs[next].focus();tabs[next].click();
});
$('#primeDays').onchange=()=>{app.primeDays=Number($('#primeDays').value);loadPrime();};
$('#refreshPrime').onclick=()=>loadPrime(true);
document.addEventListener('click',e=>{const b=e.target.closest('button');if(!b)return;if(b.dataset.close){$('#'+b.dataset.close).close();return}if(b.dataset.page){const h=b.dataset.page;if(location.hash.slice(1)!==h)location.hash=h;else page(h);return}if(b.dataset.screenId){selectedScreens.set(b.dataset.screenSample,Number(b.dataset.screenId));renderReview();return}if(b.dataset.select){app.selected=b.dataset.select;app.selectedAt=null;renderTimeline();renderWorkRhythm();renderReview();return}if(b.dataset.review){if(app.selected!==b.dataset.review)app.selectedAt=null;app.selected=b.dataset.review;$('#reviewFilter').value='all';page('review');renderReview();window.scrollTo({top:0,behavior:'smooth'});return}if(b.dataset.retry){action(async()=>{await api(`/api/samples/${encodeURIComponent(b.dataset.retry)}/analyze`,'POST',{});toast('已重新加入分析队列');await refresh()},b);return}if(b.dataset.permission){action(async()=>{await api('/api/recorder/permissions','POST',{kind:b.dataset.permission});await loadPermissions()},b);return}if(b.dataset.action==='settings')action(openSettings,b);if(b.dataset.action==='record')action(record,b);if(b.dataset.action==='preview')action(async()=>{await api('/api/reminders/preview','POST',{});toast('原生桌面卡片已弹出，25 秒后收起。')},b)});
$('#latestVisual').onclick=()=>{if(app.day.segments.length){app.selectedAt=null;app.selected=[...app.day.segments].reverse().find(s=>hasScreen(s.sample))?.sample.id||app.day.segments.at(-1).sample.id;$('#reviewFilter').value='all';page('review');renderReview()}};
$('#date').onchange=()=>action(async()=>{app.date=$('#date').value;app.selected=null;app.selectedAt=null;await refresh()});
$('#reviewFilter').onchange=()=>{const value=$('#reviewFilter').value;const s=groups().find(s=>reviewMatches(s,value));app.selected=s?.selection;app.selectedAt=null;renderReview()};
$('#settingsForm').elements.report_hour.innerHTML=Array.from({length:24},(_,i)=>`<option value="${i}">${String(i).padStart(2,'0')}:00</option>`).join('');
$('#settingsForm').onsubmit=async e=>{e.preventDefault();const f=e.target,button=f.querySelector('[type=submit]');button.disabled=true;try{const s={...app.settings};delete s.api_key_configured;for(const k of ['base_url','model','api_key','task'])s[k]=f.elements[k].value;for(const k of ['report_hour','daily_goal_minutes','retention_days','reminder_minutes'])s[k]=Number(f.elements[k].value);s.reminders=f.elements.reminders.checked;s.excluded_apps=f.elements.excluded_apps.value.split('\n').map(x=>x.trim()).filter(Boolean);await api('/api/settings','PUT',s);$('#settingsDialog').close();toast('设置已保存');await refresh()}catch(e){$('#settingsError').textContent=e.message}finally{button.disabled=false}};
$('#startForm').onsubmit=async e=>{e.preventDefault();const b=e.target.querySelector('[type=submit]');b.disabled=true;try{const displayIds=$$('#displayChoices input:checked').map(el=>Number(el.value));if(!displayIds.length)throw Error('请至少选择一块显示器');if($('#startTask').value.trim()!==app.settings.task){const settings={...app.settings,task:$('#startTask').value.trim()};delete settings.api_key_configured;await api('/api/settings','PUT',settings)}await api('/api/recorder/start','POST',{camera:$('#useCamera').checked,display_ids:displayIds});$('#startDialog').close();await refresh();toast('后台记录已开始，可以关闭网页继续工作。')}catch(e){$('#startError').textContent=e.message}finally{b.disabled=false}};
$('#testModel').onclick=e=>action(async()=>{$('#testResult').textContent='正在发送合成测试图片…';try{const r=await api('/api/settings/test','POST',{});$('#testResult').textContent=r.message}catch(err){$('#testResult').textContent=err.message}},e.currentTarget);
$('#snooze').onclick=e=>action(async()=>{await api('/api/reminders/snooze','POST',{minutes:15});toast('接下来 15 分钟保持安静。')},e.currentTarget);
$('#generateReport').onclick=e=>action(async()=>{await api('/api/report','POST',{date:app.date});await refresh()},e.currentTarget);
$('#retryPending').onclick=e=>action(async()=>{const pending=app.day.segments.filter(s=>['pending','error'].includes(s.sample.state)).slice(0,8);for(const s of pending)await api(`/api/samples/${encodeURIComponent(s.sample.id)}/analyze`,'POST',{});toast(pending.length?`已提交 ${pending.length} 个片段，请稍后查看。`:'没有待重试的记录');await refresh()},e.currentTarget);
$('#export').onclick=()=>{const a=document.createElement('a');a.href=`/api/export?date=${encodeURIComponent(app.date)}`;a.download=`flow-insight-${app.date}.json`;document.body.appendChild(a);a.click();a.remove();toast('已请求下载当天的 JSON 记录。')};
$('#deleteDay').onclick=e=>{if(confirm(`删除 ${app.date} 的记录、画面与对应分析？此操作不能撤销。`))action(async()=>{const r=await api('/api/data/delete-day','POST',{date:app.date});invalidatePrime();toast(`已删除 ${r.deleted} 个采样`);$('#settingsDialog').close();await refresh()},e.currentTarget)};
$('#resetData').onclick=()=>{const input=$('#resetConfirmation');input.value='';$('#resetError').textContent='';$('#confirmReset').disabled=true;$('#resetDialog').showModal();input.focus()};
$('#resetConfirmation').oninput=e=>{$('#confirmReset').disabled=e.target.value.trim()!=='清空全部数据'};
$('#resetForm').onsubmit=async e=>{e.preventDefault();const button=$('#confirmReset'),confirmation=$('#resetConfirmation').value;button.disabled=true;try{const r=await api('/api/data/reset','POST',{confirmation});invalidatePrime();app.date=app.status.today;app.selected=null;page('overview');$('#resetDialog').close();$('#settingsDialog').close();await refresh();toast(`已清空 ${r.deleted.samples} 个采样和 ${r.deleted.files} 个画面，设置已保留。`)}catch(err){$('#resetError').textContent=err.message}finally{button.disabled=$('#resetConfirmation').value.trim()!=='清空全部数据'}};
window.addEventListener('hashchange',()=>{const [p,id]=location.hash.slice(1).split('/');if(id){app.selected=decodeURIComponent(id);app.selectedAt=null;}page(p);if(app.day)renderReview()});
let polling=false;async function poll(){if(polling)return;polling=true;try{await refresh()}catch(e){$('#errorBanner').hidden=false;$('#errorBanner').textContent='无法连接本地后台：'+e.message;renderHero({...app,connectionLost:true});renderCameraTile({...app,connectionLost:true});$('#recordBadge span').textContent='连接中断';$('#recordBadge').classList.remove('running')}finally{polling=false}}
$('#cameraToggle').onclick=()=>{previewOn=!previewOn;try{localStorage.setItem('cameraPreview',previewOn?'on':'off')}catch{}renderCameraTile();previewTick()};
document.addEventListener('visibilitychange',previewTick);
(async()=>{await poll();const [p,id]=location.hash.slice(1).split('/');if(id){app.selected=decodeURIComponent(id);app.selectedAt=null;}page(p);if(app.day)renderReview();setInterval(poll,5000);setInterval(previewTick,500)})();

$('#modelPreset').onchange=e=>{const presets={deepseek:['https://api.deepseek.com','deepseek-flash'],qwen:['https://dashscope.aliyuncs.com/compatible-mode/v1','qwen3.8-flash']};const preset=presets[e.target.value];if(!preset)return;const f=$('#settingsForm');[f.elements.base_url.value,f.elements.model.value]=preset;f.elements.api_key.value='';$('#modelSummary').textContent=preset[1];$('#testResult').textContent='请先保存设置，再测试图片接口。';$('#keyStatus').textContent='更换服务时，请填写该服务的 API Key。';};
