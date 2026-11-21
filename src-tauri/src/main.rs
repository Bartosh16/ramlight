#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
use tauri::{
    menu::{CheckMenuItem, MenuBuilder, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder,
};
use tauri_plugin_autostart::ManagerExt;
use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};

const GIB: f64 = 1073741824.0;
static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Level {
    Green,
    Yellow,
    Orange,
    Red,
}

impl Level {
    fn rank(self) -> u8 {
        match self {
            Self::Green => 0,
            Self::Yellow => 1,
            Self::Orange => 2,
            Self::Red => 3,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Green => "Czysto",
            Self::Yellow => "Zmieści się jeden proces",
            Self::Orange => "Kolejny proces = ryzyko",
            Self::Red => "Niczego nie dokładaj",
        }
    }
    fn advice(self) -> &'static str {
        match self {
            Self::Green => "Masz zapas na kilka zadań",
            Self::Yellow => "Możesz dołożyć jeden proces",
            Self::Orange => "Zamknij coś przed kolejnym zadaniem",
            Self::Red => "Zwolnij pamięć przed dalszą pracą",
        }
    }
    fn color(self) -> [u8; 3] {
        match self {
            Self::Green => [0x22, 0xc5, 0x5e],
            Self::Yellow => [0xfa, 0xcc, 0x15],
            Self::Orange => [0xf9, 0x73, 0x16],
            Self::Red => [0xef, 0x44, 0x44],
        }
    }
}

fn reserve(total: u64) -> u64 {
    (total as f64 * 0.08).max(GIB) as u64
}
fn level_for(available: u64, total: u64, slot: u64, commit: f64) -> Level {
    let r = reserve(total);
    let base = if available >= r.saturating_add(slot.saturating_mul(2)) {
        Level::Green
    } else if available >= r.saturating_add(slot) {
        Level::Yellow
    } else if available >= r {
        Level::Orange
    } else {
        Level::Red
    };
    let minimum = if commit >= 97.0 {
        Level::Red
    } else if commit >= 90.0 {
        Level::Orange
    } else {
        Level::Green
    };
    if base.rank() >= minimum.rank() {
        base
    } else {
        minimum
    }
}

