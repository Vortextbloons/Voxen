use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use serde::Deserialize;

const RELEASE_URL: &str = "https://api.github.com/repos/Vortextbloons/Voxen/releases/latest";
const USER_AGENT: &str = "Voxen-Updater";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    Idle,
    Checking,
    Current,
    Available,
    Installing,
    Ready,
    Error,
}

#[derive(Clone, Debug)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
}

pub struct Updater {
    tx: Sender<Event>,
    rx: Receiver<Event>,
    status: UpdateStatus,
    info: Option<UpdateInfo>,
    progress: u8,
    error: Option<String>,
    asset: Option<String>,
}

enum Event {
    Checked(Result<Option<Found>, String>),
    Progress(u8),
    Installed(Result<(), String>),
}

struct Found {
    info: UpdateInfo,
    asset: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    body: Option<String>,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

impl Updater {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            status: UpdateStatus::Idle,
            info: None,
            progress: 0,
            error: None,
            asset: None,
        }
    }

    pub fn current_version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    pub fn status(&self) -> &UpdateStatus {
        &self.status
    }

    pub fn info(&self) -> Option<&UpdateInfo> {
        self.info.as_ref()
    }

    pub fn progress(&self) -> u8 {
        self.progress
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn check(&mut self) {
        if matches!(
            self.status,
            UpdateStatus::Checking | UpdateStatus::Installing
        ) {
            return;
        }
        self.status = UpdateStatus::Checking;
        self.info = None;
        self.progress = 0;
        self.error = None;
        self.asset = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Event::Checked(fetch()));
        });
    }

    pub fn install(&mut self) {
        if self.status != UpdateStatus::Available {
            return;
        }
        let (Some(info), Some(asset)) = (self.info.as_ref(), self.asset.as_ref()) else {
            return;
        };
        let version = info.version.clone();
        let asset = asset.clone();
        self.status = UpdateStatus::Installing;
        self.progress = 0;
        self.error = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = download(&version, &asset, &tx).and_then(|path| launch_helper(&path));
            let _ = tx.send(Event::Installed(result));
        });
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Checked(Ok(Some(found))) => {
                    self.status = UpdateStatus::Available;
                    self.info = Some(found.info);
                    self.asset = Some(found.asset);
                }
                Event::Checked(Ok(None)) => {
                    self.status = UpdateStatus::Current;
                }
                Event::Checked(Err(error)) => {
                    self.status = UpdateStatus::Error;
                    self.error = Some(error);
                }
                Event::Progress(progress) => {
                    if self.progress == progress {
                        continue;
                    }
                    self.progress = progress;
                }
                Event::Installed(Ok(())) => {
                    self.progress = 100;
                    self.status = UpdateStatus::Ready;
                }
                Event::Installed(Err(error)) => {
                    self.status = UpdateStatus::Error;
                    self.error = Some(error);
                }
            }
            changed = true;
        }
        changed
    }
}

impl Default for Updater {
    fn default() -> Self {
        Self::new()
    }
}

fn fetch() -> Result<Option<Found>, String> {
    let response = ureq::get(RELEASE_URL)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(30))
        .call()
        .map_err(http_error)?;
    let body = response
        .into_string()
        .map_err(|error| format!("Could not read the GitHub response: {error}"))?;
    let release: Release = serde_json::from_str(&body)
        .map_err(|error| format!("GitHub sent an unexpected release: {error}"))?;
    let tag = release.tag_name.trim();
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    if !is_newer(&version, Updater::current_version()) {
        return Ok(None);
    }
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name.eq_ignore_ascii_case("Voxen.exe"))
        .or_else(|| {
            release
                .assets
                .iter()
                .find(|asset| asset.name.to_ascii_lowercase().ends_with(".exe"))
        });
    let Some(asset) = asset else {
        return Err(format!("Release {version} has no Voxen.exe asset"));
    };
    let info = UpdateInfo {
        version,
        notes: release.body.unwrap_or_default(),
    };
    Ok(Some(Found {
        info,
        asset: asset.browser_download_url.clone(),
    }))
}

