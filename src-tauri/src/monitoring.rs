use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

const PRESENTMON_RELEASE_API: &str =
    "https://api.github.com/repos/GameTechDev/PresentMon/releases/latest";

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorPreferences {
    pub fps: bool,
    pub one_percent_low: bool,
    pub point_one_percent_low: bool,
    pub gpu_usage: bool,
    pub cpu_usage: bool,
    pub overlay_after_launch: bool,
}

impl Default for MonitorPreferences {
    fn default() -> Self {
        Self {
            fps: true,
            one_percent_low: true,
            point_one_percent_low: true,
            gpu_usage: true,
            cpu_usage: true,
            overlay_after_launch: false,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorSnapshot {
    pub game_name: String,
    pub game_running: bool,
    pub overlay_visible: bool,
    pub fps: Option<f64>,
    pub one_percent_low: Option<f64>,
    pub point_one_percent_low: Option<f64>,
    pub gpu_usage: Option<f64>,
    pub cpu_usage: Option<f64>,
    pub preferences: MonitorPreferences,
    pub message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameLaunchResult {
    pub message: String,
    pub process_id: u32,
    pub executable: String,
}

#[derive(Default)]
pub struct MonitorState {
    inner: Mutex<MonitorInner>,
}

#[derive(Default)]
struct MonitorInner {
    process_id: Option<u32>,
    game_name: String,
    frames: VecDeque<FrameSample>,
    gpu_usage: Option<f64>,
    cpu_usage: Option<f64>,
    message: String,
    preferences: MonitorPreferences,
    overlay_visible: bool,
    presentmon: Option<Child>,
}

#[derive(Clone, Copy)]
struct FrameSample {
    at: Instant,
    frame_ms: f64,
    present_ms: f64,
    gpu_busy_ms: Option<f64>,
}

pub fn record_game_launch(state: &MonitorState, process_id: u32, executable: &str) {
    with_inner(state, |inner| {
        inner.process_id = Some(process_id);
        inner.game_name = executable.to_owned();
        inner.frames.clear();
        inner.gpu_usage = None;
        inner.cpu_usage = None;
        inner.message = "Preparing frame and hardware monitoring…".into();
    });
}

#[derive(Deserialize)]
struct PresentMonRelease {
    tag_name: String,
    assets: Vec<PresentMonAsset>,
}

#[derive(Deserialize)]
struct PresentMonAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

fn with_inner<T>(state: &MonitorState, action: impl FnOnce(&mut MonitorInner) -> T) -> T {
    let mut inner = state
        .inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    action(&mut inner)
}

#[tauri::command]
pub fn get_monitor_snapshot(state: tauri::State<'_, MonitorState>) -> MonitorSnapshot {
    snapshot(&state)
}

fn snapshot(state: &MonitorState) -> MonitorSnapshot {
    with_inner(state, |inner| {
        let now = Instant::now();
        let recent_frames: Vec<f64> = inner
            .frames
            .iter()
            .filter(|frame| now.duration_since(frame.at) <= Duration::from_secs(1))
            .map(|frame| frame.frame_ms)
            .collect();
        let fps = if recent_frames.len() >= 2 {
            Some(recent_frames.len() as f64 * 1000.0 / recent_frames.iter().sum::<f64>())
        } else {
            None
        };
        let one_percent_low = percentile_low(&inner.frames, 0.01);
        // A tenth-percentile estimate needs at least 1,000 frames to avoid presenting
        // one slow frame as a stable statistic during short play sessions.
        let point_one_percent_low = if inner.frames.len() >= 1_000 {
            percentile_low(&inner.frames, 0.001)
        } else {
            None
        };
        MonitorSnapshot {
            game_name: inner.game_name.clone(),
            game_running: inner.process_id.is_some(),
            overlay_visible: inner.overlay_visible,
            fps,
            one_percent_low,
            point_one_percent_low,
            gpu_usage: inner.gpu_usage,
            cpu_usage: inner.cpu_usage,
            preferences: inner.preferences,
            message: inner.message.clone(),
        }
    })
}

fn percentile_low(frames: &VecDeque<FrameSample>, fraction: f64) -> Option<f64> {
    let count = frames.len();
    if count < 100 {
        return None;
    }
    let slow_count = ((count as f64 * fraction).ceil() as usize).max(1);
    let mut times: Vec<f64> = frames.iter().map(|frame| frame.frame_ms).collect();
    times.sort_by(f64::total_cmp);
    let slowest = &times[times.len().saturating_sub(slow_count)..];
    let average_ms = slowest.iter().sum::<f64>() / slowest.len() as f64;
    (average_ms > 0.0).then_some(1000.0 / average_ms)
}

#[tauri::command]
pub fn set_monitor_preferences(
    state: tauri::State<'_, MonitorState>,
    preferences: MonitorPreferences,
) -> Result<(), String> {
    with_inner(&state, |inner| inner.preferences = preferences);
    Ok(())
}

#[tauri::command]
pub async fn start_monitoring(app: AppHandle, process_id: u32) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || start_monitoring_blocking(&app, process_id))
        .await
        .map_err(|error| format!("Could not start game monitoring: {error}"))?
}

fn start_monitoring_blocking(app: &AppHandle, process_id: u32) -> Result<String, String> {
    if !process_is_running(process_id) {
        let state = app.state::<MonitorState>();
        with_inner(&state, |inner| {
            if inner.process_id == Some(process_id) {
                inner.process_id = None;
                inner.message = "Game closed before monitoring could start".into();
            }
        });
        return Err("The game closed before monitoring could start.".into());
    }
    let state = app.state::<MonitorState>();
    with_inner(&state, |inner| {
        if let Some(mut child) = inner.presentmon.take() {
            let _ = child.kill();
        }
        inner.process_id = Some(process_id);
        inner.frames.clear();
        inner.gpu_usage = None;
        inner.cpu_usage = None;
        inner.message = "Preparing frame and hardware monitoring…".into();
    });

    let presentmon_path = match ensure_presentmon(app) {
        Ok(path) => path,
        Err(error) => {
            with_inner(&state, |inner| {
                if inner.process_id == Some(process_id) {
                    inner.message = format!("Monitoring unavailable: {error}");
                }
            });
            return Err(error);
        }
    };
    let mut command = Command::new(&presentmon_path);
    command
        .args([
            "--process_id",
            &process_id.to_string(),
            "--output_stdout",
            "--v2_metrics",
            "--no_console_stats",
            "--no_track_input",
            "--terminate_on_proc_exit",
        ])
        .arg("--session_name")
        .arg(format!("ANRMonitor-{process_id}"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let message = format!("Could not start PresentMon frame capture: {error}");
            with_inner(&state, |inner| inner.message = message.clone());
            return Err(message);
        }
    };
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "PresentMon did not open its telemetry stream.".to_owned())?;
    let game_name = with_inner(&state, |inner| {
        inner.presentmon = Some(child);
        inner.game_name.clone()
    });
    with_inner(&state, |inner| {
        inner.message = "Waiting for the first frame…".into()
    });
    let app_for_reader = app.clone();
    thread::Builder::new()
        .name(format!("anr-monitor-{process_id}"))
        .spawn(move || read_presentmon(app_for_reader, process_id, stdout))
        .map_err(|error| format!("Could not start the monitoring reader: {error}"))?;
    Ok(if game_name.is_empty() {
        "Monitoring started. FPS and frame lows are warming up.".into()
    } else {
        format!("Monitoring {game_name}. FPS and frame lows are warming up.")
    })
}

fn read_presentmon(app: AppHandle, process_id: u32, stdout: impl Read) {
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let mut present_index = None;
    let mut display_index = None;
    let mut gpu_index = None;
    let mut last_cpu_sample = Instant::now();
    let mut previous_cpu = system_cpu_ticks();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => {
                let state = app.state::<MonitorState>();
                with_inner(&state, |inner| {
                    if inner.process_id == Some(process_id) {
                        inner.message = format!("Frame capture stopped: {error}");
                    }
                });
                break;
            }
        }
        let clean_line = line
            .trim_start_matches('\u{feff}')
            .trim_end_matches(['\r', '\n']);
        let fields: Vec<&str> = clean_line.split(',').collect();
        if present_index.is_none() {
            present_index = fields
                .iter()
                .position(|field| field.trim_matches('"') == "MsBetweenPresents");
            display_index = fields
                .iter()
                .position(|field| field.trim_matches('"') == "MsBetweenDisplayChange");
            gpu_index = fields
                .iter()
                .position(|field| field.trim_matches('"') == "MsGPUBusy");
            continue;
        }
        let present_ms = present_index
            .and_then(|index| fields.get(index))
            .and_then(|value| value.trim_matches('"').parse::<f64>().ok());
        let Some(present_ms) =
            present_ms.filter(|value| value.is_finite() && *value > 0.0 && *value < 5_000.0)
        else {
            continue;
        };
        let display_ms = display_index
            .and_then(|index| fields.get(index))
            .and_then(|value| value.trim_matches('"').parse::<f64>().ok());
        let frame_ms = if display_index.is_some() {
            let Some(display_ms) =
                display_ms.filter(|value| value.is_finite() && *value > 0.0 && *value < 5_000.0)
            else {
                continue;
            };
            display_ms
        } else {
            present_ms
        };
        let gpu_busy_ms = gpu_index
            .and_then(|index| fields.get(index))
            .and_then(|value| value.trim_matches('"').parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value >= 0.0);
        let now = Instant::now();
        let state = app.state::<MonitorState>();
        with_inner(&state, |inner| {
            if inner.process_id != Some(process_id) {
                return;
            }
            inner.frames.push_back(FrameSample {
                at: now,
                frame_ms,
                present_ms,
                gpu_busy_ms,
            });
            while inner
                .frames
                .front()
                .is_some_and(|frame| now.duration_since(frame.at) > Duration::from_secs(60))
            {
                inner.frames.pop_front();
            }
            let one_second_ago = now - Duration::from_secs(1);
            let gpu_window: Vec<&FrameSample> = inner
                .frames
                .iter()
                .filter(|frame| frame.at >= one_second_ago)
                .collect();
            let gpu_busy = gpu_window
                .iter()
                .filter_map(|frame| frame.gpu_busy_ms)
                .sum::<f64>();
            let frame_time = gpu_window
                .iter()
                .filter(|frame| frame.gpu_busy_ms.is_some())
                .map(|frame| frame.present_ms)
                .sum::<f64>();
            if frame_time > 0.0 {
                inner.gpu_usage = Some((gpu_busy * 100.0 / frame_time).clamp(0.0, 100.0));
            }
            if now.duration_since(last_cpu_sample) >= Duration::from_secs(1) {
                let current_cpu = system_cpu_ticks();
                inner.cpu_usage = system_cpu_percent(previous_cpu, current_cpu);
                previous_cpu = current_cpu;
                last_cpu_sample = now;
            }
            inner.message = "Live · rolling 60-second frame lows".into();
        });
    }
    let state = app.state::<MonitorState>();
    let process_still_running = process_is_running(process_id);
    let capture_error = with_inner(&state, |inner| {
        if inner.process_id != Some(process_id) {
            return String::new();
        }
        let Some(mut child) = inner.presentmon.take() else {
            return String::new();
        };
        if child.try_wait().ok().flatten().is_none() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let mut output = Vec::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_end(&mut output);
        }
        String::from_utf8_lossy(&output).trim().to_owned()
    });
    let hide_overlay = with_inner(&state, |inner| {
        if inner.process_id == Some(process_id) {
            if process_still_running {
                inner.message = capture_error_message(&capture_error);
                false
            } else {
                inner.process_id = None;
                inner.message = "Game closed · monitoring paused".into();
                let hide_overlay = inner.overlay_visible;
                inner.overlay_visible = false;
                hide_overlay
            }
        } else {
            false
        }
    });
    if hide_overlay {
        if let Some(window) = app.get_webview_window("monitor-overlay") {
            let _ = window.hide();
        }
    }
}

