use libloading::os::windows::{
    Library, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
};
use std::ffi::{c_char, c_void, CStr, CString};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

type Handle = *mut c_void;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub const LANGUAGES: &[(&str, &str)] = &[
    ("auto", "Automatic"),
    ("en-US", "English (US)"),
    ("en-GB", "English (UK)"),
    ("es-US", "Spanish (US)"),
    ("es-ES", "Spanish (Spain)"),
    ("fr-FR", "French (France)"),
    ("fr-CA", "French (Canada)"),
    ("it-IT", "Italian"),
    ("pt-BR", "Portuguese (Brazil)"),
    ("pt-PT", "Portuguese (Portugal)"),
    ("nl-NL", "Dutch"),
    ("de-DE", "German"),
    ("tr-TR", "Turkish"),
    ("ru-RU", "Russian"),
    ("ar-AR", "Arabic"),
    ("hi-IN", "Hindi"),
    ("ja-JP", "Japanese"),
    ("ko-KR", "Korean"),
    ("vi-VN", "Vietnamese"),
    ("uk-UA", "Ukrainian"),
    ("pl-PL", "Polish"),
    ("sv-SE", "Swedish"),
    ("cs-CZ", "Czech"),
    ("nb-NO", "Norwegian Bokmål"),
    ("da-DK", "Danish"),
    ("bg-BG", "Bulgarian"),
    ("fi-FI", "Finnish"),
    ("hr-HR", "Croatian"),
    ("sk-SK", "Slovak"),
    ("zh-CN", "Mandarin"),
    ("hu-HU", "Hungarian"),
    ("ro-RO", "Romanian"),
    ("et-EE", "Estonian"),
    ("el-GR", "Greek (Parakeet)"),
    ("lv-LV", "Latvian (Parakeet)"),
    ("lt-LT", "Lithuanian (Parakeet)"),
    ("mt-MT", "Maltese (Parakeet)"),
    ("sl-SI", "Slovenian (Parakeet)"),
];

const NEMOTRON_PRODUCTION: &[&str] = &[
    "en-US", "en-GB", "es-US", "es-ES", "fr-FR", "fr-CA", "it-IT", "pt-BR", "pt-PT", "nl-NL",
    "de-DE", "tr-TR", "ru-RU", "ar-AR", "hi-IN", "ja-JP", "ko-KR", "vi-VN", "uk-UA", "pl-PL",
    "sv-SE", "cs-CZ", "nb-NO", "da-DK", "bg-BG", "fi-FI", "hr-HR", "sk-SK", "zh-CN", "hu-HU",
    "ro-RO", "et-EE",
];

const PARAKEET_LANGUAGES: &[&str] = &[
    "en", "es", "fr", "de", "bg", "hr", "cs", "da", "nl", "et", "fi", "el", "hu", "it", "lv", "lt",
    "mt", "pl", "pt", "ro", "sk", "sl", "sv", "ru", "uk",
];

pub fn language_code(language: &str) -> Option<&'static str> {
    let language = language.trim();
    LANGUAGES
        .iter()
        .find(|(code, _)| code.eq_ignore_ascii_case(language))
        .map(|(code, _)| *code)
}

pub fn language_base(language: &str) -> &str {
    language.split('-').next().unwrap_or(language)
}

pub fn nemotron_supports(language: &str) -> bool {
    language.eq_ignore_ascii_case("auto")
        || NEMOTRON_PRODUCTION
            .iter()
            .any(|code| code.eq_ignore_ascii_case(language))
}

pub fn parakeet_supports(language: &str) -> bool {
    language.eq_ignore_ascii_case("auto")
        || PARAKEET_LANGUAGES.contains(&language_base(language))
}

pub fn english_supports(language: &str) -> bool {
    language.eq_ignore_ascii_case("auto") || language_base(language).eq_ignore_ascii_case("en")
}

pub fn supports_streaming(model_path: &str) -> bool {
    !file_name(model_path).contains("parakeet-tdt")
}

pub fn supports_boost(model_path: &str) -> bool {
    file_name(model_path).contains("nemotron") && has_embedded_tokenizer(model_path)
}

