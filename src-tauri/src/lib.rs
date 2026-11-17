pub mod cleanup;
pub mod context;
mod hotkey;
pub mod indicator;
mod insertion;
mod library;
mod nemo;
mod normalize;
pub mod theme;
pub mod ui;
pub mod updater;
pub mod widgets;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rune::{
    AppHandle, OverlayHandle, Shortcut, ShortcutId, Tray, TrayEvent, TrayIcon, TrayItem, TraySpec,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Replacement {
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub microphone: Option<String>,
    pub shortcut: String,
    pub activation_mode: String,
    pub runtime_path: String,
    pub model_path: String,
    pub device: String,
    pub replacements: Vec<Replacement>,
    pub unload_after_seconds: u64,
    #[serde(default)]
    pub cleanup_mode: cleanup::Mode,
    #[serde(default)]
    pub cleanup_runtime_path: String,
    #[serde(default)]
    pub cleanup_model_path: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub context_mode: context::Mode,
    #[serde(default)]
    pub excluded_apps: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub config: Config,
    pub phase: String,
    pub error: Option<String>,
    pub last_transcript: Option<String>,
    pub live_transcript: Option<String>,
    pub model_loaded: bool,
}

struct Backend {
    snapshot: AppSnapshot,
    audio: mpsc::Sender<AudioCommand>,
    inference: mpsc::Sender<InferenceCommand>,
    foreground: isize,
    shortcut_capture: bool,
    start_cancelled: bool,
    context: Option<String>,
    cancellation: Option<Cancellation>,
    submission: Option<insertion::SubmissionObserver>,
    shortcut_id: Option<ShortcutId>,
    indicator: Option<OverlayHandle<indicator::Indicator>>,
    indicator_phase: Option<String>,
    indicator_visible: bool,
}

impl Backend {
    fn finish_session(&mut self) {
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.finish();
        }
        self.context = None;
        drop(self.submission.take());
    }
}

#[derive(Clone)]
struct Cancellation {
    cancelled: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
}

impl Cancellation {
    fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            finished: Arc::new(AtomicBool::new(false)),
        }
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    fn finish(&self) {
        self.finished.store(true, Ordering::SeqCst);
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    fn is_finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }
}

#[derive(Clone)]
pub struct Engine {
    backend: Arc<Mutex<Backend>>,
    handle: AppHandle<ui::VoxenApp>,
    revision: Arc<AtomicU64>,
}

type CapturedAudio = Result<(Vec<f32>, u32), String>;
const DEFAULT_UNLOAD_SECONDS: u64 = 300;
const APP_ID: &str = "com.voxen.app";

enum AudioCommand {
    Start(Option<String>, mpsc::Sender<Result<(), String>>),
    Stop(mpsc::Sender<CapturedAudio>),
    Failed(String),
}

enum InferenceCommand {
    Transcribe(
        Config,
        Vec<f32>,
        u32,
        isize,
        Option<String>,
        Cancellation,
    ),
    Unload,
    UnloadAck(mpsc::Sender<()>),
    UnloadCleanup,
    StartLive(Config, isize, Option<String>, Cancellation),
    LiveAudio(Vec<f32>, u32),
}

struct Live {
    stream: nemo::Stream,
    replacements: Vec<Replacement>,
    context: Option<String>,
    target: isize,
    text: String,
    visible: String,
    inserted: String,
    failure: Option<String>,
    cancellation: Cancellation,
}

pub fn default_config() -> Config {
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
        language: default_language(),
        context_mode: context::Mode::Off,
        excluded_apps: vec![],
    }
}

fn default_language() -> String {
    "auto".into()
}

fn cleanup_enabled(config: &Config) -> bool {
    config.cleanup_mode != cleanup::Mode::Raw
        && !config.cleanup_runtime_path.trim().is_empty()
        && !config.cleanup_model_path.trim().is_empty()
}

fn context_useful(config: &Config) -> bool {
    config.context_mode.enabled()
        && (cleanup_enabled(config) || nemo::supports_boost(&config.model_path))
}

