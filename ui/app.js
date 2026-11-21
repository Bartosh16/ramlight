const invoke = window.__TAURI__.core.invoke;
const $ = (id) => document.getElementById(id);

let snapshot;
let expanded = new Set();
let confirming = null; // klucz wiersza czekajacego na "Tak/Nie": g:nazwa, p:pid albo "bulk"
let toastTimer;

const LEVEL_COLORS = { green: '#22c55e', yellow: '#facc15', orange: '#f97316', red: '#ef4444' };
const CATEGORY = {
  safe: { label: 'bezpieczny', ask: 'Ubić?' },
  risky: { label: 'ryzyko', ask: 'Może mieć niezapisane zmiany. Ubić?' },
  keep: { label: 'nie ubijaj', ask: 'Stracisz pracę w toku. Na pewno?' },
  essential: { label: 'system', ask: '' },
  unknown: { label: 'nieznany', ask: 'Nie wiem, co to jest. Na pewno?' },
};

const fmtGB = (bytes) => (bytes / 1073741824).toLocaleString('pl-PL', { maximumFractionDigits: 1, minimumFractionDigits: 1 }) + ' GB';
const fmtMem = (bytes) => (bytes >= 1073741824 ? fmtGB(bytes) : Math.round(bytes / 1048576) + ' MB');
const uptime = (sec) => {
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  return h ? `${h} h ${m} min` : `${m} min`;
};
const el = (tag, text, className) => {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  return node;
};

function toast(message) {
  $('toast').textContent = message;
  $('toast').classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => $('toast').classList.remove('show'), 4000);
}

function reportKill(r) {
  const total = r.killed + r.failed;
  toast(r.failed ? `Ubito ${r.killed} z ${total}. ${r.errors.join('; ')}` : `Ubito ${r.killed} z ${total}`);
}

async function kill(pids) {
  try {
    reportKill(await invoke('kill_processes', { pids }));
  } catch (e) {
    toast(String(e));
  }
  confirming = null;
  await refresh();
}

async function killAllSafe() {
  try {
    reportKill(await invoke('kill_all_safe'));
  } catch (e) {
    toast(String(e));
  }
  confirming = null;
  await refresh();
}

// Pasek potwierdzenia: pytanie + Tak/Nie. Klik poza wierszem niczego nie rozwija.
function confirmRow(question, onYes) {
  const row = el('div', undefined, 'confirmation');
  row.append(el('span', question));
  const yes = el('button', 'Tak', 'yes');
  const no = el('button', 'Nie', 'no');
  yes.onclick = (e) => { e.stopPropagation(); onYes(); };
  no.onclick = (e) => { e.stopPropagation(); confirming = null; render(snapshot); };
  row.append(yes, no);
  return row;
}

function killButton(key) {
  const btn = el('button', 'Ubij', 'kill');
  btn.onclick = (e) => { e.stopPropagation(); confirming = key; render(snapshot); };
  return btn;
}

function renderBulk() {
  const root = $('bulk');
  root.replaceChildren();
  const safe = snapshot.safe;
  if (confirming === 'bulk') {
    const names = safe.names.slice(0, 6).join(', ') + (safe.names.length > 6 ? '…' : '');
    root.append(confirmRow(`Ubiję ${names} (${safe.count} proc., ${fmtGB(safe.bytes)}). Sprawdź, czy nic się nie pobiera.`, killAllSafe));
    return;
  }
  const btn = el('button', undefined, 'bulk-btn');
  btn.disabled = safe.count === 0;
  btn.append(el('span', safe.count ? 'Ubij wszystkie zbędne' : 'Brak zbędnych procesów'));
  if (safe.count) btn.append(el('strong', fmtGB(safe.bytes)));
  btn.title = 'Tylko zielone: przeglądarki, komunikatory, launchery. Żółtych, pomarańczowych, szarych i systemowych nie rusza.';
  btn.onclick = () => { confirming = 'bulk'; render(snapshot); };
  root.append(btn);
}