fn has_embedded_tokenizer(model_path: &str) -> bool {
    use std::io::Read;

    const KEY: &[u8] = b"asr.tokenizer.spm_model";
    const SCAN_BYTES: usize = 2 * 1024 * 1024;

    let Ok(mut file) = std::fs::File::open(model_path) else {
        return false;
    };
    let mut head = vec![0u8; SCAN_BYTES];
    let mut filled = 0;
    while filled < head.len() {
        match file.read(&mut head[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(_) => return false,
        }
    }
    head[..filled]
        .windows(KEY.len())
        .any(|window| window == KEY)
}

fn uses_language_hint(model_path: &str) -> bool {
    file_name(model_path).contains("nemotron")
}

fn file_name(model_path: &str) -> String {
    Path::new(model_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_lowercase()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub label: String,
}

#[repr(C)]
struct BackendConfig {
    size: usize,
    gpu: i32,
}
#[repr(C)]
struct ModelConfig {
    size: usize,
    path: *const c_char,
    name: *const c_char,
}
#[repr(C)]
struct RecognizerConfig {
    size: usize,
    backend: *const BackendConfig,
    model: *const ModelConfig,
    streaming: *const c_void,
    decoder: *const c_void,
    vad: *const c_void,
    endpointing: *const c_void,
    postproc: *const c_void,
    diar: *const c_void,
    batching: *const c_void,
}
#[repr(C)]
struct Options {
    size: usize,
    request_id: *const c_char,
    language_code: *const c_char,
    interim_results: bool,
    enable_word_time_offsets: bool,
    enable_automatic_punctuation: bool,
    verbatim_transcripts: bool,
    profanity_filter: bool,
    stop_history_eou_ms: i32,
    speech_contexts: *const c_void,
    speech_context_count: usize,
    max_alternatives: i32,
    enable_speaker_diarization: bool,
    max_speaker_count: i32,
}

#[repr(C)]
struct SpeechContext {
    size: usize,
    phrases: *const *const c_char,
    phrase_count: usize,
    boost: f32,
}

struct ContextBias {
    _phrases: Vec<CString>,
    _pointers: Vec<*const c_char>,
    context: SpeechContext,
}

impl ContextBias {
    fn new(terms: &[String]) -> Self {
        let phrases: Vec<CString> = terms
            .iter()
            .filter(|term| !term.trim().is_empty())
            .filter_map(|term| CString::new(term.as_str()).ok())
            .collect();
        let pointers: Vec<*const c_char> =
            phrases.iter().map(|phrase| phrase.as_ptr()).collect();
        let context = SpeechContext {
            size: std::mem::size_of::<SpeechContext>(),
            phrases: pointers.as_ptr(),
            phrase_count: pointers.len(),
            boost: 3.0,
        };
        Self {
            _phrases: phrases,
            _pointers: pointers,
            context,
        }
    }

    fn apply(&self, options: &mut Options) {
        if self.context.phrase_count > 0 {
            options.speech_contexts = &self.context as *const SpeechContext as *const c_void;
            options.speech_context_count = 1;
        }
    }
}

#[derive(Clone, Copy)]
struct Api {
    create: unsafe extern "C" fn(*const RecognizerConfig, *mut Handle) -> i32,
    destroy: unsafe extern "C" fn(Handle),
    recognize:
        unsafe extern "C" fn(Handle, *const Options, *const f32, usize, i32, *mut Handle) -> i32,
    stream_start: unsafe extern "C" fn(Handle, *const Options, *mut Handle) -> i32,
    stream_push: unsafe extern "C" fn(Handle, *const f32, usize, i32) -> i32,
    stream_finish: unsafe extern "C" fn(Handle) -> i32,
    stream_next: unsafe extern "C" fn(Handle, *mut Handle) -> i32,
    stream_close: unsafe extern "C" fn(Handle),
    result_destroy: unsafe extern "C" fn(Handle),
    result_alternative_count: unsafe extern "C" fn(*const c_void) -> usize,
    result_transcript: unsafe extern "C" fn(*const c_void, usize) -> *const c_char,
    result_is_final: unsafe extern "C" fn(*const c_void) -> bool,
    options_default: unsafe extern "C" fn() -> Options,
    last_error: unsafe extern "C" fn() -> *const c_char,
}

impl Api {
    unsafe fn load(library: &Library) -> Result<Self, String> {
        let create = *library
            .get::<unsafe extern "C" fn(*const RecognizerConfig, *mut Handle) -> i32>(
                b"nemo_speech_asr_create\0",
            )
            .map_err(|e| e.to_string())?;
        let destroy = *library
            .get::<unsafe extern "C" fn(Handle)>(b"nemo_speech_asr_destroy\0")
            .map_err(|e| e.to_string())?;
        let recognize =
            *library
                .get::<unsafe extern "C" fn(
                    Handle,
                    *const Options,
                    *const f32,
                    usize,
                    i32,
                    *mut Handle,
                ) -> i32>(b"nemo_speech_asr_recognize_f32\0")
                .map_err(|e| e.to_string())?;
        let stream_start = *library
            .get::<unsafe extern "C" fn(Handle, *const Options, *mut Handle) -> i32>(
                b"nemo_speech_asr_streaming_recognize\0",
            )
            .map_err(|e| e.to_string())?;
        let stream_push = *library
            .get::<unsafe extern "C" fn(Handle, *const f32, usize, i32) -> i32>(
                b"nemo_speech_asr_stream_push_f32\0",
            )
            .map_err(|e| e.to_string())?;
        let stream_finish = *library
            .get::<unsafe extern "C" fn(Handle) -> i32>(b"nemo_speech_asr_stream_finish\0")
            .map_err(|e| e.to_string())?;
        let stream_next = *library
            .get::<unsafe extern "C" fn(Handle, *mut Handle) -> i32>(
                b"nemo_speech_asr_stream_next\0",
            )
            .map_err(|e| e.to_string())?;
        let stream_close = *library
            .get::<unsafe extern "C" fn(Handle)>(b"nemo_speech_asr_stream_close\0")
            .map_err(|e| e.to_string())?;
        let result_destroy = *library
            .get::<unsafe extern "C" fn(Handle)>(b"nemo_speech_asr_result_destroy\0")
            .map_err(|e| e.to_string())?;
        let result_alternative_count = *library
            .get::<unsafe extern "C" fn(*const c_void) -> usize>(
                b"nemo_speech_asr_result_alternative_count\0",
            )
            .map_err(|e| e.to_string())?;
        let result_transcript = *library
            .get::<unsafe extern "C" fn(*const c_void, usize) -> *const c_char>(
                b"nemo_speech_asr_result_transcript\0",
            )
            .map_err(|e| e.to_string())?;
        let result_is_final = *library
            .get::<unsafe extern "C" fn(*const c_void) -> bool>(
                b"nemo_speech_asr_result_is_final\0",
            )
            .map_err(|e| e.to_string())?;
        let options_default = *library
            .get::<unsafe extern "C" fn() -> Options>(
                b"nemo_speech_asr_recognition_options_default\0",
            )
            .map_err(|e| e.to_string())?;
        let last_error = *library
            .get::<unsafe extern "C" fn() -> *const c_char>(b"nemo_speech_asr_last_error\0")
            .map_err(|e| e.to_string())?;
        Ok(Api {
            create,
            destroy,
            recognize,
            stream_start,
            stream_push,
            stream_finish,
            stream_next,
            stream_close,
            result_destroy,
            result_alternative_count,
            result_transcript,
            result_is_final,
            options_default,
            last_error,
        })
    }
}

pub struct Recognizer {
    handle: Handle,
    api: Api,
    language: Option<CString>,
    streaming: bool,
    boosting: bool,
    _library: Arc<Library>,
}

impl Recognizer {
    pub fn load(
        runtime_path: &str,
        model_path: &str,
        device: &str,
        language: &str,
    ) -> Result<Self, String> {
        let gpu = parse_device(device)?;
        let name = file_name(model_path);
        let streaming = supports_streaming(model_path);
        let boosting = supports_boost(model_path);
        if name.contains("nemotron-speech-streaming-en") && !english_supports(language) {
            return Err("The Nemotron English model supports English only".into());
        }
        let language = if uses_language_hint(model_path) {
            let code = language_code(language)
                .ok_or_else(|| format!("Unsupported recognition language: {language}"))?;
            Some(CString::new(code).map_err(|e| e.to_string())?)
        } else {
            None
        };
        let dll = resolve_library(runtime_path)?;
        let library = unsafe {
            Library::load_with_flags(
                &dll,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            )
        }
        .map_err(|e| format!("Failed to load {}: {e}", dll.display()))?;
        library.pin().map_err(|e| e.to_string())?;
        let api = unsafe { Api::load(&library)? };
        let model = CString::new(model_path).map_err(|e| e.to_string())?;
        let model_config = ModelConfig {
            size: std::mem::size_of::<ModelConfig>(),
            path: model.as_ptr(),
            name: std::ptr::null(),
        };
        let backend = BackendConfig {
            size: std::mem::size_of::<BackendConfig>(),
            gpu,
        };
        let config = RecognizerConfig {
            size: std::mem::size_of::<RecognizerConfig>(),
            backend: &backend,
            model: &model_config,
            streaming: std::ptr::null(),
            decoder: std::ptr::null(),
            vad: std::ptr::null(),
            endpointing: std::ptr::null(),
            postproc: std::ptr::null(),
            diar: std::ptr::null(),
            batching: std::ptr::null(),
        };
        let mut handle = std::ptr::null_mut();
        let status = unsafe { (api.create)(&config, &mut handle) };
        if status != 0 {
            let error = unsafe { last_error(&api) };
            return Err(error);
        }
        if handle.is_null() {
            return Err("The NeMo-Speech runtime created no recognizer".into());
        }
        Ok(Recognizer {
            handle,
            api,
            language,
            streaming,
            boosting,
            _library: Arc::new(library),
        })
    }

    fn options(&self) -> Options {
        let mut options = unsafe { (self.api.options_default)() };
        options.size = std::mem::size_of::<Options>();
        if let Some(language) = &self.language {
            options.language_code = language.as_ptr();
        }
        options.enable_automatic_punctuation = true;
        options
    }

    pub fn transcribe(
        &mut self,
        samples: &[f32],
        sample_rate: u32,
        terms: &[String],
    ) -> Result<String, String> {
        if samples.is_empty() {
            return Err("No audio was captured".into());
        }
        let bias = ContextBias::new(if self.boosting { terms } else { &[] });
        let mut options = self.options();
        bias.apply(&mut options);
        let mut result: Handle = std::ptr::null_mut();
        let status = unsafe {
            (self.api.recognize)(
                self.handle,
                &options,
                samples.as_ptr(),
                samples.len(),
                sample_rate as i32,
                &mut result,
            )
        };
        if status != 0 {
            let error = unsafe { last_error(&self.api) };
            if !result.is_null() {
                unsafe { (self.api.result_destroy)(result) };
            }
            return Err(error);
        }
        if result.is_null() {
            return Err("The NeMo-Speech runtime returned no result".into());
        }
        let alternatives = unsafe { (self.api.result_alternative_count)(result) };
        let transcript = if alternatives == 0 {
            Err("The NeMo-Speech runtime returned no alternatives".into())
        } else {
            Ok(unsafe { read_transcript(&self.api, result, 0) })
        };
        unsafe { (self.api.result_destroy)(result) };
        transcript
    }

    pub fn stream(&self, terms: &[String]) -> Result<Stream, String> {
        if !self.streaming {
            return Err("This model is offline-only".into());
        }
        let bias = ContextBias::new(if self.boosting { terms } else { &[] });
        let mut options = self.options();
        bias.apply(&mut options);
        options.interim_results = true;
        let mut handle = std::ptr::null_mut();
        let status = unsafe { (self.api.stream_start)(self.handle, &options, &mut handle) };
        if status != 0 {
            return Err(unsafe { last_error(&self.api) });
        }
        if handle.is_null() {
            return Err("The NeMo-Speech runtime created no stream".into());
        }
        Ok(Stream {
            handle,
            api: self.api,
            _library: self._library.clone(),
        })
    }
}

pub struct Stream {
    handle: Handle,
    api: Api,
    _library: Arc<Library>,
}

impl Stream {
    pub fn push(&mut self, samples: &[f32], sample_rate: u32) -> Result<(), String> {
        if samples.is_empty() {
            return Ok(());
        }
        let status = unsafe {
            (self.api.stream_push)(
                self.handle,
                samples.as_ptr(),
                samples.len(),
                sample_rate as i32,
            )
        };
        if status != 0 {
            return Err(unsafe { last_error(&self.api) });
        }
        Ok(())
    }

    pub fn finish(&mut self) -> Result<(), String> {
        let status = unsafe { (self.api.stream_finish)(self.handle) };
        if status != 0 {
            return Err(unsafe { last_error(&self.api) });
        }
        Ok(())
    }

    pub fn next(&mut self) -> Result<Option<(String, bool)>, String> {
        let mut result: Handle = std::ptr::null_mut();
        let status = unsafe { (self.api.stream_next)(self.handle, &mut result) };
        if status != 0 {
            return Err(unsafe { last_error(&self.api) });
        }
        if result.is_null() {
            return Ok(None);
        }
        let is_final = unsafe { (self.api.result_is_final)(result) };
        let alternatives = unsafe { (self.api.result_alternative_count)(result) };
        let transcript = if alternatives == 0 {
            String::new()
        } else {
            unsafe { read_transcript(&self.api, result, 0) }
        };
        unsafe { (self.api.result_destroy)(result) };
        Ok(Some((transcript, is_final)))
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { (self.api.stream_close)(self.handle) };
        }
    }
}

impl Drop for Recognizer {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { (self.api.destroy)(self.handle) };
        }
    }
}