fn protected(name: &str, pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    const NAMES: &[&str] = &[
        "system",
        "registry",
        "smss.exe",
        "csrss.exe",
        "wininit.exe",
        "winlogon.exe",
        "services.exe",
        "lsass.exe",
        "svchost.exe",
        "dwm.exe",
        "explorer.exe",
        "fontdrvhost.exe",
        "memory compression",
        "sihost.exe",
        "ctfmon.exe",
        "msmpeng.exe",
        "audiodg.exe",
        "spoolsv.exe",
        "wmiprvse.exe",
        "securityhealthservice.exe",
        "ramlight.exe",
        "ramlight",
    ];
    NAMES.iter().any(|n| name.eq_ignore_ascii_case(n))
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    slot_gb: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self { slot_gb: 2.0 }
    }
}
fn settings_path() -> PathBuf {
    PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
        .join("RamLight")
        .join("settings.json")
}
fn load_settings() -> Settings {
    fs::read(settings_path())
        .ok()
        .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
        .filter(|s| (0.5..=16.0).contains(&s.slot_gb))
        .unwrap_or_default()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Trend {
    state: &'static str,
    mb_per_min: f64,
    minutes_to_red: Option<f64>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PidRow {
    pid: u32,
    bytes: u64,
    uptime_secs: u64,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Group {
    name: String,
    bytes: u64,
    count: usize,
    uptime_secs: u64,
    protected: bool,
    pids: Vec<PidRow>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    level: Level,
    label: &'static str,
    advice: &'static str,
    total_bytes: u64,
    available_bytes: u64,
    used_percent: f64,
    commit_percent: f64,
    trend: Trend,
    groups: Vec<Group>,
    reserve_bytes: u64,
    slot_bytes: u64,
}
#[derive(Serialize)]
struct KillResult {
    killed: usize,
    failed: usize,
    errors: Vec<String>,
}

struct MemoryState {
    total: u64,
    available: u64,
    commit: f64,
    level: Level,
    candidate: Level,
    consecutive: u8,
    short: VecDeque<u64>,
    history: VecDeque<(Instant, u64)>,
    settings: Settings,
}
impl MemoryState {
    fn new() -> Self {
        Self {
            total: 0,
            available: 0,
            commit: 0.0,
            level: Level::Green,
            candidate: Level::Green,
            consecutive: 0,
            short: VecDeque::new(),
            history: VecDeque::new(),
            settings: load_settings(),
        }
    }
    fn sample(&mut self, total: u64, available: u64, commit: f64) {
        self.total = total;
        self.commit = commit;
        self.short.push_back(available);
        if self.short.len() > 3 {
            self.short.pop_front();
        }
        self.available = self.short.iter().copied().sum::<u64>() / self.short.len() as u64;
        let now = Instant::now();
        self.history.push_back((now, self.available));
        while self
            .history
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t) > Duration::from_secs(300))
        {
            self.history.pop_front();
        }
        let candidate = level_for(
            self.available,
            total,
            (self.settings.slot_gb * GIB) as u64,
            commit,
        );
        if candidate == self.level {
            self.consecutive = 0;
        } else {
            if candidate == self.candidate {
                self.consecutive += 1;
            } else {
                self.candidate = candidate;
                self.consecutive = 1;
            }
            if self.consecutive >= 2 {
                self.level = candidate;
                self.consecutive = 0;
            }
        }
    }
    fn trend(&self) -> Trend {
        let Some((first, _)) = self.history.front() else {
            return Trend {
                state: "collecting",
                mb_per_min: 0.0,
                minutes_to_red: None,
            };
        };
        if first.elapsed() < Duration::from_secs(60) {
            return Trend {
                state: "collecting",
                mb_per_min: 0.0,
                minutes_to_red: None,
            };
        }
        let n = self.history.len() as f64;
        let points: Vec<(f64, f64)> = self
            .history
            .iter()
            .map(|(t, b)| {
                (
                    t.duration_since(*first).as_secs_f64() / 60.0,
                    *b as f64 / 1_000_000.0,
                )
            })
            .collect();
        let mx = points.iter().map(|p| p.0).sum::<f64>() / n;
        let my = points.iter().map(|p| p.1).sum::<f64>() / n;
        let den = points.iter().map(|p| (p.0 - mx).powi(2)).sum::<f64>();
        let slope = if den > 0.0 {
            points.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum::<f64>() / den
        } else {
            0.0
        };
        if slope < -20.0 {
            Trend {
                state: "rising",
                mb_per_min: -slope,
                minutes_to_red: Some(
                    ((self.available.saturating_sub(reserve(self.total))) as f64
                        / 1_000_000.0
                        / -slope)
                        .max(0.0),
                ),
            }
        } else if slope > 20.0 {
            Trend {
                state: "falling",
                mb_per_min: slope,
                minutes_to_red: None,
            }
        } else {
            Trend {
                state: "stable",
                mb_per_min: slope,
                minutes_to_red: None,
            }
        }
    }
}
struct AppState {
    memory: Arc<Mutex<MemoryState>>,
    processes: Mutex<(System, Option<Instant>)>,
}

fn commit_percent() -> f64 {
    let mut info = PERFORMANCE_INFORMATION::default();
    unsafe {
        if GetPerformanceInfo(
            &mut info,
            std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32,
        )
        .is_ok()
            && info.CommitLimit > 0
        {
            100.0 * info.CommitTotal as f64 / info.CommitLimit as f64
        } else {
            0.0
        }
    }
}

fn icon(level: Level) -> tauri::image::Image<'static> {
    let mut rgba = vec![0u8; 32 * 32 * 4];
    let c = level.color();
    for y in 0..32 {
        for x in 0..32 {
            let d = (((x as f64 + 0.5) - 16.0).powi(2) + ((y as f64 + 0.5) - 16.0).powi(2)).sqrt();
            let a = ((15.0 - d) * 255.0).clamp(0.0, 255.0) as u8;
            let rgb = if d > 12.5 { [0x0a, 0x0d, 0x12] } else { c };
            let i = (y * 32 + x) * 4;
            rgba[i..i + 3].copy_from_slice(&rgb);
            rgba[i + 3] = a;
        }
    }
    tauri::image::Image::new_owned(rgba, 32, 32)
}

// Klik w ikone zabiera fokus popupowi, wiec zamyka go blur tuz przed klikiem.
// Pamietamy moment zamkniecia, zeby ten sam klik nie otworzyl okna ponownie.
static LAST_CLOSED: Mutex<Option<Instant>> = Mutex::new(None);
fn destroy_popup(w: &tauri::WebviewWindow) {
    *LAST_CLOSED.lock().unwrap() = Some(Instant::now());
    let _ = w.destroy();
}

fn toggle_popup(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("popup") {
        destroy_popup(&w);
        return;
    }
    if LAST_CLOSED
        .lock()
        .unwrap()
        .is_some_and(|t| t.elapsed() < Duration::from_millis(400))
    {
        return;
    }
    open_popup(app);
}