fn apply_cleanup(
    cleaner: &mut Option<cleanup::Cleaner>,
    config: &Config,
    text: String,
    context: Option<&str>,
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
        .clean_with_context(text.trim(), config.cleanup_mode, context)
    {
        Ok(cleaned) => (cleaned, None),
        Err(error) => (
            text,
            Some(format!("Cleanup kept the original transcript: {error}")),
        ),
    }
}

fn indicator_active(phase: &str, starting_cancelled: bool) -> bool {
    match phase {
        "starting" => !starting_cancelled,
        "recording" | "transcribing" | "inserting" => true,
        _ => false,
    }
}

fn config_dir() -> Result<PathBuf, String> {
    rune::dirs::AppDirs::new(APP_ID)
        .map(|dirs| dirs.config().to_path_buf())
        .ok_or_else(|| "Could not find the application config folder".into())
}

impl Engine {
    pub fn new(handle: AppHandle<ui::VoxenApp>) -> Self {
        let (audio, audio_rx) = mpsc::channel();
        let (inference, inference_rx) = mpsc::channel();
        let (config, config_error) = load_config();
        let snapshot = AppSnapshot {
            config,
            phase: if config_error.is_some() {
                "error".into()
            } else {
                "idle".into()
            },
            error: config_error,
            last_transcript: None,
            live_transcript: None,
            model_loaded: false,
        };
        let engine = Engine {
            backend: Arc::new(Mutex::new(Backend {
                snapshot,
                audio,
                inference,
                foreground: 0,
                shortcut_capture: false,
                start_cancelled: false,
                context: None,
                cancellation: None,
                submission: None,
                shortcut_id: None,
                indicator: None,
                indicator_phase: None,
                indicator_visible: false,
            })),
            handle,
            revision: Arc::new(AtomicU64::new(0)),
        };
        let audio_engine = engine.clone();
        std::thread::spawn(move || audio_worker(audio_engine, audio_rx));
        let inference_engine = engine.clone();
        std::thread::spawn(move || inference_worker(inference_engine, inference_rx));
        engine
    }

