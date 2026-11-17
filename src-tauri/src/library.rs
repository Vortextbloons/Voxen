use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

pub const RUNTIME_VERSION: &str = "0.2.0";
pub const RUNTIME_DIR: &str = "nemo-speech-0.2.0-windows-x86_64-vulkan";
pub const RUNTIME_URL: &str = "https://github.com/NVIDIA/NeMo-Speech.cpp/releases/download/v0.2.0/nemo-speech-0.2.0-windows-x86_64-vulkan.zip";
pub const RUNTIME_SHA256: &str = "edf15a04ba98740aa0ab75ae4681ac7ad0a488b22b35ce2cbebd10590bb1c1e1";
pub const RUNTIME_BYTES: u64 = 19_302_464;
pub const RUNTIME_LICENSE: &str = "Apache-2.0";
pub const RUNTIME_LICENSE_URL: &str = "https://github.com/NVIDIA/NeMo-Speech.cpp/blob/v0.2.0/LICENSE";

const USER_AGENT: &str = "Voxen/0.1";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelId {
    Multilingual,
    English,
    Parakeet,
}

pub struct ModelSpec {
    pub id: ModelId,
    pub label: &'static str,
    pub detail: &'static str,
    pub filename: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
    pub license: &'static str,
    pub license_url: &'static str,
    pub streaming: bool,
    pub boosting: bool,
}

pub const MODELS: &[ModelSpec] = &[
    ModelSpec {
        id: ModelId::Multilingual,
        label: "Nemotron 3.5 multilingual",
        detail: "32 production locales · live streaming",
        filename: "nemotron-3.5-asr-streaming-0.6b.q8_0.gguf",
        url: "https://huggingface.co/nvidia/nemotron-3.5-asr-streaming-0.6b/resolve/ea30d66debe3740a08b573244286791d423d6b3e/nemotron-3.5-asr-streaming-0.6b.q8_0.gguf",
        sha256: "3fc991d3badad7277c11030a7519832cddaf2057aafed6d4b25147e953a070b1",
        bytes: 742_090_464,
        license: "NVIDIA Open Model License (OpenMDW 1.1)",
        license_url: "https://huggingface.co/nvidia/nemotron-3.5-asr-streaming-0.6b",
        streaming: true,
        boosting: true,
    },
    ModelSpec {
        id: ModelId::English,
        label: "Nemotron English",
        detail: "English only · live streaming",
        filename: "nemotron-speech-streaming-en-0.6b.q8_0.gguf",
        url: "https://huggingface.co/nvidia/nemotron-speech-streaming-en-0.6b/resolve/ebe59e5a817142986528bbbee5dba8db7b38ed50/nemotron-speech-streaming-en-0.6b.q8_0.gguf",
        sha256: "d9a01898d2a611c8764e23a1c2f45e70bbd5a425dc4de93692ac951dd603812d",
        bytes: 699_872_960,
        license: "NVIDIA Open Model License",
        license_url: "https://huggingface.co/nvidia/nemotron-speech-streaming-en-0.6b",
        streaming: true,
        boosting: false,
    },
    ModelSpec {
        id: ModelId::Parakeet,
        label: "Parakeet TDT v3",
        detail: "25 European languages · offline only, no live typing",
        filename: "parakeet-tdt-0.6b-v3.q8_0.gguf",
        url: "https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3/resolve/541d1f99c6b0c3cd0b11a95167540bb8edefd82b/parakeet-tdt-0.6b-v3.q8_0.gguf",
        sha256: "e3880d0aaaaf2c308ea2c35016b2b895c423eb3fda924c1b463d1c19b7f4d32e",
        bytes: 713_975_456,
        license: "CC-BY-4.0",
        license_url: "https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3",
        streaming: false,
        boosting: false,
    },
];

pub fn model_spec(id: ModelId) -> &'static ModelSpec {
    MODELS
        .iter()
        .find(|model| model.id == id)
        .expect("model catalog entry")
}

