mod cleanup;
#[cfg(target_os = "windows")]
mod insertion;
mod nemo;
mod normalize;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::{Deserialize, Serialize};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Replacement {
    from: String,
    to: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    microphone: Option<String>,
    shortcut: String,
    activation_mode: String,
    runtime_path: String,
    model_path: String,
    device: String,
    replacements: Vec<Replacement>,
    unload_after_seconds: u64,
    #[serde(default)]
    cleanup_mode: cleanup::Mode,
    #[serde(default)]
    cleanup_runtime_path: String,
    #[serde(default)]
    cleanup_model_path: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppSnapshot {
    config: Config,
    phase: String,
    error: Option<String>,
    last_transcript: Option<String>,
    live_transcript: Option<String>,
    model_loaded: bool,
}

struct Backend {
    snapshot: AppSnapshot,
    audio: mpsc::Sender<AudioCommand>,
    inference: mpsc::Sender<InferenceCommand>,
    foreground: isize,
    shortcut_capture: bool,
    start_cancelled: bool,
}

struct AppState(Mutex<Backend>);
struct IndicatorState(Mutex<bool>);
type CapturedAudio = Result<(Vec<f32>, u32), String>;
const DEFAULT_UNLOAD_SECONDS: u64 = 300;

enum AudioCommand {
    Start(Option<String>, mpsc::Sender<Result<(), String>>),
    Stop(mpsc::Sender<CapturedAudio>),
    Failed(String),
}

enum InferenceCommand {
    Transcribe(Config, Vec<f32>, u32, isize),
    Unload,
    UnloadCleanup,
    StartLive(Config, isize),
    LiveAudio(Vec<f32>, u32),
}

struct Live {
    stream: nemo::Stream,
    replacements: Vec<Replacement>,
    target: isize,
    text: String,
    visible: String,
    inserted: String,
    failure: Option<String>,
}

fn default_config() -> Config {
    Config {
        microphone: None,
        shortcut: "Ctrl+Shift+Space".into(),
        activation_mode: "hold".into(),
        runtime_path: String::new(),
        model_path: String::new(),
        device: "vulkan:0".into(),
        replacements: vec![],
        unload_after_seconds: DEFAULT_UNLOAD_SECONDS,
        cleanup_mode: cleanup::Mode::Raw,
        cleanup_runtime_path: String::new(),
        cleanup_model_path: String::new(),
    }
}

fn cleanup_enabled(config: &Config) -> bool {
    config.cleanup_mode != cleanup::Mode::Raw
        && !config.cleanup_runtime_path.trim().is_empty()
        && !config.cleanup_model_path.trim().is_empty()
}

fn apply_cleanup(
    cleaner: &mut Option<cleanup::Cleaner>,
    config: &Config,
    text: String,
) -> (String, Option<String>) {
    if !cleanup_enabled(config) || text.trim().is_empty() {
        return (text, None);
    }
    let stale = cleaner.as_ref().is_some_and(|active| {
        !active.matches(&config.cleanup_runtime_path, &config.cleanup_model_path)
    });
    if stale {
        *cleaner = None;
    }
    if cleaner.is_none() {
        match cleanup::Cleaner::load(&config.cleanup_runtime_path, &config.cleanup_model_path) {
            Ok(loaded) => *cleaner = Some(loaded),
            Err(error) => return (text, Some(format!("Cleanup is unavailable: {error}"))),
        }
    }
    match cleaner
        .as_mut()
        .unwrap()
        .clean(text.trim(), config.cleanup_mode)
    {
        Ok(cleaned) => (cleaned, None),
        Err(error) => (
            text,
            Some(format!("Cleanup kept the original transcript: {error}")),
        ),
    }
}

fn publish(app: &AppHandle) -> AppSnapshot {
    let snapshot = app.state::<AppState>().0.lock().unwrap().snapshot.clone();
    let _ = app.emit("dictation-state", &snapshot);
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        update_indicator(&handle);
    });
    snapshot
}

fn build_indicator(app: &AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    let window = tauri::WebviewWindowBuilder::new(
        app,
        "indicator",
        tauri::WebviewUrl::App("index.html?indicator=1".into()),
    )
    .title("Voxen dictation")
    .inner_size(168.0, 68.0)
    .transparent(true)
    .decorations(false)
    .resizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focused(false)
    .focusable(false)
    .visible(false)
    .build()?;
    let _ = window.set_ignore_cursor_events(true);
    Ok(window)
}