    pub fn snapshot(&self) -> AppSnapshot {
        self.backend.lock().unwrap().snapshot.clone()
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    pub fn list_microphones() -> Result<Vec<String>, String> {
        let mut names = cpal::default_host()
            .input_devices()
            .map_err(|e| e.to_string())?
            .map(|device| device.name().map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        names.sort();
        names.dedup();
        Ok(names)
    }

    pub(crate) fn list_devices(runtime_path: &str) -> Result<Vec<nemo::Device>, String> {
        nemo::list_devices(runtime_path)
    }

    pub fn attach_indicator(&self, indicator: OverlayHandle<indicator::Indicator>) {
        self.backend.lock().unwrap().indicator = Some(indicator);
    }

    fn publish(&self) {
        self.revision.fetch_add(1, Ordering::SeqCst);
        self.sync_indicator();
        self.handle.wake();
    }

    fn sync_indicator(&self) {
        let (show, phase, error, phase_changed, visibility_changed, indicator) = {
            let mut backend = self.backend.lock().unwrap();
            let show = indicator_active(&backend.snapshot.phase, backend.start_cancelled);
            let phase = backend.snapshot.phase.clone();
            let error = backend.snapshot.error.is_some();
            let phase_changed = show && backend.indicator_phase.as_deref() != Some(phase.as_str());
            let visibility_changed = backend.indicator_visible != show;
            if show {
                backend.indicator_phase = Some(phase.clone());
            } else {
                backend.indicator_phase = None;
            }
            backend.indicator_visible = show;
            (
                show,
                phase,
                error,
                phase_changed,
                visibility_changed,
                backend.indicator.clone(),
            )
        };
        let Some(indicator) = indicator else {
            return;
        };
        if visibility_changed {
            if show {
                indicator.show();
            } else {
                indicator.hide();
            }
        }
        if show && phase_changed {
            indicator.update(move |state, _| state.set(&phase, error));
        }
    }

    pub fn report_error(&self, error: String) {
        {
            let mut backend = self.backend.lock().unwrap();
            backend.snapshot.phase = "error".into();
            backend.snapshot.error = Some(error);
        }
        self.publish();
    }

    pub fn save_config(&self, config: Config) -> Result<AppSnapshot, String> {
        let shortcut = validate_config(&config)?;
        let backend = self.backend.lock().unwrap();
        if !is_available(&backend.snapshot.phase) {
            return Err("Wait until dictation finishes before changing settings".into());
        }
        if backend.shortcut_capture {
            return Err("Finish selecting the shortcut before saving".into());
        }
        let old_shortcut = backend.snapshot.config.shortcut.clone();
        let old_id = backend.shortcut_id;
        let changed = config.shortcut != old_shortcut;
        let registered = old_id.is_some();
        let directory = config_dir()?;
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let pending = directory.join("config.new");
        std::fs::write(
            &pending,
            serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        drop(backend);
        let new_id = if changed || !registered {
            match self.register_shortcut(shortcut) {
                Ok(id) => Some(id),
                Err(error) => {
                    let _ = std::fs::remove_file(&pending);
                    return Err(error);
                }
            }
        } else {
            None
        };
        if changed && registered {
            if let Some(old_id) = old_id {
                if let Err(error) = self.handle.unregister_shortcut(old_id) {
                    if let Some(new_id) = new_id {
                        let _ = self.handle.unregister_shortcut(new_id);
                    }
                    let _ = std::fs::remove_file(&pending);
                    return Err(error.to_string());
                }
            }
        }
        if let Err(error) = std::fs::rename(&pending, directory.join("config.json")) {
            if changed && registered {
                if let Ok(old) = old_shortcut.parse::<Shortcut>() {
                    let _ = self.register_shortcut(old);
                }
            }
            if let Some(new_id) = new_id {
                let _ = self.handle.unregister_shortcut(new_id);
            }
            return Err(error.to_string());
        }
        {
            let mut backend = self.backend.lock().unwrap();
            if let Some(new_id) = new_id {
                backend.shortcut_id = Some(new_id);
            }
            if let Err(error) = backend.inference.send(InferenceCommand::Unload) {
                return Err(error.to_string());
            }
            backend.snapshot.config = config;
            backend.snapshot.phase = "idle".into();
            backend.snapshot.error = None;
        }
        self.publish();
        Ok(self.snapshot())
    }

    fn register_shortcut(&self, shortcut: Shortcut) -> Result<ShortcutId, String> {
        let label = shortcut.to_string();
        let owner = self.handle.shortcut_owner(&shortcut);
        self.handle
            .register_shortcut(shortcut, |state, tx| state.on_shortcut(tx))
            .map_err(|error| match error {
                rune::ShortcutError::Conflict => owner
                    .map(|owner| format!("{label} is already used by {owner}"))
                    .unwrap_or_else(|| format!("{label} is already in use")),
                other => other.to_string(),
            })
    }

    pub fn set_shortcut_capture(&self, active: bool) -> Result<(), String> {
        let mut backend = self.backend.lock().unwrap();
        if backend.shortcut_capture == active {
            return Ok(());
        }
        if active && !is_available(&backend.snapshot.phase) {
            return Err("Wait until dictation finishes before changing the shortcut".into());
        }
        let shortcut = backend
            .snapshot
            .config
            .shortcut
            .parse::<Shortcut>()
            .map_err(|error| error.to_string())?;
        if active {
            if let Some(id) = backend.shortcut_id.take() {
                self.handle
                    .unregister_shortcut(id)
                    .map_err(|error| error.to_string())?;
            }
        } else if backend.shortcut_id.is_none() {
            match self.register_shortcut(shortcut) {
                Ok(id) => backend.shortcut_id = Some(id),
                Err(error) => {
                    backend.shortcut_capture = false;
                    return Err(error);
                }
            }
        }
        backend.shortcut_capture = active;
        Ok(())
    }

    pub fn start_recording(&self) -> Result<(), String> {
        let (config, foreground, receiver) = {
            let mut backend = self.backend.lock().unwrap();
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
                backend.foreground = foreground_target();
            }
            let (tx, rx) = mpsc::channel();
            backend.finish_session();
            let cancellation = Cancellation::new();
            let submission = if backend.foreground != 0 {
                match insertion::SubmissionObserver::install(backend.foreground, &cancellation) {
                    Ok(observer) => Some(observer),
                    Err(error) => {
                        cancellation.finish();
                        return Err(format!(
                            "Keyboard submission protection unavailable: {error}"
                        ));
                    }
                }
            } else {
                None
            };
            backend
                .audio
                .send(AudioCommand::Start(
                    backend.snapshot.config.microphone.clone(),
                    tx,
                ))
                .map_err(|error| {
                    cancellation.finish();
                    error.to_string()
                })?;
            backend.cancellation = Some(cancellation);
            backend.submission = submission;
            backend.start_cancelled = false;
            backend.snapshot.phase = "starting".into();
            backend.snapshot.error = None;
            backend.snapshot.live_transcript = None;
            (backend.snapshot.config.clone(), backend.foreground, rx)
        };
        let context = if context_useful(&config) {
            context::capture(
                foreground,
                config.context_mode,
                &config.excluded_apps,
            )
        } else {
            None
        };
        {
            let mut backend = self.backend.lock().unwrap();
            backend.context = context;
        }
        self.publish();
        let engine = self.clone();
        std::thread::spawn(move || complete_start(engine, config, foreground, receiver));
        Ok(())
    }

    pub fn stop_recording(&self) -> Result<(), String> {
        let mut backend = self.backend.lock().unwrap();
        if backend.snapshot.phase == "starting" {
            backend.start_cancelled = true;
            drop(backend);
            self.publish();
            return Ok(());
        }
        if backend.snapshot.phase != "recording" {
            return Err("Recording is not active".into());
        }
        let cancellation = backend
            .cancellation
            .clone()
            .ok_or_else(|| "Recording is not active".to_string())?;
        let job = (|| {
            let (tx, rx) = mpsc::channel();
            backend
                .audio
                .send(AudioCommand::Stop(tx))
                .map_err(|e| e.to_string())?;
            let (samples, rate) = rx.recv().map_err(|e| e.to_string())??;
            backend.snapshot.phase = "transcribing".into();
            Ok::<InferenceCommand, String>(InferenceCommand::Transcribe(
                backend.snapshot.config.clone(),
                samples,
                rate,
                backend.foreground,
                backend.context.clone(),
                cancellation.clone(),
            ))
        })();
        let job = match job {
            Ok(job) => job,
            Err(error) => {
                backend.finish_session();
                return Err(error);
            }
        };
        drop(backend);
        self.publish();
        let result = self.backend.lock().unwrap().inference.send(job);
        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                self.backend.lock().unwrap().finish_session();
                Err(error.to_string())
            }
        }
    }