function renderGroups() {
  const root = $('groups');
  root.replaceChildren();
  for (const g of snapshot.groups) {
    const card = el('div', undefined, `group cat-${g.category}`);
    const main = el('div', undefined, 'group-main');
    const expand = el('button', expanded.has(g.name) ? '⌄' : '›', 'expand');
    expand.setAttribute('aria-label', `Rozwiń ${g.name}`);
    expand.onclick = () => {
      expanded.has(g.name) ? expanded.delete(g.name) : expanded.add(g.name);
      render(snapshot);
    };
    main.append(expand, el('span', g.name, 'name'), el('span', fmtMem(g.bytes), 'value'));
    const groupKey = `g:${g.name}`;
    if (g.category === 'essential') main.append(el('span', 'system', 'locked'));
    else if (confirming !== groupKey) main.append(killButton(groupKey));
    card.append(main);

    const meta = el('div', undefined, 'meta');
    meta.append(el('span', CATEGORY[g.category].label, `chip c-${g.category}`), el('span', `${g.count} proc. · ${uptime(g.uptimeSecs)} · ${g.hint}`));
    meta.title = g.hint;
    card.append(meta);

    if (confirming === groupKey) {
      card.append(confirmRow(`${CATEGORY[g.category].ask} (${g.count} proc., ${fmtMem(g.bytes)})`, () => kill(g.pids.map((p) => p.pid))));
    }

    if (expanded.has(g.name)) {
      const list = el('div', undefined, 'pids');
      for (const p of g.pids) {
        const pidKey = `p:${p.pid}`;
        const row = el('div', undefined, 'pid-row');
        row.append(el('span', `PID ${p.pid} · ${uptime(p.uptimeSecs)}`), el('span', fmtMem(p.bytes)));
        if (g.category !== 'essential' && confirming !== pidKey) row.append(killButton(pidKey));
        list.append(row);
        if (confirming === pidKey) list.append(confirmRow(`${CATEGORY[g.category].ask} (PID ${p.pid})`, () => kill([p.pid])));
      }
      card.append(list);
    }
    root.append(card);
  }
}

function render(s) {
  snapshot = s;
  const color = LEVEL_COLORS[s.level];
  $('dot').style.background = color;
  $('fill').style.background = color;
  $('fill').style.width = `${s.usedPercent}%`;
  $('level').textContent = s.label;
  $('advice').textContent = s.advice;
  $('used').textContent = `${fmtGB(s.totalBytes - s.availableBytes)} / ${fmtGB(s.totalBytes)} (${Math.round(s.usedPercent)}%)`;
  $('available').textContent = `Wolne ${fmtGB(s.availableBytes)}`;
  $('commit').textContent = `Commit ${Math.round(s.commitPercent)}%`;
  const t = s.trend;
  $('trend').textContent =
    t.state === 'collecting' ? 'Zbieram dane…'
    : t.state === 'rising' ? `Przyrost ~${Math.round(t.mbPerMin)} MB/min · RAM skończy się za ~${Math.ceil(t.minutesToRed)} min`
    : t.state === 'falling' ? 'Spada zużycie'
    : 'Stabilnie';
  $('reserve').textContent = `Rezerwa ${fmtGB(s.reserveBytes)}`;
  $('reserve-detail').textContent = fmtGB(s.reserveBytes);
  renderBulk();
  renderGroups();
}

async function refresh() {
  // W trakcie potwierdzania nie przerysowujemy listy, zeby wiersz nie uciekl spod kursora.
  if (confirming && snapshot) return;
  try {
    render(await invoke('get_snapshot'));
  } catch (e) {
    toast(String(e));
  }
}

$('close').onclick = () => invoke('close_popup');
document.addEventListener('keydown', (e) => {
  if (e.key !== 'Escape') return;
  if (confirming) { confirming = null; render(snapshot); } else invoke('close_popup');
});
$('settings-toggle').onclick = async () => {
  $('settings').classList.toggle('hidden');
  if (!$('settings').classList.contains('hidden')) $('slot').value = (await invoke('get_settings')).slotGb;
};
$('save').onclick = async () => {
  try {
    await invoke('set_settings', { slotGb: Number($('slot').value) });
    toast('Ustawienia zapisane');
    $('settings').classList.add('hidden');
    refresh();
  } catch (e) {
    toast(String(e));
  }
};
refresh();
setInterval(refresh, 2000);