pub fn model_from_path(path: &str) -> Option<ModelId> {
    let name = Path::new(path).file_name()?.to_str()?.to_lowercase();
    if name.contains("parakeet-tdt") {
        Some(ModelId::Parakeet)
    } else if name.contains("nemotron-3.5") {
        Some(ModelId::Multilingual)
    } else if name.contains("nemotron-speech-streaming-en") {
        Some(ModelId::English)
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Runtime,
    Model(ModelId),
}

pub struct Artifact {
    pub path: PathBuf,
    pub ready: bool,
    pub progress: u8,
    pub phase: Option<String>,
    pub error: Option<String>,
}

impl Artifact {
    fn new(path: PathBuf, ready: bool) -> Self {
        Self {
            path,
            ready,
            progress: 0,
            phase: None,
            error: None,
        }
    }

    pub fn busy(&self) -> bool {
        self.phase.is_some()
    }
}

pub struct Library {
    root: PathBuf,
    pub runtime: Artifact,
    pub models: Vec<Artifact>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
}

enum Event {
    Progress(Kind, u8, String),
    Done(Kind, Result<(), String>),
}

impl Library {
    pub fn new() -> Self {
        let root = rune::dirs::AppDirs::new(crate::APP_ID)
            .map(|dirs| dirs.cache().to_path_buf())
            .unwrap_or_else(|| std::env::temp_dir().join(APP_ID_FALLBACK));
        let runtime = root.join("runtime").join(RUNTIME_DIR);
        let models = MODELS
            .iter()
            .map(|spec| {
                let path = root.join("models").join(spec.filename);
                Artifact::new(path.clone(), artifact_ready(Kind::Model(spec.id), &path))
            })
            .collect();
        let (tx, rx) = mpsc::channel();
        Self {
            root,
            runtime: Artifact::new(
                runtime.clone(),
                artifact_ready(Kind::Runtime, &runtime),
            ),
            models,
            tx,
            rx,
        }
    }

    pub fn model(&self, id: ModelId) -> &Artifact {
        self.models
            .iter()
            .find(|artifact| {
                artifact.path.file_name().and_then(|name| name.to_str())
                    == Some(model_spec(id).filename)
            })
            .expect("model artifact")
    }

    pub fn install(&mut self, kind: Kind) {
        let phase = match kind {
            Kind::Runtime => "Downloading runtime…".to_string(),
            Kind::Model(id) => format!("Downloading {}…", model_spec(id).label),
        };
        let artifact = self.artifact_mut(kind);
        if artifact.busy() || artifact.ready {
            return;
        }
        artifact.progress = 0;
        artifact.phase = Some(phase);
        artifact.error = None;
        let root = self.root.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = match kind {
                Kind::Runtime => install_runtime(&root, &tx),
                Kind::Model(id) => install_model(&root, model_spec(id), &tx),
            };
            let _ = tx.send(Event::Done(kind, result));
        });
    }

    pub fn poll(&mut self) -> (bool, Option<Kind>) {
        let mut changed = false;
        let mut completed = None;
        while let Ok(event) = self.rx.try_recv() {
            changed = true;
            match event {
                Event::Progress(kind, progress, phase) => {
                    let artifact = self.artifact_mut(kind);
                    artifact.progress = progress;
                    artifact.phase = Some(phase);
                }
                Event::Done(kind, result) => {
                    let artifact = self.artifact_mut(kind);
                    artifact.phase = None;
                    match result {
                        Ok(()) => {
                            artifact.ready = artifact_ready(kind, &artifact.path);
                            if artifact.ready {
                                artifact.progress = 100;
                                artifact.error = None;
                                completed = Some(kind);
                            } else {
                                artifact.progress = 0;
                                artifact.error =
                                    Some("The installed files did not verify".into());
                            }
                        }
                        Err(error) => {
                            artifact.progress = 0;
                            artifact.error = Some(error);
                        }
                    }
                }
            }
        }
        (changed, completed)
    }

    pub fn mark_missing(&mut self, kind: Kind) {
        let artifact = self.artifact_mut(kind);
        artifact.ready = false;
        artifact.progress = 0;
        artifact.phase = None;
        artifact.error = None;
    }

    fn artifact_mut(&mut self, kind: Kind) -> &mut Artifact {
        match kind {
            Kind::Runtime => &mut self.runtime,
            Kind::Model(id) => {
                let filename = model_spec(id).filename;
                self.models
                    .iter_mut()
                    .find(|artifact| {
                        artifact.path.file_name().and_then(|name| name.to_str()) == Some(filename)
                    })
                    .expect("model artifact")
            }
        }
    }
}

impl Default for Library {
    fn default() -> Self {
        Self::new()
    }
}

const APP_ID_FALLBACK: &str = "com.voxen.app";

fn artifact_ready(kind: Kind, path: &Path) -> bool {
    match kind {
        Kind::Runtime => {
            path.join("bin").join("nemo_speech_asr_c.dll").is_file()
                && path.join("bin").join("nemo-speech.exe").is_file()
        }
        Kind::Model(id) => path
            .metadata()
            .is_ok_and(|metadata| metadata.len() == model_spec(id).bytes),
    }
}

fn install_runtime(root: &Path, tx: &Sender<Event>) -> Result<(), String> {
    let archive = root.join("runtime.zip");
    let staging = root.join("runtime-staging");
    let directory = root.join("runtime");
    let target = directory.join(RUNTIME_DIR);
    if staging.exists() {
        std::fs::remove_dir_all(&staging)
            .map_err(|error| format!("Could not clear {}: {error}", staging.display()))?;
    }
    std::fs::create_dir_all(&staging)
        .map_err(|error| format!("Could not create {}: {error}", staging.display()))?;
    download(
        RUNTIME_URL,
        &archive,
        RUNTIME_SHA256,
        RUNTIME_BYTES,
        Kind::Runtime,
        "Downloading runtime…",
        tx,
    )?;
    let _ = tx.send(Event::Progress(Kind::Runtime, 90, "Extracting runtime…".into()));
    extract(&archive, &staging)?;
    let _ = std::fs::remove_file(&archive);
    let extracted = staging.join(RUNTIME_DIR);
    if !extracted.join("bin").join("nemo_speech_asr_c.dll").is_file()
        || !extracted.join("bin").join("nemo-speech.exe").is_file()
    {
        return Err("The runtime archive did not contain the NeMo-Speech runtime".into());
    }
    if target.exists() {
        std::fs::remove_dir_all(&target)
            .map_err(|error| format!("Could not replace the runtime: {error}"))?;
    }
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create {}: {error}", directory.display()))?;
    std::fs::rename(&extracted, &target)
        .map_err(|error| format!("Could not install the runtime: {error}"))?;
    let _ = std::fs::remove_dir_all(&staging);
    Ok(())
}