    pub fn unload_model(&self) -> Result<(), String> {
        let backend = self.backend.lock().unwrap();
        if !is_available(&backend.snapshot.phase) {
            return Err("Wait until dictation finishes before unloading the model".into());
        }
        backend
            .inference
            .send(InferenceCommand::Unload)
            .map_err(|e| e.to_string())
    }

    pub fn remove_managed_model(&self, path: &str) -> Result<(), String> {
        self.unload_and_wait()?;
        std::fs::remove_file(path).map_err(|error| format!("Could not remove the model: {error}"))
    }

    pub fn remove_managed_runtime(&self, path: &str) -> Result<(), String> {
        self.unload_and_wait()?;
        std::fs::remove_dir_all(path)
            .map_err(|error| format!("Could not remove the runtime: {error}"))
    }

    fn unload_and_wait(&self) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        {
            let backend = self.backend.lock().unwrap();
            if !is_available(&backend.snapshot.phase) {
                return Err("Wait until dictation finishes before removing files".into());
            }
            backend
                .inference
                .send(InferenceCommand::UnloadAck(tx))
                .map_err(|e| e.to_string())?;
        }
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| "The model did not unload in time".to_string())
    }

    pub fn handle_shortcut(&self) {
        let (mode, phase, capture) = {
            let backend = self.backend.lock().unwrap();
            (
                backend.snapshot.config.activation_mode.clone(),
                backend.snapshot.phase.clone(),
                backend.shortcut_capture,
            )
        };
        if capture {
            return;
        }
        let result = match (mode.as_str(), phase.as_str()) {
            ("hold", phase) if is_available(phase) => self.start_recording(),
            ("hold", "starting" | "recording") => self.stop_recording(),
            ("toggle", "starting" | "recording") => self.stop_recording(),
            ("toggle", phase) if is_available(phase) => self.start_recording(),
            _ => Ok(()),
        };
        if let Err(error) = result {
            self.report_error(error);
        }
    }

    pub fn shortcut_down(&self) -> bool {
        let shortcut = self
            .backend
            .lock()
            .unwrap()
            .snapshot
            .config
            .shortcut
            .clone();
        hotkey::shortcut_down(&shortcut)
    }

    pub fn open_window(&self) {
        self.handle.open();
    }

    pub fn hide_window(&self) {
        self.handle.hide();
    }

    pub fn quit(&self) {
        self.handle.quit();
    }

    pub fn shutdown(&self) {
        let _ = self
            .backend
            .lock()
            .unwrap()
            .inference
            .send(InferenceCommand::Unload);
        std::thread::sleep(Duration::from_millis(300));
    }

    fn ensure_shortcut(&self) {
        let shortcut = {
            let backend = self.backend.lock().unwrap();
            if backend.shortcut_id.is_some() || backend.shortcut_capture {
                return;
            }
            backend.snapshot.config.shortcut.clone()
        };
        let Ok(shortcut) = shortcut.parse::<Shortcut>() else {
            self.report_error(format!(
                "Could not register shortcut: invalid binding {shortcut}. Choose another shortcut in settings."
            ));
            return;
        };
        match self.register_shortcut(shortcut) {
            Ok(id) => {
                self.backend.lock().unwrap().shortcut_id = Some(id);
                self.publish();
            }
            Err(error) => self.report_error(format!(
                "Could not register shortcut: {error}. Choose another shortcut in settings."
            )),
        }
    }
}