fn open_popup(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("popup") {
        let _ = w.set_focus();
        return;
    }
    let builder = WebviewWindowBuilder::new(app, "popup", WebviewUrl::App("index.html".into()))
        .title("RamLight")
        .inner_size(380.0, 560.0)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .visible(false);
    if let Ok(window) = builder.build() {
        if let Ok(Some(monitor)) = window
            .current_monitor()
            .or_else(|_| window.primary_monitor())
        {
            let work = monitor.work_area();
            let scale = monitor.scale_factor();
            let width = (380.0 * scale) as i32;
            let height = (560.0 * scale) as i32;
            let margin = (12.0 * scale) as i32;
            let _ = window.set_position(PhysicalPosition::new(
                work.position.x + work.size.width as i32 - width - margin,
                work.position.y + work.size.height as i32 - height - margin,
            ));
        }
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[tauri::command]
fn close_popup(app: AppHandle) {
    if let Some(w) = app.get_webview_window("popup") {
        destroy_popup(&w);
    }
}

#[tauri::command]
fn get_settings(state: tauri::State<'_, AppState>) -> Settings {
    state.memory.lock().unwrap().settings.clone()
}
#[tauri::command]
fn set_settings(state: tauri::State<'_, AppState>, slot_gb: f64) -> Result<Settings, String> {
    if !slot_gb.is_finite() || !(0.5..=16.0).contains(&slot_gb) {
        return Err("Dozwolony zakres: 0,5–16 GB".into());
    }
    let settings = Settings { slot_gb };
    let path = settings_path();
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(
        path,
        serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    state.memory.lock().unwrap().settings = settings.clone();
    Ok(settings)
}

#[tauri::command]
fn get_snapshot(state: tauri::State<'_, AppState>) -> Snapshot {
    let m = state.memory.lock().unwrap();
    let mut snapshot = Snapshot {
        level: m.level,
        label: m.level.label(),
        advice: m.level.advice(),
        total_bytes: m.total,
        available_bytes: m.available,
        used_percent: if m.total > 0 {
            100.0 * (1.0 - m.available as f64 / m.total as f64)
        } else {
            0.0
        },
        commit_percent: m.commit,
        trend: m.trend(),
        groups: Vec::new(),
        reserve_bytes: reserve(m.total),
        slot_bytes: (m.settings.slot_gb * GIB) as u64,
    };
    drop(m);
    let mut guard = state.processes.lock().unwrap();
    if guard
        .1
        .is_none_or(|t| t.elapsed() >= Duration::from_millis(2500))
    {
        guard.0.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_memory(),
        );
        guard.1 = Some(Instant::now());
    }
    let mut groups: HashMap<String, Group> = HashMap::new();
    for (pid, proc_) in guard.0.processes() {
        let name = proc_.name().to_string_lossy().into_owned();
        let key = name.to_lowercase();
        let bytes = proc_.memory();
        let uptime = proc_.run_time();
        let group = groups.entry(key).or_insert_with(|| Group {
            name: name.clone(),
            bytes: 0,
            count: 0,
            uptime_secs: 0,
            protected: protected(&name, pid.as_u32()),
            pids: Vec::new(),
        });
        group.bytes += bytes;
        group.count += 1;
        group.uptime_secs = group.uptime_secs.max(uptime);
        group.pids.push(PidRow {
            pid: pid.as_u32(),
            bytes,
            uptime_secs: uptime,
        });
    }
    snapshot.groups = groups.into_values().collect();
    snapshot.groups.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    snapshot.groups.truncate(12);
    for g in &mut snapshot.groups {
        g.pids.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    }
    snapshot
}

#[tauri::command]
fn kill_processes(pids: Vec<u32>) -> KillResult {
    let mut result = KillResult {
        killed: 0,
        failed: 0,
        errors: Vec::new(),
    };
    let mut system = System::new();
    for raw in pids.into_iter().take(100) {
        let pid = sysinfo::Pid::from_u32(raw);
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing(),
        );
        match system.process(pid) {
            None => {
                result.failed += 1;
                result.errors.push(format!("PID {raw}: nie znaleziono"));
            }
            Some(p) if protected(&p.name().to_string_lossy(), raw) => {
                result.failed += 1;
                result.errors.push(format!("PID {raw}: proces systemowy"));
            }
            Some(p) if p.kill() => result.killed += 1,
            Some(_) => {
                result.failed += 1;
                result.errors.push(format!(
                    "PID {raw}: brak uprawnień lub proces już zakończony"
                ));
            }
        }
    }
    result
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(w) = app.get_webview_window("popup") {
                let _ = w.set_focus();
            } else {
                open_popup(app);
            }
        }))
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .manage(AppState {
            memory: Arc::new(Mutex::new(MemoryState::new())),
            processes: Mutex::new((System::new(), None)),
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            kill_processes,
            get_settings,
            set_settings,
            close_popup
        ])
        .on_window_event(|w, e| {
            if w.label() == "popup" {
                if let tauri::WindowEvent::Focused(false) = e {
                    if let Some(p) = w.app_handle().get_webview_window("popup") {
                        destroy_popup(&p);
                    }
                }
            }
        })
        .setup(|app| {
            let first = !settings_path().exists();
            if first {
                let path = settings_path();
                let _ = fs::create_dir_all(path.parent().unwrap());
                let _ = fs::write(
                    path,
                    serde_json::to_vec_pretty(&Settings::default()).unwrap(),
                );
                let _ = app.autolaunch().enable();
            }
            let auto = CheckMenuItem::with_id(
                app,
                "autostart",
                "Uruchamiaj z Windows",
                true,
                app.autolaunch().is_enabled().unwrap_or(false),
                None::<&str>,
            )?;
            let show = MenuItem::with_id(app, "show", "Pokaż", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Zamknij", true, None::<&str>)?;
            let menu = MenuBuilder::new(app)
                .item(&show)
                .item(&auto)
                .item(&quit)
                .build()?;
            let tray = TrayIconBuilder::new()
                .icon(icon(Level::Green))
                .tooltip("RamLight")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_popup(tray.app_handle());
                    }
                })
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "show" => open_popup(app),
                    "quit" => {
                        QUIT_REQUESTED.store(true, Ordering::SeqCst);
                        app.exit(0);
                    }
                    "autostart" => {
                        let enabled = app.autolaunch().is_enabled().unwrap_or(false);
                        let outcome = if enabled {
                            app.autolaunch().disable()
                        } else {
                            app.autolaunch().enable()
                        };
                        if outcome.is_ok() {
                            let _ = auto.set_checked(!enabled);
                        }
                    }
                    _ => {}
                })
                .build(app)?;
            let state = app.state::<AppState>();
            let memory = state.memory.clone();
            thread::spawn(move || {
                let mut sys = System::new();
                let mut old = Level::Green;
                loop {
                    sys.refresh_memory();
                    let total = sys.total_memory();
                    let available = sys.available_memory();
                    let commit = commit_percent();
                    let (level, used, free) = {
                        let mut m = memory.lock().unwrap();
                        m.sample(total, available, commit);
                        (
                            m.level,
                            if m.total > 0 {
                                100 * (m.total - m.available) / m.total
                            } else {
                                0
                            },
                            m.available,
                        )
                    };
                    if level != old {
                        let _ = tray.set_icon(Some(icon(level)));
                        old = level;
                    }
                    let tooltip = format!(
                        "RAM {used}% · wolne {:.1} GB · {}",
                        free as f64 / GIB,
                        level.label()
                    );
                    let _ = tray.set_tooltip(Some(&tooltip));
                    thread::sleep(Duration::from_secs(2));
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Nie można zbudować RamLight")
        .run(|_app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if !QUIT_REQUESTED.load(Ordering::SeqCst) {
                    api.prevent_exit();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn boundaries_16_gib() {
        let t = 16 * GIB as u64;
        let r = reserve(t);
        let s = 2 * GIB as u64;
        assert_eq!(r, (1.28 * GIB) as u64);
        assert!(matches!(level_for(r + 2 * s, t, s, 0.0), Level::Green));
        assert!(matches!(level_for(r + 2 * s - 1, t, s, 0.0), Level::Yellow));
        assert!(matches!(level_for(r + s, t, s, 0.0), Level::Yellow));
        assert!(matches!(level_for(r + s - 1, t, s, 0.0), Level::Orange));
        assert!(matches!(level_for(r, t, s, 0.0), Level::Orange));
        assert!(matches!(level_for(r - 1, t, s, 0.0), Level::Red));
    }
    #[test]
    fn boundaries_64_gib_and_commit() {
        let t = 64 * GIB as u64;
        let r = reserve(t);
        let s = 2 * GIB as u64;
        assert!(matches!(level_for(r + 2 * s, t, s, 0.0), Level::Green));
        assert!(matches!(level_for(r + s, t, s, 0.0), Level::Yellow));
        assert!(matches!(level_for(r, t, s, 0.0), Level::Orange));
        assert!(matches!(level_for(r - 1, t, s, 0.0), Level::Red));
        assert!(matches!(level_for(t, t, s, 90.0), Level::Orange));
        assert!(matches!(level_for(t, t, s, 97.0), Level::Red));
    }
    #[test]
    fn deny_list() {
        assert!(protected("SvChOsT.ExE", 123));
        assert!(protected("anything.exe", std::process::id()));
        assert!(protected("RamLight.exe", 123));
        assert!(!protected("notepad.exe", 123));
    }
}