fn install_model(root: &Path, spec: &ModelSpec, tx: &Sender<Event>) -> Result<(), String> {
    let directory = root.join("models");
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create {}: {error}", directory.display()))?;
    let model = directory.join(spec.filename);
    download(
        spec.url,
        &model,
        spec.sha256,
        spec.bytes,
        Kind::Model(spec.id),
        &format!("Downloading {}…", spec.label),
        tx,
    )
}

fn download(
    url: &str,
    destination: &Path,
    sha256: &str,
    expected: u64,
    kind: Kind,
    phase: &str,
    tx: &Sender<Event>,
) -> Result<(), String> {
    let response = ureq::get(url)
        .set("User-Agent", USER_AGENT)
        .timeout(Duration::from_secs(1800))
        .call()
        .map_err(http_error)?;
    let total = response
        .header("content-length")
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(expected);
    let part = destination.with_file_name(format!(
        "{}.part",
        destination
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("download")
    ));
    let mut reader = response.into_reader();
    let mut buffer = [0u8; 64 * 1024];
    let mut downloaded = 0u64;
    let mut reported = 0u8;
    let mut hasher = Sha256::new();
    {
        let mut file = std::fs::File::create(&part)
            .map_err(|error| format!("Could not write {}: {error}", part.display()))?;
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| format!("Download failed: {error}"))?;
            if read == 0 {
                break;
            }
            file.write_all(&buffer[..read])
                .map_err(|error| format!("Could not write {}: {error}", part.display()))?;
            hasher.update(&buffer[..read]);
            downloaded += read as u64;
            let progress = (downloaded * 88 / total.max(1)).min(88) as u8;
            if progress != reported {
                reported = progress;
                let _ = tx.send(Event::Progress(kind, progress, phase.to_string()));
            }
        }
    }
    if downloaded != total {
        let _ = std::fs::remove_file(&part);
        return Err("The download is incomplete".into());
    }
    let digest = format!("{:x}", hasher.finalize());
    if !digest.eq_ignore_ascii_case(sha256) {
        let _ = std::fs::remove_file(&part);
        return Err("The download failed its integrity check".into());
    }
    if destination.exists() {
        std::fs::remove_file(destination)
            .map_err(|error| format!("Could not replace {}: {error}", destination.display()))?;
    }
    std::fs::rename(&part, destination)
        .map_err(|error| format!("Could not finish {}: {error}", destination.display()))?;
    let _ = tx.send(Event::Progress(kind, 100, "Installed".into()));
    Ok(())
}

fn extract(archive: &Path, destination: &Path) -> Result<(), String> {
    let script = format!(
        "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
        quote(archive),
        quote(destination)
    );
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| format!("Could not start the extractor: {error}"))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        let message = message.trim();
        return Err(if message.is_empty() {
            "The runtime archive could not be extracted".into()
        } else {
            format!("The runtime archive could not be extracted: {message}")
        });
    }
    Ok(())
}

fn quote(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

fn http_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, _) => format!("The download server returned HTTP {code}"),
        ureq::Error::Transport(error) => format!("Could not reach the download server: {error}"),
    }
}

pub fn size_label(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else {
        format!("{:.0} MB", bytes as f64 / 1_000_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_runtime_needs_the_library_and_cli() {
        let directory = std::env::temp_dir().join("voxen-runtime-layout-test");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(directory.join("bin")).unwrap();
        std::fs::write(directory.join("bin").join("nemo_speech_asr_c.dll"), b"stub").unwrap();
        assert!(!artifact_ready(Kind::Runtime, &directory));
        std::fs::write(directory.join("bin").join("nemo-speech.exe"), b"stub").unwrap();
        assert!(artifact_ready(Kind::Runtime, &directory));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn model_catalog_names_are_recognized() {
        assert_eq!(
            model_from_path("C:\\models\\nemotron-3.5-asr-streaming-0.6b.q8_0.gguf"),
            Some(ModelId::Multilingual)
        );
        assert_eq!(
            model_from_path("parakeet-tdt-0.6b-v3.q8_0.gguf"),
            Some(ModelId::Parakeet)
        );
        assert_eq!(
            model_from_path("parakeet-tdt-0.6b-v3.oss-align.q8_0.gguf"),
            Some(ModelId::Parakeet)
        );
        assert_eq!(model_from_path("custom.gguf"), None);
    }

    #[test]
    fn catalog_marks_only_the_tokenizer_equipped_model_as_boosting() {
        assert!(model_spec(ModelId::Multilingual).boosting);
        assert!(!model_spec(ModelId::English).boosting);
        assert!(!model_spec(ModelId::Parakeet).boosting);
    }
}