fn capture_error_message(error: &str) -> String {
    let normalized = error.to_ascii_lowercase();
    if normalized.contains("access denied") || normalized.contains("privilege") {
        return "Windows blocked frame capture. Run ANR as administrator, or add your account to Performance Log Users.".into();
    }
    error
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| format!("Frame capture stopped: {}", line.trim()))
        .unwrap_or_else(|| {
            "Frame capture stopped unexpectedly. Try launching the game from Setup again.".into()
        })
}

fn ensure_presentmon(app: &AppHandle) -> Result<PathBuf, String> {
    let path = app
        .path()
        .document_dir()
        .map_err(|error| format!("Could not locate Documents for the monitoring tool: {error}"))?
        .join("ANR Native Act")
        .join("tools")
        .join("PresentMon.exe");
    if path.is_file() {
        return Ok(path);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not prepare the monitoring tool folder: {error}"))?;
    }
    let client = Client::builder()
        .user_agent("ANR-Native-Activation/monitoring")
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|error| format!("Could not prepare the GitHub connection: {error}"))?;
    let release = client
        .get(PRESENTMON_RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("Could not find PresentMon on GitHub: {error}"))?
        .json::<PresentMonRelease>()
        .map_err(|error| {
            format!("GitHub returned unreadable PresentMon release details: {error}")
        })?;
    if release
        .tag_name
        .chars()
        .any(|character| matches!(character, '/' | '\\' | ':'))
    {
        return Err("GitHub returned an invalid PresentMon release tag.".into());
    }
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name.starts_with("PresentMon-") && asset.name.ends_with("-x64.exe"))
        .ok_or_else(|| {
            format!(
                "The latest PresentMon release ({}) has no standalone x64 console app.",
                release.tag_name
            )
        })?;
    let temporary = path.with_extension("exe.part");
    let mut response = client
        .get(&asset.browser_download_url)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("Could not download the official PresentMon tool: {error}"))?;
    let mut output = File::create(&temporary)
        .map_err(|error| format!("Could not create the PresentMon cache: {error}"))?;
    response
        .copy_to(&mut output)
        .map_err(|error| format!("PresentMon download was interrupted: {error}"))?;
    output.flush().map_err(|error| error.to_string())?;
    drop(output);
    if let Some(expected) = asset
        .digest
        .as_deref()
        .and_then(|value| value.strip_prefix("sha256:"))
    {
        let mut file = File::open(&temporary).map_err(|error| error.to_string())?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        if !format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(expected) {
            let _ = fs::remove_file(&temporary);
            return Err("The PresentMon download did not match GitHub's SHA-256 digest.".into());
        }
    }
    let mut signature = [0_u8; 2];
    if File::open(&temporary)
        .and_then(|mut file| file.read_exact(&mut signature))
        .is_err()
        || signature != *b"MZ"
    {
        let _ = fs::remove_file(&temporary);
        return Err("The downloaded PresentMon file is not a valid Windows application.".into());
    }
    fs::rename(&temporary, &path)
        .map_err(|error| format!("Could not finish caching PresentMon: {error}"))?;
    Ok(path)
}

