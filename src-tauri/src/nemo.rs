use libloading::os::windows::{
    Library, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
};
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};

type Handle = *mut c_void;

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

struct Api {
    create: unsafe extern "C" fn(*const RecognizerConfig, *mut Handle) -> i32,
    destroy: unsafe extern "C" fn(Handle),
    recognize:
        unsafe extern "C" fn(Handle, *const Options, *const f32, usize, i32, *mut Handle) -> i32,
    result_destroy: unsafe extern "C" fn(Handle),
    result_alternative_count: unsafe extern "C" fn(*const c_void) -> usize,
    result_transcript: unsafe extern "C" fn(*const c_void, usize) -> *const c_char,
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
            result_destroy,
            result_alternative_count,
            result_transcript,
            options_default,
            last_error,
        })
    }
}

pub struct Recognizer {
    handle: Handle,
    api: Api,
    _library: Library,
}

impl Recognizer {
    pub fn load(runtime_path: &str, model_path: &str, device: &str) -> Result<Self, String> {
        let gpu = parse_device(device)?;
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
            _library: library,
        })
    }

    pub fn transcribe(&mut self, samples: &[f32], sample_rate: u32) -> Result<String, String> {
        if samples.is_empty() {
            return Err("No audio was captured".into());
        }
        let mut options = unsafe { (self.api.options_default)() };
        options.size = std::mem::size_of::<Options>();
        options.enable_automatic_punctuation = true;
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
            let pointer = unsafe { (self.api.result_transcript)(result, 0) };
            Ok(if pointer.is_null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(pointer) }
                    .to_string_lossy()
                    .into_owned()
            })
        };
        unsafe { (self.api.result_destroy)(result) };
        transcript
    }
}

impl Drop for Recognizer {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { (self.api.destroy)(self.handle) };
        }
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
            Recognizer::load(&runtime, &model, "vulkan:0").expect("Failed to load the recognizer");
        let transcript = recognizer
            .transcribe(&samples, rate)
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
            transcript.ends_with('.'),
            "Punctuation missing: {transcript}"
        );
        assert_eq!(recognizer.transcribe(&samples, rate).unwrap(), transcript);
        drop(recognizer);
        let mut reloaded = Recognizer::load(&runtime, &model, "vulkan:0").unwrap();
        assert_eq!(reloaded.transcribe(&samples, rate).unwrap(), transcript);
    }
}