unsafe fn read_transcript(api: &Api, result: Handle, alternative: usize) -> String {
    let pointer = (api.result_transcript)(result, alternative);
    if pointer.is_null() {
        String::new()
    } else {
        CStr::from_ptr(pointer).to_string_lossy().into_owned()
    }
}

unsafe fn last_error(api: &Api) -> String {
    let pointer = (api.last_error)();
    if pointer.is_null() {
        return "The NeMo-Speech runtime reported an error".into();
    }
    CStr::from_ptr(pointer).to_string_lossy().into_owned()
}

fn parse_device(device: &str) -> Result<i32, String> {
    if device.eq_ignore_ascii_case("cpu") {
        return Ok(-1);
    }
    match device.strip_prefix("vulkan:") {
        Some(index) => index
            .parse::<i32>()
            .ok()
            .filter(|value| *value >= 0)
            .ok_or_else(|| format!("Device must be vulkan:<index> or cpu, got {device}")),
        None => Err(format!(
            "Device must be vulkan:<index> or cpu, got {device}"
        )),
    }
}

pub fn list_devices(runtime_path: &str) -> Result<Vec<Device>, String> {
    let cli = resolve_cli(runtime_path)?;
    let output = Command::new(&cli)
        .args(["doctor", "--json"])
        .stdin(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| format!("Could not inspect the runtime: {error}"))?;
    if !output.status.success() {
        return Err("The runtime could not list its devices".into());
    }
    devices_from_doctor(&output.stdout)
}