fn load_config() -> (Config, Option<String>) {
    let Ok(directory) = config_dir() else {
        return (default_config(), None);
    };
    let Ok(contents) = std::fs::read(directory.join("config.json")) else {
        return (default_config(), None);
    };
    match serde_json::from_slice::<Config>(&contents) {
        Ok(config) if validate_config(&config).is_ok() => (config, None),
        Ok(_) => (
            default_config(),
            Some("Saved settings are invalid. Review and save settings again.".into()),
        ),
        Err(error) => (
            default_config(),
            Some(format!("Could not read saved settings: {error}")),
        ),
    }
}

fn audio_worker(engine: Engine, receiver: mpsc::Receiver<AudioCommand>) {
    let sender = engine.backend.lock().unwrap().audio.clone();
    let inference = engine.backend.lock().unwrap().inference.clone();
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
                    engine.backend.lock().unwrap().finish_session();
                    engine.report_error(error);
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

fn inference_worker(engine: Engine, receiver: mpsc::Receiver<InferenceCommand>) {
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
                engine.backend.lock().unwrap().snapshot.model_loaded = false;
                engine.publish();
            }
            InferenceCommand::UnloadAck(reply) => {
                live = None;
                model = None;
                cleaner = None;
                deadline = None;
                engine.backend.lock().unwrap().snapshot.model_loaded = false;
                engine.publish();
                let _ = reply.send(());
            }
            InferenceCommand::UnloadCleanup => {
                cleaner = None;
                deadline = None;
            }
            InferenceCommand::StartLive(config, target, context, cancellation) => {
                live = None;
                deadline = None;
                match start_live(&engine, &mut model, &config, target, context, cancellation) {
                    Ok(session) => live = Some(session),
                    Err(error) => {
                        engine.backend.lock().unwrap().snapshot.error =
                            Some(format!("Live typing is unavailable: {error}"));
                        engine.publish();
                    }
                }
            }
            InferenceCommand::LiveAudio(samples, rate) => {
                if let Some(session) = live.as_mut() {
                    if session.failure.is_none() {
                        match session.stream.push(&samples, rate) {
                            Ok(()) => pump_live(&engine, session),
                            Err(error) => session.failure = Some(error),
                        }
                    }
                }
            }
            InferenceCommand::Transcribe(
                config,
                samples,
                rate,
                foreground,
                context,
                cancellation,
            ) => {
                let mut cleanup_warning: Option<String> = None;
                let result = (|| {
                    if let Some(mut session) = live.take() {
                        if session.failure.is_none() {
                            match session.stream.finish() {
                                Ok(()) => pump_live(&engine, &mut session),
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
                            let (cleaned, warning) = apply_cleanup(
                                &mut cleaner,
                                &config,
                                transcript.clone(),
                                session.context.as_deref(),
                            );
                            cleanup_warning = warning;
                            let cleaned = normalize::normalize(&cleaned);
                            if cleaned != transcript
                                && session.target != 0
                                && !session.cancellation.is_cancelled()
                            {
                                engine.backend.lock().unwrap().snapshot.phase = "inserting".into();
                                engine.publish();
                                if let Err(failure) = insertion::replace_text(
                                    session.target,
                                    &session.inserted,
                                    &cleaned,
                                    &session.cancellation,
                                ) {
                                    error = Some(failure);
                                }
                            }
                            transcript = cleaned;
                        }
                        {
                            let mut backend = engine.backend.lock().unwrap();
                            backend.snapshot.last_transcript = Some(transcript);
                            backend.snapshot.live_transcript = None;
                        }
                        engine.publish();
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
                            &config.language,
                        )?);
                        engine.backend.lock().unwrap().snapshot.model_loaded = true;
                        engine.publish();
                    }
                    let terms = context
                        .as_deref()
                        .map(context::terms)
                        .unwrap_or_default();
                    let text = model
                        .as_mut()
                        .unwrap()
                        .transcribe(&samples, rate, &terms)?;
                    let text = apply_replacements(text, &config.replacements);
                    let (text, warning) =
                        apply_cleanup(&mut cleaner, &config, text, context.as_deref());
                    cleanup_warning = warning;
                    let text = normalize::normalize(&text);
                    engine.backend.lock().unwrap().snapshot.last_transcript = Some(text.clone());
                    if !text.trim().is_empty() && foreground != 0 && !cancellation.is_cancelled() {
                        engine.backend.lock().unwrap().snapshot.phase = "inserting".into();
                        engine.publish();
                        #[cfg(target_os = "windows")]
                        insertion::insert_text(foreground, &text, &cancellation)?;
                    }
                    Ok::<(), String>(())
                })();
                engine.backend.lock().unwrap().finish_session();
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
                        {
                            let mut backend = engine.backend.lock().unwrap();
                            backend.snapshot.phase = "idle".into();
                            backend.snapshot.error = cleanup_warning;
                        }
                        engine.publish();
                    }
                    Err(error) => engine.report_error(error),
                }
            }
        }
    }
}