fn position_indicator(window: &tauri::WebviewWindow) {
    if let Ok(Some(monitor)) = window.primary_monitor() {
        let scale = monitor.scale_factor();
        let size = monitor.size();
        let origin = monitor.position();
        let _ = window.set_position(tauri::PhysicalPosition::new(
            origin.x + (size.width as i32 - (168.0 * scale) as i32) / 2,
            origin.y + size.height as i32 - (108.0 * scale) as i32,
        ));
    }
}

fn indicator_active(phase: &str, starting_cancelled: bool) -> bool {
    match phase {
        "starting" => !starting_cancelled,
        "recording" | "transcribing" | "inserting" => true,
        _ => false,
    }
}

fn update_indicator(app: &AppHandle) {
    let indicator = app.state::<IndicatorState>();
    let mut visible = indicator.0.lock().unwrap();
    let (phase, starting_cancelled) = {
        let state = app.state::<AppState>();
        let backend = state.0.lock().unwrap();
        (backend.snapshot.phase.clone(), backend.start_cancelled)
    };
    if indicator_active(&phase, starting_cancelled) {
        let window = match app.get_webview_window("indicator") {
            Some(window) => window,
            None => match build_indicator(app) {
                Ok(window) => {
                    *visible = false;
                    window
                }
                Err(_) => return,
            },
        };
        if !*visible {
            position_indicator(&window);
            if window.show().is_ok() {
                *visible = true;
            }
        }
    } else if *visible {
        if let Some(window) = app.get_webview_window("indicator") {
            let _ = window.hide();
        }
        *visible = false;
    }
}

fn report_error(app: &AppHandle, error: String) {
    {
        let state = app.state::<AppState>();
        let mut backend = state.0.lock().unwrap();
        backend.snapshot.phase = "error".into();
        backend.snapshot.error = Some(error);
    }
    publish(app);
}

fn audio_worker(
    app: AppHandle,
    receiver: mpsc::Receiver<AudioCommand>,
    sender: mpsc::Sender<AudioCommand>,
    inference: mpsc::Sender<InferenceCommand>,
) {
    let mut stream: Option<cpal::Stream> = None;
    let mut samples = Arc::new(Mutex::new(Vec::<f32>::new()));
    let mut rate = 0;
    let mut forwarded = 0;
    loop {
        let command = if stream.is_some() {
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(command) => Some(command),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match receiver.recv() {
                Ok(command) => Some(command),
                Err(_) => break,
            }
        };
        match command {
            Some(AudioCommand::Start(name, reply)) => {
                let result = (|| {
                    let host = cpal::default_host();
                    let device = match name.as_deref().filter(|name| !name.is_empty()) {
                        Some(name) => host
                            .input_devices()
                            .map_err(|e| e.to_string())?
                            .find(|device| device.name().ok().as_deref() == Some(name))
                            .ok_or("Selected microphone is unavailable")?,
                        None => host
                            .default_input_device()
                            .ok_or("No microphone is available")?,
                    };
                    let supported = device.default_input_config().map_err(|e| e.to_string())?;
                    let config: cpal::StreamConfig = supported.clone().into();
                    let channels = config.channels as usize;
                    rate = config.sample_rate.0;
                    samples = Arc::new(Mutex::new(Vec::new()));
                    let target = samples.clone();
                    let errors = sender.clone();
                    let on_error = move |error: cpal::StreamError| {
                        let _ = errors.send(AudioCommand::Failed(format!(
                            "Microphone stream failed: {error}"
                        )));
                    };
                    let active = match supported.sample_format() {
                        cpal::SampleFormat::F32 => device.build_input_stream(
                            &config,
                            move |data: &[f32], _| {
                                append_mono(&target, data.iter().copied(), channels)
                            },
                            on_error,
                            None,
                        ),
                        cpal::SampleFormat::I16 => device.build_input_stream(
                            &config,
                            move |data: &[i16], _| {
                                append_mono(
                                    &target,
                                    data.iter().map(|value| *value as f32 / 32768.0),
                                    channels,
                                )
                            },
                            on_error,
                            None,
                        ),
                        cpal::SampleFormat::U16 => device.build_input_stream(
                            &config,
                            move |data: &[u16], _| {
                                append_mono(
                                    &target,
                                    data.iter().map(|value| (*value as f32 - 32768.0) / 32768.0),
                                    channels,
                                )
                            },
                            on_error,
                            None,
                        ),
                        format => return Err(format!("Unsupported microphone format: {format:?}")),
                    }
                    .map_err(|e| e.to_string())?;
                    active.play().map_err(|e| e.to_string())?;
                    Ok(active)
                })();
                match result {
                    Ok(active) => {
                        stream = Some(active);
                        forwarded = 0;
                        let _ = reply.send(Ok(()));
                    }
                    Err(error) => {
                        let _ = reply.send(Err(error));
                    }
                }
            }
            Some(AudioCommand::Stop(reply)) => {
                if stream.take().is_none() {
                    let _ = reply.send(Err("Recording is not active".into()));
                } else {
                    forward_audio(&inference, &samples, &mut forwarded, rate);
                    let captured = std::mem::take(&mut *samples.lock().unwrap());
                    forwarded = 0;
                    let _ = reply.send(Ok((captured, rate)));
                }
            }
            Some(AudioCommand::Failed(error)) => {
                if stream.take().is_some() {
                    samples.lock().unwrap().clear();
                    forwarded = 0;
                    report_error(&app, error);
                }
            }
            None => {
                if stream.is_some() {
                    forward_audio(&inference, &samples, &mut forwarded, rate);
                }
            }
        }
    }
}