#[tauri::command]
pub fn stop_monitoring(state: tauri::State<'_, MonitorState>) -> Result<(), String> {
    with_inner(&state, |inner| {
        if let Some(mut child) = inner.presentmon.take() {
            let _ = child.kill();
        }
        inner.process_id = None;
        inner.frames.clear();
        inner.gpu_usage = None;
        inner.cpu_usage = None;
        inner.message = "Monitoring stopped".into();
    });
    Ok(())
}

#[tauri::command]
pub fn set_overlay_visible(app: AppHandle, visible: bool) -> Result<(), String> {
    if visible {
        let window = if let Some(window) = app.get_webview_window("monitor-overlay") {
            window
        } else {
            WebviewWindowBuilder::new(
                &app,
                "monitor-overlay",
                WebviewUrl::App("index.html?overlay=1".into()),
            )
            .title("ANR performance overlay")
            .inner_size(270.0, 134.0)
            .position(18.0, 18.0)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .focusable(false)
            .resizable(false)
            .visible(false)
            .build()
            .map_err(|error| format!("Could not create the in-game overlay: {error}"))?
        };
        enable_click_through(&window)?;
        window
            .show()
            .map_err(|error| format!("Could not show the in-game overlay: {error}"))?;
        with_inner(&app.state::<MonitorState>(), |inner| {
            inner.overlay_visible = true
        });
    } else {
        if let Some(window) = app.get_webview_window("monitor-overlay") {
            window
                .hide()
                .map_err(|error| format!("Could not hide the in-game overlay: {error}"))?;
        }
        with_inner(&app.state::<MonitorState>(), |inner| {
            inner.overlay_visible = false
        });
    }
    Ok(())
}