fn start_live(
    engine: &Engine,
    model: &mut Option<nemo::Recognizer>,
    config: &Config,
    target: isize,
    context: Option<String>,
    cancellation: Cancellation,
) -> Result<Live, String> {
    if model.is_none() {
        *model = Some(nemo::Recognizer::load(
            &config.runtime_path,
            &config.model_path,
            &config.device,
            &config.language,
        )?);
        engine.backend.lock().unwrap().snapshot.model_loaded = true;
        engine.publish();
    }
    Ok(Live {
        stream: model.as_ref().unwrap().stream(
            &context
                .as_deref()
                .map(context::terms)
                .unwrap_or_default(),
        )?,
        replacements: config.replacements.clone(),
        context,
        target,
        text: String::new(),
        visible: String::new(),
        inserted: String::new(),
        failure: None,
        cancellation,
    })
}

fn pump_live(engine: &Engine, session: &mut Live) {
    loop {
        match session.stream.next() {
            Ok(Some((transcript, is_final))) => {
                apply_live(engine, session, transcript, is_final);
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

fn apply_live(engine: &Engine, session: &mut Live, transcript: String, is_final: bool) {
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
    if session.target != 0 && !session.cancellation.is_cancelled() {
        match insertion::replace_text(
            session.target,
            &session.inserted,
            &visible,
            &session.cancellation,
        ) {
            Ok(()) => session.inserted = visible.clone(),
            Err(error) => session.failure = Some(error),
        }
    }
    session.visible = visible;
    engine.backend.lock().unwrap().snapshot.live_transcript =
        (!session.visible.is_empty()).then_some(session.visible.clone());
    engine.publish();
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
    engine: Engine,
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
        let mut backend = engine.backend.lock().unwrap();
        if started && backend.start_cancelled {
            let (tx, _rx) = mpsc::channel();
            let _ = backend.audio.send(AudioCommand::Stop(tx));
        }
        let completion = apply_start_completion(&mut backend, result);
        if completion == StartCompletion::Listening {
            if let Some(cancellation) = backend.cancellation.clone() {
                if nemo::supports_streaming(&config.model_path) {
                    let _ = backend.inference.send(InferenceCommand::StartLive(
                        config,
                        foreground,
                        backend.context.clone(),
                        cancellation,
                    ));
                }
            }
        } else {
            backend.finish_session();
        }
    }
    engine.publish();
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
    if nemo::language_code(&config.language).is_none() {
        return Err("Choose a language supported by the installed speech model".into());
    }
    let model_path = config.model_path.to_lowercase();
    if model_path.contains("parakeet-tdt") && !nemo::parakeet_supports(&config.language) {
        return Err("Parakeet TDT v3 does not support the selected language".into());
    }
    if model_path.contains("nemotron-speech-streaming-en")
        && !nemo::english_supports(&config.language)
    {
        return Err("The Nemotron English model supports English only".into());
    }
    if model_path.contains("nemotron-3.5") && !nemo::nemotron_supports(&config.language) {
        return Err(
            "Nemotron 3.5 does not offer the selected locale out of the box; choose Parakeet or another locale"
                .into(),
        );
    }
    if config
        .excluded_apps
        .iter()
        .any(|application| application.trim().is_empty())
    {
        return Err("Excluded application names cannot be empty".into());
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

#[cfg(target_os = "windows")]
fn foreground_target() -> isize {
    use windows::Win32::System::Threading::GetCurrentProcessId;
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    let window = unsafe { GetForegroundWindow() };
    if window.is_invalid() {
        return 0;
    }
    let mut process = 0u32;
    unsafe {
        GetWindowThreadProcessId(window, Some(&mut process));
    }
    if process == unsafe { GetCurrentProcessId() } {
        0
    } else {
        window.0 as isize
    }
}

fn tray_icon() -> Option<TrayIcon> {
    let bytes = include_bytes!("../icons/icon.png");
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    buffer.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer
            .chunks(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buffer
            .chunks(2)
            .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
            .collect(),
        png::ColorType::Grayscale => buffer
            .iter()
            .flat_map(|value| [*value, *value, *value, 255])
            .collect(),
        png::ColorType::Indexed => return None,
    };
    let size = 32u32;
    let mut small = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        let source_y = y * info.height / size;
        for x in 0..size {
            let source_x = x * info.width / size;
            let index = ((source_y * info.width + source_x) * 4) as usize;
            small.extend_from_slice(&rgba[index..index + 4]);
        }
    }
    Some(TrayIcon::rgba(size, size, small))
}

fn ui_poll(state: &mut ui::VoxenApp, clock: f64, tx: &mut rune::Tx) -> bool {
    state.poll(clock, tx)
}

fn ui_key(state: &mut ui::VoxenApp, event: &rune::KeyEvent, tx: &mut rune::Tx) -> bool {
    state.on_key(event, tx)
}

fn start_tray(engine: &Engine, handle: AppHandle<ui::VoxenApp>) {
    let Some(icon) = tray_icon() else {
        engine.report_error("Could not load the tray icon".into());
        return;
    };
    let settings = handle.clone();
    let quit = handle.clone();
    let tray = Tray::new(
        TraySpec::new(APP_ID, "Voxen", icon)
            .tooltip("Voxen — local dictation")
            .menu(vec![
                TrayItem::action("settings", "Settings"),
                TrayItem::action("quit", "Quit"),
            ]),
        move |event| match event {
            TrayEvent::Activate { .. } => settings.open(),
            TrayEvent::Menu(id) if id == "settings" => settings.open(),
            TrayEvent::Menu(id) if id == "quit" => quit.quit(),
            _ => {}
        },
    );
    match tray {
        Ok(tray) => std::mem::forget(tray),
        Err(error) => engine.report_error(format!("Could not create the tray icon: {error}")),
    }
}

pub fn run() {
    rune::set_app_id(APP_ID);
    let _ = rune::set_app_icon(include_bytes!("../icons/icon.png").to_vec());
    let spec = rune::AppSpec {
        min_size: Some((720.0, 520.0)),
        poll: Some(ui_poll),
        on_key: Some(ui_key),
        ..rune::AppSpec::new(
            "Voxen",
            (900.0, 680.0),
            || unreachable!("state is built by App::init"),
            ui::VoxenApp::view,
        )
    };
    let mut app = rune::App::new(spec)
        .keep_running()
        .release_gpu_when_closed()
        .allow_unmodified_shortcuts()
        .modifier_keys()
        .on_close(|state, tx| state.on_close(tx))
        .on_focus(|state, focused, tx| state.on_focus(focused, tx))
        .on_quit(|state, _| state.engine.shutdown());
    let handle = app.handle();
    let engine = Engine::new(handle.clone());
    if !rune::is_headless() {
        let indicator = app.overlay(
            rune::OverlaySpec {
                name: "voxen-indicator",
                size: (220.0, 76.0),
                edge: rune::ScreenEdge::Bottom,
                margin: 40.0,
                align: rune::Align::Center,
                inset: 0.0,
                interactive: false,
                view: indicator::view,
            },
            indicator::Indicator::new(),
        );
        engine.attach_indicator(indicator);
        start_tray(&engine, handle.clone());
    }
    let startup = engine.clone();
    handle.update(move |_, _| startup.ensure_shortcut());
    let init_engine = engine.clone();
    app = app.init(move || ui::VoxenApp::new(init_engine));
    app.run();
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
    fn context_is_only_captured_when_it_can_be_used() {
        let mut config = default_config();
        config.context_mode = context::Mode::Nearby;
        config.model_path = "C:\\models\\nemotron-speech-streaming-en-0.6b.q8_0.gguf".into();
        assert!(!context_useful(&config));
        config.cleanup_mode = cleanup::Mode::Light;
        config.cleanup_runtime_path = "C:\\runtime".into();
        config.cleanup_model_path = "C:\\cleanup.gguf".into();
        assert!(context_useful(&config));
        config.context_mode = context::Mode::Off;
        assert!(!context_useful(&config));
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
    fn indicator_visibility_follows_the_phase() {
        assert!(indicator_active("recording", false));
        assert!(indicator_active("transcribing", false));
        assert!(!indicator_active("idle", false));
        assert!(indicator_active("starting", false));
        assert!(!indicator_active("starting", true));
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
            context: None,
            cancellation: None,
            submission: None,
            shortcut_id: None,
            indicator: None,
            indicator_phase: None,
            indicator_visible: false,
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
    fn cancellation_is_scoped_to_one_dictation_session() {
        let first = Cancellation::new();
        let second = Cancellation::new();
        first.cancel();
        assert!(first.is_cancelled());
        assert!(!second.is_cancelled());
        second.finish();
        assert!(!first.is_finished());
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
}