fn devices_from_doctor(json: &[u8]) -> Result<Vec<Device>, String> {
    let value: serde_json::Value =
        serde_json::from_slice(json).map_err(|_| "The runtime returned unreadable device data")?;
    let mut devices = Vec::new();
    if let Some(entries) = value.get("devices").and_then(|value| value.as_array()) {
        for entry in entries {
            let kind = entry
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let id = if kind == "cpu" {
                "cpu".to_string()
            } else if kind.ends_with("gpu") {
                match entry.get("index").and_then(|value| value.as_u64()) {
                    Some(index) => format!("vulkan:{index}"),
                    None => continue,
                }
            } else {
                continue;
            };
            let description = entry
                .get("description")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .trim();
            let memory = entry.get("memory_total").and_then(|value| value.as_u64());
            let label = match (description.is_empty(), memory) {
                (false, Some(memory)) if id != "cpu" => {
                    format!("{description} · {:.0} GB", memory as f64 / 1_000_000_000.0)
                }
                (false, _) => description.to_string(),
                (true, _) if id == "cpu" => "CPU".to_string(),
                (true, _) => id.clone(),
            };
            devices.push(Device { id, label });
        }
    }
    if !devices.iter().any(|device| device.id == "cpu") {
        devices.push(Device {
            id: "cpu".to_string(),
            label: "CPU".to_string(),
        });
    }
    Ok(devices)
}

