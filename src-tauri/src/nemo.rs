use libloading::os::windows::{
    Library, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
};
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
    _library: Arc<Library>,
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
            _library: Arc::new(library),
        })
    }

    fn options(&self) -> Options {
        let mut options = unsafe { (self.api.options_default)() };
        options.size = std::mem::size_of::<Options>();
        options.enable_automatic_punctuation = true;
        options
    }

    pub fn transcribe(&mut self, samples: &[f32], sample_rate: u32) -> Result<String, String> {
        if samples.is_empty() {
            return Err("No audio was captured".into());
        }
        let options = self.options();
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

    pub fn stream(&self) -> Result<Stream, String> {
        let mut options = self.options();
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
            Recognizer::load(&runtime, &model, "vulkan:0").expect("Failed to load the recognizer");
        let mut stream = recognizer
            .stream()
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
