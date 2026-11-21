const invoke = window.__TAURI__.core.invoke;
const $ = id => document.getElementById(id);
let snapshot, expanded = new Set(), confirming = null, toastTimer;
const fmtGB = bytes => (bytes / 1073741824).toLocaleString('pl-PL', {maximumFractionDigits:1,minimumFractionDigits:1}) + ' GB';
const fmtMem = bytes => bytes >= 1073741824 ? fmtGB(bytes) : Math.round(bytes / 1048576) + ' MB';
const uptime = sec => { const h=Math.floor(sec/3600), m=Math.floor((sec%3600)/60); return h ? `${h} h ${m} min` : `${m} min`; };
const text = (tag, value, className) => { const el=document.createElement(tag); el.textContent=value; if(className) el.className=className; return el; };
function toast(message) { $('toast').textContent=message; $('toast').classList.add('show'); clearTimeout(toastTimer); toastTimer=setTimeout(()=>$('toast').classList.remove('show'),3500); }
async function kill(pids) { try { const r=await invoke('kill_processes',{pids}); toast(r.failed ? `Ubito ${r.killed} z ${r.killed+r.failed}. ${r.errors.join('; ')}` : `Ubito ${r.killed} z ${r.killed}`); confirming=null; await refresh(); } catch(e) { toast(String(e)); } }
function actionButton(pids, bytes, key, host) {
  if(confirming===key) { const row=text('div',`Na pewno? (${pids.length} proc., ${fmtGB(bytes)})`,'confirmation'); const yes=text('button','Tak','yes'), no=text('button','Nie','no'); yes.onclick=e=>{e.stopPropagation();kill(pids)}; no.onclick=e=>{e.stopPropagation();confirming=null;renderGroups()}; row.append(yes,no); host.append(row); }
  else { const btn=text('button','Ubij','kill'); btn.onclick=e=>{e.stopPropagation();confirming=key;renderGroups()}; host.append(btn); }
}
function renderGroups() {
  const root=$('groups'); root.replaceChildren();
  for(const g of snapshot.groups) {
    const card=text('div','','group'), main=text('div','','group-main');
    const expand=text('button',expanded.has(g.name)?'⌄':'›','expand'); expand.setAttribute('aria-label',`Rozwiń ${g.name}`); expand.onclick=()=>{expanded.has(g.name)?expanded.delete(g.name):expanded.add(g.name);renderGroups()};
    main.append(expand,text('span',g.name,'name'),text('span',fmtMem(g.bytes),'value'));
    if(g.protected) main.append(text('span','systemowy','system')); else if(confirming!==`g:${g.name}`) actionButton(g.pids.map(p=>p.pid),g.bytes,`g:${g.name}`,main);
    card.append(main);
    const meta=text('div','','meta'); meta.append(text('span',`${g.count} proc.  ·  ${uptime(g.uptimeSecs)}`)); card.append(meta);
    if(confirming===`g:${g.name}`) actionButton(g.pids.map(p=>p.pid),g.bytes,`g:${g.name}`,card);
    if(expanded.has(g.name)) { const pids=text('div','','pids'); for(const p of g.pids) { const row=text('div','','pid-row'); row.append(text('span',`PID ${p.pid} · ${uptime(p.uptimeSecs)}`),text('span',fmtMem(p.bytes))); if(!g.protected && confirming!==`p:${p.pid}`) actionButton([p.pid],p.bytes,`p:${p.pid}`,row); pids.append(row); if(confirming===`p:${p.pid}`) actionButton([p.pid],p.bytes,`p:${p.pid}`,pids); } card.append(pids); }
    root.append(card);
  }
}
function render(s) {
  snapshot=s; const colors={green:'#22c55e',yellow:'#facc15',orange:'#f97316',red:'#ef4444'}; const color=colors[s.level];
  $('dot').style.background=color; $('fill').style.background=color; $('fill').style.width=`${s.usedPercent}%`;
  $('level').textContent=s.label; $('advice').textContent=s.advice;
  $('used').textContent=`${fmtGB(s.totalBytes-s.availableBytes)} / ${fmtGB(s.totalBytes)} (${Math.round(s.usedPercent)}%)`;
  $('available').textContent=`Wolne ${fmtGB(s.availableBytes)}`; $('commit').textContent=`Commit ${Math.round(s.commitPercent)}%`;
  const t=s.trend; $('trend').textContent=t.state==='collecting'?'Zbieram dane…':t.state==='rising'?`Przyrost ~${Math.round(t.mbPerMin)} MB/min · RAM skończy się za ~${Math.ceil(t.minutesToRed)} min`:t.state==='falling'?'Spada zużycie':'Stabilnie';
  $('reserve').textContent=`Rezerwa ${fmtGB(s.reserveBytes)}`; $('reserve-detail').textContent=fmtGB(s.reserveBytes);
  renderGroups();
}
async function refresh() { try { render(await invoke('get_snapshot')); } catch(e) { toast(String(e)); } }
$('close').onclick=()=>invoke('close_popup');
document.addEventListener('keydown',e=>{if(e.key==='Escape')invoke('close_popup')});
$('settings-toggle').onclick=async()=>{ $('settings').classList.toggle('hidden'); if(!$('settings').classList.contains('hidden')) $('slot').value=(await invoke('get_settings')).slotGb; };
$('save').onclick=async()=>{try {await invoke('set_settings',{slotGb:Number($('slot').value)});toast('Ustawienia zapisane');$('settings').classList.add('hidden');refresh();}catch(e){toast(String(e))}};
refresh(); setInterval(refresh,2000);