fn resolve_cli(runtime_path: &str) -> Result<PathBuf, String> {
    let path = Path::new(runtime_path);
    let mut candidates = Vec::new();
    if path.is_dir() {
        candidates.push(path.join("bin").join("nemo-speech.exe"));
        candidates.push(path.join("nemo-speech.exe"));
    } else if let Some(parent) = path.parent() {
        candidates.push(parent.join("nemo-speech.exe"));
        if let Some(root) = parent.parent() {
            candidates.push(root.join("bin").join("nemo-speech.exe"));
            candidates.push(root.join("nemo-speech.exe"));
        }
    }
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| "The NeMo-Speech CLI was not found next to the runtime".to_string())
}

fn resolve_library(runtime_path: &str) -> Result<PathBuf, String> {
    let path = Path::new(runtime_path);
    if path.is_dir() {
        for candidate in [
            path.join("bin").join("nemo_speech_asr_c.dll"),
            path.join("nemo_speech_asr_c.dll"),
        ] {
            if candidate.is_file() {
                return absolute(candidate);
            }
        }
        return Err(format!(
            "nemo_speech_asr_c.dll was not found in {}",
            path.display()
        ));
    }
    if path.is_file() {
        return absolute(path.to_path_buf());
    }
    Err(format!(
        "The NeMo-Speech runtime path does not exist: {}",
        path.display()
    ))
}