#[cfg(windows)]
fn enable_click_through(window: &WebviewWindow) -> Result<(), String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
        WS_EX_TRANSPARENT,
    };
    let hwnd = window
        .hwnd()
        .map_err(|error| format!("Could not access the overlay window: {error}"))?;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            style
                | WS_EX_LAYERED.0 as isize
                | WS_EX_TRANSPARENT.0 as isize
                | WS_EX_TOOLWINDOW.0 as isize,
        );
    }
    Ok(())
}

#[cfg(not(windows))]
fn enable_click_through(_window: &WebviewWindow) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
fn process_is_running(process_id: u32) -> bool {
    use windows::Win32::{
        Foundation::{CloseHandle, STILL_ACTIVE},
        System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) else {
            return false;
        };
        let mut exit_code = 0;
        let running = GetExitCodeProcess(handle, &mut exit_code).is_ok()
            && exit_code == STILL_ACTIVE.0 as u32;
        let _ = CloseHandle(handle);
        running
    }
}

#[cfg(not(windows))]
fn process_is_running(_process_id: u32) -> bool {
    true
}

#[cfg(windows)]
fn system_cpu_ticks() -> Option<(u64, u64, u64)> {
    use windows::Win32::{Foundation::FILETIME, System::Threading::GetSystemTimes};
    fn ticks(value: FILETIME) -> u64 {
        (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
    }
    let (mut idle, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe {
        GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).ok()?;
    }
    Some((ticks(idle), ticks(kernel), ticks(user)))
}

#[cfg(not(windows))]
fn system_cpu_ticks() -> Option<(u64, u64, u64)> {
    None
}

fn system_cpu_percent(
    previous: Option<(u64, u64, u64)>,
    current: Option<(u64, u64, u64)>,
) -> Option<f64> {
    let (previous_idle, previous_kernel, previous_user) = previous?;
    let (idle, kernel, user) = current?;
    let idle_delta = idle.saturating_sub(previous_idle);
    let total_delta = kernel.saturating_sub(previous_kernel) + user.saturating_sub(previous_user);
    if total_delta == 0 {
        return None;
    }
    let busy_delta = total_delta.saturating_sub(idle_delta);
    Some((busy_delta as f64 * 100.0 / total_delta as f64).clamp(0.0, 100.0))
}