fn numeric_parts(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|part| {
            part.chars()
                .take_while(|character| character.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        })
        .collect()
}

fn is_newer(candidate: &str, current: &str) -> bool {
    let candidate = numeric_parts(candidate);
    let current = numeric_parts(current);
    for index in 0..candidate.len().max(current.len()) {
        let left = candidate.get(index).copied().unwrap_or(0);
        let right = current.get(index).copied().unwrap_or(0);
        if left != right {
            return left > right;
        }
    }
    false
}

fn http_error(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, _) => format!("GitHub returned HTTP {code}"),
        ureq::Error::Transport(error) => format!("Could not reach GitHub: {error}"),
    }
}

fn download(version: &str, url: &str, tx: &Sender<Event>) -> Result<PathBuf, String> {
    let directory = std::env::temp_dir().join("voxen-update");
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create {}: {error}", directory.display()))?;
    let path = directory.join(format!("Voxen-{version}.exe"));
    let response = ureq::get(url)
        .set("User-Agent", USER_AGENT)
        .timeout(Duration::from_secs(300))
        .call()
        .map_err(http_error)?;
    let total = response
        .header("content-length")
        .and_then(|value| value.parse::<u64>().ok());
    let mut reader = response.into_reader();
    let mut buffer = [0u8; 64 * 1024];
    let mut downloaded = 0u64;
    let mut reported = 0u8;
    {
        let mut file = std::fs::File::create(&path)
            .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| format!("Download failed: {error}"))?;
            if read == 0 {
                break;
            }
            file.write_all(&buffer[..read])
                .map_err(|error| format!("Could not write {}: {error}", path.display()))?;
            downloaded += read as u64;
            if let Some(total) = total.filter(|total| *total > 0) {
                let progress = (downloaded * 100 / total).min(100) as u8;
                if progress != reported {
                    reported = progress;
                    let _ = tx.send(Event::Progress(progress));
                }
            }
        }
    }
    if let Some(total) = total.filter(|total| *total > 0) {
        if total != downloaded {
            let _ = std::fs::remove_file(&path);
            return Err("The update download is incomplete".into());
        }
    }
    let mut magic = [0u8; 2];
    let readable = std::fs::File::open(&path).and_then(|mut file| file.read_exact(&mut magic));
    if readable.is_err() || &magic != b"MZ" {
        let _ = std::fs::remove_file(&path);
        return Err("The update download is not a Windows executable".into());
    }
    if total.is_some_and(|total| downloaded != total) {
        let _ = std::fs::remove_file(&path);
        return Err("The update download was incomplete".into());
    }
    Ok(path)
}

fn launch_helper(update: &Path) -> Result<(), String> {
    let current = std::env::current_exe()
        .map_err(|error| format!("Could not locate the running Voxen.exe: {error}"))?;
    let batch = update.with_file_name("voxen-update.bat");
    std::fs::write(&batch, helper_script(&current, update))
        .map_err(|error| format!("Could not write {}: {error}", batch.display()))?;
    Command::new("cmd")
        .arg("/C")
        .arg(&batch)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| format!("Could not start the update helper: {error}"))?;
    Ok(())
}

fn helper_script(target: &Path, source: &Path) -> String {
    let pid = std::process::id();
    let mut script = String::new();
    script.push_str("@echo off\r\n");
    script.push_str(&format!("set \"target={}\"\r\n", target.display()));
    script.push_str(&format!("set \"source={}\"\r\n", source.display()));
    script.push_str(":wait\r\n");
    script.push_str(&format!(
        "tasklist /FI \"PID eq {pid}\" | find \"{pid}\" >nul\r\n"
    ));
    script.push_str("if errorlevel 1 goto ready\r\n");
    script.push_str("ping -n 2 127.0.0.1 >nul\r\n");
    script.push_str("goto wait\r\n");
    script.push_str(":ready\r\n");
    script.push_str("move /y \"%source%\" \"%target%\" >nul\r\n");
    script.push_str("start \"\" \"%target%\"\r\n");
    script.push_str("del \"%~f0\"\r\n");
    script
}