fn absolute(path: PathBuf) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path);
    }
    std::env::current_dir()
        .map(|directory| directory.join(path))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_wav_pcm16(bytes: &[u8]) -> (Vec<f32>, u32) {
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        let mut format = None;
        let mut channels = 0;
        let mut rate = 0;
        let mut bits = 0;
        let mut data: &[u8] = &[];
        let mut offset = 12;
        while offset + 8 <= bytes.len() {
            let id = &bytes[offset..offset + 4];
            let size =
                u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap()) as usize;
            let body = &bytes[offset + 8..(offset + 8 + size).min(bytes.len())];
            if id == b"fmt " {
                format = Some(u16::from_le_bytes(body[0..2].try_into().unwrap()));
                channels = u16::from_le_bytes(body[2..4].try_into().unwrap());
                rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                bits = u16::from_le_bytes(body[14..16].try_into().unwrap());
            } else if id == b"data" {
                data = body;
            }
            offset += 8 + size + (size & 1);
        }
        assert_eq!(format, Some(1));
        assert_eq!(channels, 1);
        assert_eq!(bits, 16);
        (
            data.chunks_exact(2)
                .map(|sample| i16::from_le_bytes([sample[0], sample[1]]) as f32 / 32768.0)
                .collect(),
            rate,
        )
    }

    #[test]
    fn parakeet_is_offline_only() {
        let path = "parakeet-tdt-0.6b-v3.q8_0.gguf";
        assert!(!supports_streaming(path));
        assert!(supports_streaming("nemotron-3.5-asr-streaming-0.6b.q8_0.gguf"));
        assert!(supports_streaming(
            "nemotron-speech-streaming-en-0.6b.q8_0.gguf"
        ));
    }

    #[test]
    fn word_boosting_needs_a_nemotron_model_with_the_embedded_tokenizer() {
        let directory = std::env::temp_dir().join("voxen-boost-tokenizer-test");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let tokenizer = directory.join("nemotron-3.5-asr-streaming-0.6b.q8_0.gguf");
        std::fs::write(&tokenizer, b"GGUF asr.tokenizer.spm_model").unwrap();
        assert!(supports_boost(tokenizer.to_str().unwrap()));
        let legacy = directory.join("nemotron-speech-streaming-en-0.6b.q8_0.gguf");
        std::fs::write(&legacy, b"GGUF without a tokenizer").unwrap();
        assert!(!supports_boost(legacy.to_str().unwrap()));
        assert!(!supports_boost("parakeet-tdt-0.6b-v3.q8_0.gguf"));
        assert!(uses_language_hint(
            "nemotron-speech-streaming-en-0.6b.q8_0.gguf"
        ));
        assert!(!uses_language_hint("parakeet-tdt-0.6b-v3.q8_0.gguf"));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn language_hints_use_supported_locales() {
        assert_eq!(language_code("en-US"), Some("en-US"));
        assert_eq!(language_code("auto"), Some("auto"));
        assert!(language_code("en").is_none());
        assert!(parakeet_supports("es-ES"));
        assert!(!parakeet_supports("ja-JP"));
        assert!(english_supports("en-GB"));
        assert!(!english_supports("de-DE"));
        assert!(!nemotron_supports("el-GR"));
        assert!(nemotron_supports("de-DE"));
    }

    #[test]
    fn doctor_devices_map_to_runtime_identifiers() {
        let devices = devices_from_doctor(
            br#"{
                "devices": [
                    {"index": 0, "type": "gpu", "description": "AMD Radeon RX 9060 XT", "memory_total": 17095983104},
                    {"index": 2, "type": "cpu", "description": "Ryzen 5 9600X"}
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(devices[0].id, "vulkan:0");
        assert!(devices[0].label.contains("17 GB"));
        assert_eq!(devices[1].id, "cpu");
        assert_eq!(devices[1].label, "Ryzen 5 9600X");
    }

    #[test]
    fn doctor_devices_always_include_cpu() {
        let devices = devices_from_doctor(br#"{"devices":[{"index":0,"type":"gpu","description":"GPU"}]}"#)
            .unwrap();
        assert!(devices.iter().any(|device| device.id == "cpu"));
    }

    #[test]
    #[ignore]
    fn real_runtime_transcribes_sample() {
        let runtime = std::env::var("VOXEN_RUNTIME").expect("VOXEN_RUNTIME is not set");
        let model = std::env::var("VOXEN_MODEL").expect("VOXEN_MODEL is not set");
        let wav = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("runtime")
            .join("jfk.wav");
        let bytes =
            std::fs::read(&wav).unwrap_or_else(|e| panic!("Failed to read {}: {e}", wav.display()));
        let (samples, rate) = read_wav_pcm16(&bytes);
        let mut recognizer =
            Recognizer::load(&runtime, &model, "vulkan:0", "auto").expect("Failed to load the recognizer");
        let terms = vec!["Voxen".to_string()];
        let transcript = recognizer
            .transcribe(&samples, rate, &terms)
            .expect("Failed to transcribe the sample");
        assert!(
            transcript.to_lowercase().contains("country"),
            "Unexpected transcript: {transcript}"
        );
        assert!(
            transcript.starts_with("And"),
            "Capitalization missing: {transcript}"
        );
        assert!(
            transcript.contains('.'),
            "Punctuation missing: {transcript}"
        );
        assert_eq!(recognizer.transcribe(&samples, rate, &[]).unwrap(), transcript);
        drop(recognizer);
        let mut reloaded = Recognizer::load(&runtime, &model, "vulkan:0", "auto").unwrap();
        assert_eq!(reloaded.transcribe(&samples, rate, &[]).unwrap(), transcript);
    }

    #[test]
    #[ignore]
    fn real_runtime_streams_sample_incrementally() {
        let runtime = std::env::var("VOXEN_RUNTIME").expect("VOXEN_RUNTIME is not set");
        let model = std::env::var("VOXEN_MODEL").expect("VOXEN_MODEL is not set");
        let wav = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("runtime")
            .join("jfk.wav");
        let bytes =
            std::fs::read(&wav).unwrap_or_else(|e| panic!("Failed to read {}: {e}", wav.display()));
        let (samples, rate) = read_wav_pcm16(&bytes);
        let recognizer =
            Recognizer::load(&runtime, &model, "vulkan:0", "auto").expect("Failed to load the recognizer");
        let terms = vec!["Voxen".to_string()];
        let mut stream = recognizer
            .stream(&terms)
            .expect("Failed to start a streaming recognizer");
        let step = (rate as usize / 10).max(1);
        let mut interims = 0;
        for chunk in samples.chunks(step) {
            stream.push(chunk, rate).expect("Failed to push audio");
            while let Some((_, is_final)) = stream.next().expect("Failed to pull a result") {
                if !is_final {
                    interims += 1;
                }
            }
        }
        stream.finish().expect("Failed to flush the stream");
        let mut finals = Vec::new();
        while let Some((transcript, is_final)) = stream.next().expect("Failed to flush the stream")
        {
            if is_final {
                finals.push(transcript);
            }
        }
        let transcript = finals.join(" ");
        assert!(
            interims > 0,
            "No interim results were produced before the final transcript"
        );
        assert!(
            transcript.to_lowercase().contains("country"),
            "Unexpected transcript: {transcript}"
        );
    }
}