fn forward_audio(
    inference: &mpsc::Sender<InferenceCommand>,
    samples: &Arc<Mutex<Vec<f32>>>,
    forwarded: &mut usize,
    rate: u32,
) {
    let chunk = {
        let captured = samples.lock().unwrap();
        if captured.len() <= *forwarded {
            return;
        }
        let chunk = captured[*forwarded..].to_vec();
        *forwarded = captured.len();
        chunk
    };
    let _ = inference.send(InferenceCommand::LiveAudio(chunk, rate));
}

fn append_mono(target: &Arc<Mutex<Vec<f32>>>, data: impl Iterator<Item = f32>, channels: usize) {
    let mut output = target.lock().unwrap();
    let mut sum = 0.0;
    let mut count = 0;
    for value in data {
        sum += value;
        count += 1;
        if count == channels {
            output.push(sum / channels as f32);
            sum = 0.0;
            count = 0;
        }
    }
}

fn inference_worker(app: AppHandle, receiver: mpsc::Receiver<InferenceCommand>) {
    let mut model: Option<nemo::Recognizer> = None;
    let mut cleaner: Option<cleanup::Cleaner> = None;
    let mut deadline: Option<Instant> = None;
    let mut unload_model_on_idle = false;
    let mut live: Option<Live> = None;
    loop {
        let command = match deadline {
            Some(until) => {
                match receiver.recv_timeout(until.saturating_duration_since(Instant::now())) {
                    Ok(command) => command,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if unload_model_on_idle {
                            InferenceCommand::Unload
                        } else {
                            InferenceCommand::UnloadCleanup
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            None => match receiver.recv() {
                Ok(command) => command,
                Err(_) => break,
            },
        };
        match command {
            InferenceCommand::Unload => {
                live = None;
                model = None;
                cleaner = None;
                deadline = None;
                app.state::<AppState>()
                    .0
                    .lock()
                    .unwrap()
                    .snapshot
                    .model_loaded = false;
                publish(&app);
            }
            InferenceCommand::UnloadCleanup => {
                cleaner = None;
                deadline = None;
            }
            InferenceCommand::StartLive(config, target) => {
                live = None;
                deadline = None;
                match start_live(&app, &mut model, &config, target) {
                    Ok(session) => live = Some(session),
                    Err(error) => {
                        app.state::<AppState>().0.lock().unwrap().snapshot.error =
                            Some(format!("Live typing is unavailable: {error}"));
                        publish(&app);
                    }
                }
            }
            InferenceCommand::LiveAudio(samples, rate) => {
                if let Some(session) = live.as_mut() {
                    if session.failure.is_none() {
                        match session.stream.push(&samples, rate) {
                            Ok(()) => pump_live(&app, session),
                            Err(error) => session.failure = Some(error),
                        }
                    }
                }
            }
            InferenceCommand::Transcribe(config, samples, rate, foreground) => {
                let mut cleanup_warning: Option<String> = None;
                let result = (|| {
                    if let Some(mut session) = live.take() {
                        if session.failure.is_none() {
                            match session.stream.finish() {
                                Ok(()) => pump_live(&app, &mut session),
                                Err(error) => session.failure = Some(error),
                            }
                        }
                        let error = session.failure;
                        let mut transcript = if error.is_some() {
                            session.visible
                        } else {
                            session.text
                        };
                        let mut error = error;
                        if error.is_none() {
                            let (cleaned, warning) = apply_cleanup(&mut cleaner, &config, transcript.clone());
                            cleanup_warning = warning;
                            let cleaned = normalize::normalize(&cleaned);
                            if cleaned != transcript && session.target != 0 {
                                app.state::<AppState>().0.lock().unwrap().snapshot.phase =
                                    "inserting".into();
                                publish(&app);
                                if let Err(failure) = insertion::replace_text(session.target, &session.inserted, &cleaned) {
                                    error = Some(failure);
                                }
                            }
                            transcript = cleaned;
                        }
                        let state = app.state::<AppState>();
                        let mut backend = state.0.lock().unwrap();
                        backend.snapshot.last_transcript = Some(transcript);
                        backend.snapshot.live_transcript = None;
                        drop(backend);
                        publish(&app);
                        return match error {
                            Some(error) => Err(error),
                            None => Ok(()),
                        };
                    }
                    if model.is_none() {
                        model = Some(nemo::Recognizer::load(
                            &config.runtime_path,
                            &config.model_path,
                            &config.device,
                        )?);
                        app.state::<AppState>()
                            .0
                            .lock()
                            .unwrap()
                            .snapshot
                            .model_loaded = true;
                        publish(&app);
                    }
                    let text = model.as_mut().unwrap().transcribe(&samples, rate)?;
                    let text = apply_replacements(text, &config.replacements);
                    let (text, warning) = apply_cleanup(&mut cleaner, &config, text);
                    cleanup_warning = warning;
                    let text = normalize::normalize(&text);
                    app.state::<AppState>()
                        .0
                        .lock()
                        .unwrap()
                        .snapshot
                        .last_transcript = Some(text.clone());
                    if !text.trim().is_empty() && foreground != 0 {
                        app.state::<AppState>().0.lock().unwrap().snapshot.phase =
                            "inserting".into();
                        publish(&app);
                        #[cfg(target_os = "windows")]
                        insertion::insert_text(foreground, &text)?;
                    }
                    Ok::<(), String>(())
                })();
                unload_model_on_idle = config.unload_after_seconds > 0;
                deadline = if model.is_some() && (unload_model_on_idle || cleaner.is_some()) {
                    let idle = if unload_model_on_idle {
                        config.unload_after_seconds
                    } else {
                        DEFAULT_UNLOAD_SECONDS
                    };
                    Instant::now().checked_add(Duration::from_secs(idle))
                } else {
                    None
                };
                match result {
                    Ok(()) => {
                        let state = app.state::<AppState>();
                        let mut backend = state.0.lock().unwrap();
                        backend.snapshot.phase = "idle".into();
                        backend.snapshot.error = cleanup_warning;
                        drop(backend);
                        publish(&app);
                    }
                    Err(error) => report_error(&app, error),
                }
            }
        }
    }
}

fn start_live(
    app: &AppHandle,
    model: &mut Option<nemo::Recognizer>,
    config: &Config,
    target: isize,
) -> Result<Live, String> {
    if model.is_none() {
        *model = Some(nemo::Recognizer::load(
            &config.runtime_path,
            &config.model_path,
            &config.device,
        )?);
        app.state::<AppState>()
            .0
            .lock()
            .unwrap()
            .snapshot
            .model_loaded = true;
        publish(app);
    }
    Ok(Live {
        stream: model.as_ref().unwrap().stream()?,
        replacements: config.replacements.clone(),
        target,
        text: String::new(),
        visible: String::new(),
        inserted: String::new(),
        failure: None,
    })
}

fn pump_live(app: &AppHandle, session: &mut Live) {
    loop {
        match session.stream.next() {
            Ok(Some((transcript, is_final))) => {
                apply_live(app, session, transcript, is_final);
                if session.failure.is_some() {
                    break;
                }
            }
            Ok(None) => break,
            Err(error) => {
                session.failure = Some(error);
                break;
            }
        }
    }
}

fn apply_live(app: &AppHandle, session: &mut Live, transcript: String, is_final: bool) {
    let transcript = apply_replacements(transcript, &session.replacements);
    if is_final {
        if !session.text.is_empty() && !transcript.is_empty() {
            session.text.push(' ');
        }
        session.text.push_str(&transcript);
    }
    let mut visible = session.text.clone();
    if !is_final && !transcript.is_empty() {
        if !visible.is_empty() {
            visible.push(' ');
        }
        visible.push_str(&transcript);
    }
    if session.target != 0 {
        match insertion::replace_text(session.target, &session.inserted, &visible) {
            Ok(()) => session.inserted = visible.clone(),
            Err(error) => session.failure = Some(error),
        }
    }
    session.visible = visible;
    app.state::<AppState>().0.lock().unwrap().snapshot.live_transcript =
        (!session.visible.is_empty()).then_some(session.visible.clone());
    publish(app);
}

fn apply_replacements(text: String, replacements: &[Replacement]) -> String {
    replacements
        .iter()
        .filter(|replacement| !replacement.from.is_empty())
        .fold(text, |text, replacement| {
            text.replace(&replacement.from, &replacement.to)
        })
}

fn is_available(phase: &str) -> bool {
    matches!(phase, "idle" | "error")
}

#[derive(Debug, PartialEq, Eq)]
enum StartCompletion {
    Listening,
    Idle,
    Error,
}

fn apply_start_completion(backend: &mut Backend, result: Result<(), String>) -> StartCompletion {
    let cancelled = std::mem::take(&mut backend.start_cancelled);
    backend.snapshot.live_transcript = None;
    if cancelled {
        backend.snapshot.phase = "idle".into();
        backend.snapshot.error = None;
        return StartCompletion::Idle;
    }
    match result {
        Ok(()) => {
            backend.snapshot.phase = "recording".into();
            backend.snapshot.error = None;
            StartCompletion::Listening
        }
        Err(error) => {
            backend.snapshot.phase = "error".into();
            backend.snapshot.error = Some(error);
            StartCompletion::Error
        }
    }
}

fn complete_start(
    app: AppHandle,
    config: Config,
    foreground: isize,
    receiver: mpsc::Receiver<Result<(), String>>,
) {
    let result = receiver
        .recv()
        .map_err(|error| error.to_string())
        .and_then(|result| result);
    let started = result.is_ok();
    {
        let state = app.state::<AppState>();
        let mut backend = state.0.lock().unwrap();
        if started && backend.start_cancelled {
            let (tx, _rx) = mpsc::channel();
            let _ = backend.audio.send(AudioCommand::Stop(tx));
        }
        let completion = apply_start_completion(&mut backend, result);
        if completion == StartCompletion::Listening {
            let _ = backend
                .inference
                .send(InferenceCommand::StartLive(config, foreground));
        }
    }
    publish(&app);
}

fn validate_config(config: &Config) -> Result<Shortcut, String> {
    if !matches!(config.activation_mode.as_str(), "hold" | "toggle") {
        return Err("Choose hold or toggle activation".into());
    }
    if config.device != "cpu"
        && config
            .device
            .strip_prefix("vulkan:")
            .and_then(|value| value.parse::<u32>().ok())
            .is_none()
    {
        return Err("Device must be vulkan:<index> or cpu".into());
    }
    if config.unload_after_seconds > 86_400 {
        return Err(
            "Choose an unload delay up to 86400 seconds, or 0 to keep the model loaded".into(),
        );
    }
    if config
        .replacements
        .iter()
        .any(|replacement| replacement.from.is_empty())
    {
        return Err("Replacement source phrases cannot be empty".into());
    }
    if config.cleanup_mode != cleanup::Mode::Raw
        && (config.cleanup_runtime_path.trim().is_empty()
            || config.cleanup_model_path.trim().is_empty())
    {
        return Err("Set the cleanup runtime and model paths, or choose the Raw mode".into());
    }
    config
        .shortcut
        .parse::<Shortcut>()
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_snapshot(state: State<'_, AppState>) -> AppSnapshot {
    state.0.lock().unwrap().snapshot.clone()
}

#[tauri::command]
fn list_microphones() -> Result<Vec<String>, String> {
    let mut names = cpal::default_host()
        .input_devices()
        .map_err(|e| e.to_string())?
        .map(|device| device.name().map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    names.dedup();
    Ok(names)
}

#[tauri::command]
fn set_shortcut_capture(app: AppHandle, state: State<'_, AppState>, active: bool) -> Result<(), String> {
    let mut backend = state.0.lock().unwrap();
    if backend.shortcut_capture == active { return Ok(()); }
    if active && !is_available(&backend.snapshot.phase) {
        return Err("Wait until dictation finishes before changing the shortcut".into());
    }
    let shortcut = backend.snapshot.config.shortcut.parse::<Shortcut>().map_err(|error| error.to_string())?;
    if active {
        if app.global_shortcut().is_registered(shortcut) {
            app.global_shortcut().unregister(shortcut).map_err(|error| error.to_string())?;
        }
    } else if !app.global_shortcut().is_registered(shortcut) {
        app.global_shortcut().register(shortcut).map_err(|error| error.to_string())?;
    }
    backend.shortcut_capture = active;
    Ok(())
}

#[tauri::command]
fn save_config(
    app: AppHandle,
    state: State<'_, AppState>,
    config: Config,
) -> Result<AppSnapshot, String> {
    let shortcut = validate_config(&config)?;
    let mut backend = state.0.lock().unwrap();
    if !is_available(&backend.snapshot.phase) {
        return Err("Wait until dictation finishes before changing settings".into());
    }
    if backend.shortcut_capture {
        return Err("Finish selecting the shortcut before saving".into());
    }
    let old_shortcut = backend
        .snapshot
        .config
        .shortcut
        .parse::<Shortcut>()
        .map_err(|e| e.to_string())?;
    let changed = shortcut != old_shortcut;
    let old_registered = app.global_shortcut().is_registered(old_shortcut);
    let register_new = changed || !old_registered;
    let directory = app.path().app_config_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let pending = directory.join("config.new");
    std::fs::write(
        &pending,
        serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if register_new {
        app.global_shortcut()
            .register(shortcut)
            .map_err(|e| e.to_string())?;
    }
    if changed && old_registered {
        if let Err(error) = app.global_shortcut().unregister(old_shortcut) {
            let _ = app.global_shortcut().unregister(shortcut);
            return Err(error.to_string());
        }
    }
    if let Err(error) = std::fs::rename(&pending, directory.join("config.json")) {
        if changed && old_registered {
            let _ = app.global_shortcut().register(old_shortcut);
        }
        if register_new {
            let _ = app.global_shortcut().unregister(shortcut);
        }
        return Err(error.to_string());
    }
    backend
        .inference
        .send(InferenceCommand::Unload)
        .map_err(|e| e.to_string())?;
    backend.snapshot.config = config;
    backend.snapshot.phase = "idle".into();
    backend.snapshot.error = None;
    drop(backend);
    Ok(publish(&app))
}

#[tauri::command]
fn start_recording(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let (config, foreground, receiver) = {
        let mut backend = state.0.lock().unwrap();
        if !is_available(&backend.snapshot.phase) {
            return Err("Dictation is already in progress".into());
        }
        if backend.snapshot.config.runtime_path.trim().is_empty()
            || backend.snapshot.config.model_path.trim().is_empty()
        {
            return Err("Set and save the local runtime and model paths first".into());
        }
        #[cfg(target_os = "windows")]
        {
            backend.foreground =
                unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow().0 as isize };
            if app
                .get_webview_window("main")
                .and_then(|window| window.hwnd().ok())
                .is_some_and(|hwnd| hwnd.0 as isize == backend.foreground)
            {
                backend.foreground = 0;
            }
        }
        let (tx, rx) = mpsc::channel();
        backend
            .audio
            .send(AudioCommand::Start(
                backend.snapshot.config.microphone.clone(),
                tx,
            ))
            .map_err(|e| e.to_string())?;
        backend.start_cancelled = false;
        backend.snapshot.phase = "starting".into();
        backend.snapshot.error = None;
        backend.snapshot.live_transcript = None;
        (
            backend.snapshot.config.clone(),
            backend.foreground,
            rx,
        )
    };
    publish(&app);
    std::thread::spawn(move || complete_start(app, config, foreground, receiver));
    Ok(())
}

#[tauri::command]
fn stop_recording(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let mut backend = state.0.lock().unwrap();
    if backend.snapshot.phase == "starting" {
        backend.start_cancelled = true;
        drop(backend);
        publish(&app);
        return Ok(());
    }
    if backend.snapshot.phase != "recording" {
        return Err("Recording is not active".into());
    }
    let (tx, rx) = mpsc::channel();
    backend
        .audio
        .send(AudioCommand::Stop(tx))
        .map_err(|e| e.to_string())?;
    let (samples, rate) = rx.recv().map_err(|e| e.to_string())??;
    backend.snapshot.phase = "transcribing".into();
    let job = InferenceCommand::Transcribe(
        backend.snapshot.config.clone(),
        samples,
        rate,
        backend.foreground,
    );
    drop(backend);
    publish(&app);
    state
        .0
        .lock()
        .unwrap()
        .inference
        .send(job)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn unload_model(state: State<'_, AppState>) -> Result<(), String> {
    let backend = state.0.lock().unwrap();
    if !is_available(&backend.snapshot.phase) {
        return Err("Wait until dictation finishes before unloading the model".into());
    }
    backend
        .inference
        .send(InferenceCommand::Unload)
        .map_err(|e| e.to_string())
}

fn open_settings(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    } else {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Ok(builder) =
                tauri::WebviewWindowBuilder::from_config(&app, &app.config().app.windows[0])
            {
                let _ = builder.build();
            }
        });
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let snapshot = AppSnapshot {
        config: default_config(),
        phase: "idle".into(),
        error: None,
        last_transcript: None,
        live_transcript: None,
        model_loaded: false,
    };
    let (audio_tx, audio_rx) = mpsc::channel();
    let (inference_tx, inference_rx) = mpsc::channel();
    let audio_inference = inference_tx.clone();
    tauri::Builder::default()
        .manage(IndicatorState(Mutex::new(false)))
        .plugin(tauri_plugin_global_shortcut::Builder::new().with_handler(|app, _, event| {
            let state = app.state::<AppState>();
            if state.0.lock().unwrap().shortcut_capture { return; }
            let (mode, phase) = { let backend = state.0.lock().unwrap(); (backend.snapshot.config.activation_mode.clone(), backend.snapshot.phase.clone()) };
            let result = match (mode.as_str(), event.state) {
                ("hold", ShortcutState::Pressed) if is_available(&phase) => start_recording(app.clone(), state),
                ("hold", ShortcutState::Released) if matches!(phase.as_str(), "starting" | "recording") => stop_recording(app.clone(), state),
                ("toggle", ShortcutState::Pressed) if matches!(phase.as_str(), "starting" | "recording") => stop_recording(app.clone(), state),
                ("toggle", ShortcutState::Pressed) if is_available(&phase) => start_recording(app.clone(), state),
                _ => Ok(()),
            };
            if let Err(error) = result { report_error(app, error); }
        }).build())
        .manage(AppState(Mutex::new(Backend { snapshot, audio: audio_tx.clone(), inference: inference_tx, foreground: 0, shortcut_capture: false, start_cancelled: false })))
        .setup(move |app| {
            let handle = app.handle().clone();
            std::thread::spawn(move || audio_worker(handle, audio_rx, audio_tx, audio_inference));
            let handle = app.handle().clone();
            std::thread::spawn(move || inference_worker(handle, inference_rx));
            let _ = build_indicator(app.handle());
            if let Ok(directory) = app.path().app_config_dir() {
                if let Ok(contents) = std::fs::read(directory.join("config.json")) {
                    match serde_json::from_slice::<Config>(&contents) {
                        Ok(config) => {
                            if validate_config(&config).is_ok() { app.state::<AppState>().0.lock().unwrap().snapshot.config = config; }
                            else { report_error(app.handle(), "Saved settings are invalid. Review and save settings again.".into()); }
                        }
                        Err(error) => report_error(app.handle(), format!("Could not read saved settings: {error}")),
                    }
                }
            }
            let shortcut = app.state::<AppState>().0.lock().unwrap().snapshot.config.shortcut.clone();
            if let Err(error) = app.global_shortcut().register(shortcut.as_str()) {
                report_error(app.handle(), format!("Could not register shortcut: {error}. Choose another shortcut in settings."));
            }
            let settings = tauri::menu::MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
            let quit = tauri::menu::MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = tauri::menu::Menu::with_items(app, &[&settings, &quit])?;
            let icon = app.default_window_icon().cloned().unwrap_or_else(|| tauri::image::Image::new_owned([36, 88, 197, 255].repeat(32 * 32), 32, 32));
            tauri::tray::TrayIconBuilder::new().icon(icon).tooltip("Voxen — local dictation").menu(&menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "settings" => open_settings(app),
                    "quit" => app.exit(0),
                    _ => {}
                }).build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if matches!(event, tauri::WindowEvent::Focused(false) | tauri::WindowEvent::CloseRequested { .. }) {
                    let app = window.app_handle();
                    if let Err(error) = set_shortcut_capture(app.clone(), app.state::<AppState>(), false) {
                        report_error(app, error);
                    }
                }
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let window = window.clone();
                    tauri::async_runtime::spawn(async move { let _ = window.destroy(); });
                }
            }
        })
        .invoke_handler(tauri::generate_handler![get_snapshot, list_microphones, save_config, set_shortcut_capture, start_recording, stop_recording, unload_model])
        .build(tauri::generate_context!())
        .expect("error while building Voxen")
        .run(|app, event| match event {
            tauri::RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
            tauri::RunEvent::Exit => {
                let _ = app
                    .state::<AppState>()
                    .0
                    .lock()
                    .unwrap()
                    .inference
                    .send(InferenceCommand::Unload);
                std::thread::sleep(Duration::from_millis(300));
            }
            _ => {}
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_capture_is_averaged_to_mono() {
        let target = Arc::new(Mutex::new(Vec::new()));
        append_mono(&target, [0.2, 0.6, -0.5, 0.5].into_iter(), 2);
        assert_eq!(*target.lock().unwrap(), vec![0.4, 0.0]);
    }

    #[test]
    fn empty_replacements_are_rejected_and_cannot_expand_text() {
        let mut config = default_config();
        config.replacements.push(Replacement {
            from: String::new(),
            to: "oops".into(),
        });
        assert!(validate_config(&config).is_err());
        assert_eq!(
            apply_replacements("hello".into(), &config.replacements),
            "hello"
        );
    }

    #[test]
    fn transcribing_and_inserting_block_new_recordings() {
        assert!(!is_available("starting"));
        assert!(!is_available("transcribing"));
        assert!(!is_available("inserting"));
        assert!(!is_available("recording"));
        assert!(is_available("error"));
    }

    fn backend_for_test() -> Backend {
        let (audio, _audio_receiver) = mpsc::channel();
        let (inference, _inference_receiver) = mpsc::channel();
        Backend {
            snapshot: AppSnapshot {
                config: default_config(),
                phase: "starting".into(),
                error: None,
                last_transcript: None,
                live_transcript: None,
                model_loaded: false,
            },
            audio,
            inference,
            foreground: 0,
            shortcut_capture: false,
            start_cancelled: false,
        }
    }

    #[test]
    fn cancelled_microphone_start_returns_to_idle_after_setup_finishes() {
        let mut backend = backend_for_test();
        backend.start_cancelled = true;
        assert_eq!(
            apply_start_completion(&mut backend, Ok(())),
            StartCompletion::Idle
        );
        assert_eq!(backend.snapshot.phase, "idle");
        assert!(backend.snapshot.error.is_none());
    }

    #[test]
    fn microphone_start_completion_tracks_success_or_error() {
        let mut backend = backend_for_test();
        assert_eq!(
            apply_start_completion(&mut backend, Ok(())),
            StartCompletion::Listening
        );
        assert_eq!(backend.snapshot.phase, "recording");

        let mut backend = backend_for_test();
        assert_eq!(
            apply_start_completion(&mut backend, Err("No microphone is available".into())),
            StartCompletion::Error
        );
        assert_eq!(backend.snapshot.phase, "error");
        assert_eq!(
            backend.snapshot.error.as_deref(),
            Some("No microphone is available")
        );
    }

    #[test]
    fn cleanup_requires_a_mode_and_both_paths() {
        let mut config = default_config();
        config.cleanup_runtime_path = "C:\\runtime".into();
        config.cleanup_model_path = "C:\\cleanup.gguf".into();
        assert!(!cleanup_enabled(&config));
        config.cleanup_mode = cleanup::Mode::Light;
        assert!(cleanup_enabled(&config));
        config.cleanup_model_path = String::new();
        assert!(!cleanup_enabled(&config));
    }

    #[test]
    fn selecting_a_cleanup_mode_requires_cleanup_paths() {
        let mut config = default_config();
        config.cleanup_mode = cleanup::Mode::Polished;
        assert!(validate_config(&config).is_err());
        config.cleanup_runtime_path = "C:\\runtime".into();
        config.cleanup_model_path = "C:\\cleanup.gguf".into();
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn cleanup_preserves_the_original_when_disabled_or_empty() {
        let mut cleaner = None;
        let config = default_config();
        let (text, warning) = apply_cleanup(&mut cleaner, &config, "hello there".into());
        assert_eq!(text, "hello there");
        assert_eq!(warning, None);
    }
}
