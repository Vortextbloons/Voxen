use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Raw,
    Light,
    Polished,
}

const MAX_INPUT_CHARS: usize = 2000;
const SERVER_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 18231;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
const MIN_OUTPUT_TOKENS: usize = 48;
const MAX_OUTPUT_TOKENS: usize = 256;

const CLEAR_FILLERS: &[&str] = &["um", "umm", "uh", "uhh"];

const EDGE_FILLERS: &[&str] = &["er", "erm", "ah", "hmm", "mm", "mmm", "basically"];

const CUES: &[&[&str]] = &[
    &["no", "wait"],
    &["wait", "no"],
    &["no", "sorry"],
    &["i", "mean"],
    &["i", "meant"],
    &["or", "rather"],
    &["scratch", "that"],
    &["make", "that"],
];

const ARTIFACTS: &[&str] = &[
    "```",
    "<|",
    "return only the cleaned",
    "here is the cleaned",
];

pub struct Cleaner {
    server: PathBuf,
    model: PathBuf,
    port: u16,
    child: Option<Child>,
}

impl Cleaner {
    pub fn load(runtime_path: &str, model_path: &str) -> Result<Self, String> {
        let server = resolve_server(runtime_path)?;
        let model = check_model(model_path)?;
        let port = std::env::var("VOXEN_CLEANUP_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_PORT);
        Ok(Cleaner {
            server,
            model,
            port,
            child: None,
        })
    }

    pub fn matches(&self, runtime_path: &str, model_path: &str) -> bool {
        resolve_server(runtime_path).is_ok_and(|server| server == self.server)
            && check_model(model_path).is_ok_and(|model| model == self.model)
    }

    pub fn unload(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub fn clean(&mut self, text: &str, mode: Mode) -> Result<String, String> {
        if mode == Mode::Raw {
            return Ok(text.to_string());
        }
        let text = text.trim();
        if text.is_empty() {
            return Err("The transcript was empty".into());
        }
        if text.chars().count() > MAX_INPUT_CHARS {
            return Err("The transcript is too long to clean".into());
        }
        self.ensure_started()?;
        let words = text.split_whitespace().count();
        let max_tokens = (words * 2 + 16).clamp(MIN_OUTPUT_TOKENS, MAX_OUTPUT_TOKENS);
        let body = serde_json::json!({
            "messages": [
                {"role": "system", "content": system_prompt(mode)},
                {"role": "user", "content": text},
            ],
            "max_tokens": max_tokens,
            "temperature": 0,
        });
        let raw = http_request(
            &self.address(),
            "POST",
            "/v1/chat/completions",
            Some(body.to_string().as_bytes()),
            REQUEST_TIMEOUT,
        )?;
        let (status, payload) = split_response(&raw)?;
        if status != 200 {
            return Err("The cleanup runtime rejected the transcript".into());
        }
        let response: serde_json::Value =
            serde_json::from_slice(&payload).map_err(|_| "The cleanup runtime gave no text")?;
        if response
            .pointer("/choices/0/finish_reason")
            .and_then(|value| value.as_str())
            == Some("length")
        {
            return Err("The cleanup was cut short".into());
        }
        let candidate = response
            .pointer("/choices/0/message/content")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if candidate.is_empty() {
            return Err("The cleaned transcript was empty".into());
        }
        guard(text, &candidate)?;
        Ok(candidate)
    }

    fn address(&self) -> String {
        format!("{SERVER_HOST}:{}", self.port)
    }

    fn ensure_started(&mut self) -> Result<(), String> {
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(None) => return Ok(()),
                _ => self.unload(),
            }
        }
        let mut command = Command::new(&self.server);
        command
            .arg("-m")
            .arg(&self.model)
            .arg("--host")
            .arg(SERVER_HOST)
            .arg("--port")
            .arg(self.port.to_string())
            .arg("-c")
            .arg("2048")
            .arg("-t")
            .arg(threads().to_string())
            .arg("-rea")
            .arg("off")
            .arg("--no-webui")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        match command.spawn() {
            Ok(child) => self.child = Some(child),
            Err(_) => return Err("The cleanup runtime did not start".into()),
        }
        if self.wait_ready() && self.serves_model() {
            return Ok(());
        }
        self.unload();
        Err("The cleanup runtime did not start".into())
    }

    fn wait_ready(&mut self) -> bool {
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        while Instant::now() < deadline {
            match self.child.as_mut().map(|child| child.try_wait()) {
                Some(Ok(None)) => {}
                _ => return false,
            }
            if let Ok(raw) = http_request(
                &self.address(),
                "GET",
                "/health",
                None,
                Duration::from_secs(2),
            ) {
                let (status, payload) = split_response(&raw).unwrap_or((0, Vec::new()));
                if status == 200 && payload.windows(4).any(|w| w == b"\"ok\"") {
                    return true;
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        false
    }

    fn serves_model(&self) -> bool {
        let wanted = self
            .model
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_lowercase();
        match http_request(
            &self.address(),
            "GET",
            "/v1/models",
            None,
            Duration::from_secs(5),
        ) {
            Ok(raw) => match split_response(&raw) {
                Ok((200, payload)) => String::from_utf8_lossy(&payload)
                    .to_lowercase()
                    .contains(&wanted),
                _ => false,
            },
            Err(_) => false,
        }
    }
}

impl Drop for Cleaner {
    fn drop(&mut self) {
        self.unload();
    }
}

fn system_prompt(mode: Mode) -> &'static str {
    match mode {
        Mode::Raw => "",
        Mode::Light => "You clean up dictated text. Remove only the filler interjections um and uh. When the speaker corrects themselves with a cue like no wait, I mean, or rather, keep only the corrected words that come right after the cue in the same sentence and delete the earlier words plus the cue. Example: Let's meet on Tuesday, no wait, Wednesday at 4 becomes Let's meet on Wednesday at 4. Change nothing else. Preserve the speaker's exact wording, word order, and punctuation. Never add, remove, reorder, or reword information. Keep every conjunction (especially and and or), name, number, unit, date, negation, symbol, path, and command exactly as spoken, including hyphens, slashes, dots, and other symbols inside words. Return only the cleaned text with no commentary, labels, quotes, or formatting marks.",
        Mode::Polished => "You clean up dictated text. Remove only the filler interjections um and uh. When the speaker corrects themselves with a cue like no wait, I mean, or rather, keep only the corrected words that come right after the cue in the same sentence and delete the earlier words plus the cue. Example: Let's meet on Tuesday, no wait, Wednesday at 4 becomes Let's meet on Wednesday at 4. Fix punctuation and capitalization. Split run-on sentences and format a spoken enumeration without conjunctions as a short numbered list only when every listed word is kept and no conjunction is dropped. Keep every word the speaker said: never add, drop, reorder, or reword words. Keep every conjunction (especially and and or), name, number, unit, date, negation, symbol, path, and command exactly as spoken, including hyphens, slashes, dots, and other symbols inside words. Return only the cleaned text with no commentary, labels, quotes, or formatting marks.",
    }
}

pub fn guard(original: &str, candidate: &str) -> Result<(), String> {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return Err("The cleaned transcript was empty".into());
    }
    let lowered = candidate.to_lowercase();
    if ARTIFACTS.iter().any(|mark| lowered.contains(mark)) {
        return Err("The cleaned transcript contained extra text".into());
    }
    let expected = expected_words(original);
    let actual = candidate_words(candidate);
    let mut exact = 0;
    let mut index = 0;
    let mut position = 0;
    while index < expected.len() && position < actual.len() {
        if expected[index] == actual[position].word {
            exact += 1;
            index += 1;
            position += 1;
        } else if is_list_marker(&actual, position) {
            position += 1;
        } else {
            return Err(format!("The cleanup changed \"{}\"", expected[index]));
        }
    }
    while position < actual.len() {
        if !is_list_marker(&actual, position) {
            return Err("The cleanup added words".into());
        }
        position += 1;
    }
    if index < expected.len() {
        return Err(format!("The cleanup dropped \"{}\"", expected[index]));
    }
    if exact == 0 && !expected.is_empty() {
        return Err("The cleanup kept nothing".into());
    }
    Ok(())
}

struct Token {
    word: String,
    sentence: bool,
    clause: bool,
}

fn strip_token(chunk: &str) -> String {
    let start = chunk.trim_start_matches(|value| {
        matches!(value, '"' | '\'' | '(' | '[' | '{' | '<')
    });
    let trimmed = start.trim_end_matches(|value| {
        matches!(
            value,
            '.' | ',' | '!' | '?' | ':' | ';' | '"' | '\'' | ')' | ']' | '}' | '>'
        )
    });
    trimmed.to_lowercase()
}

fn ends_sentence(chunk: &str) -> bool {
    let trimmed = chunk.trim_end_matches(|value| {
        matches!(value, '"' | '\'' | ')' | ']' | '}' | '>')
    });
    matches!(
        trimmed.chars().last(),
        Some('.' | '!' | '?' | ':' | ';')
    )
}

fn ends_clause(chunk: &str) -> bool {
    if ends_sentence(chunk) {
        return true;
    }
    let trimmed = chunk.trim_end_matches(|value| {
        matches!(value, '"' | '\'' | ')' | ']' | '}' | '>')
    });
    matches!(trimmed.chars().last(), Some(','))
}

fn content_words(text: &str) -> Vec<Token> {
    let mut tagged = Vec::new();
    let mut sentence = true;
    let mut clause = true;
    for chunk in text.split_whitespace() {
        if chunk.is_empty() {
            continue;
        }
        let core = strip_token(chunk);
        let sentence_end = ends_sentence(chunk);
        let clause_end = ends_clause(chunk);
        if !core.is_empty() && core.chars().any(|value| value.is_alphanumeric()) {
            tagged.push(Token {
                word: core,
                sentence,
                clause,
            });
        }
        if sentence_end {
            sentence = true;
            clause = true;
        } else {
            sentence = false;
            clause = clause_end;
        }
    }
    tagged
}

fn candidate_words(text: &str) -> Vec<Token> {
    content_words(text)
}

fn stripped_words(text: &str) -> Vec<Token> {
    let tagged = content_words(text);
    let mut kept = Vec::new();
    let mut index = 0;
    while index < tagged.len() {
        let word = &tagged[index].word;
        if CLEAR_FILLERS.contains(&word.as_str()) {
            index += 1;
            continue;
        }
        if EDGE_FILLERS.contains(&word.as_str())
            && tagged[index].clause
            && tagged
                .get(index + 1)
                .is_some_and(|next| next.clause || next.sentence)
        {
            index += 1;
            continue;
        }
        kept.push(Token {
            word: word.clone(),
            sentence: tagged[index].sentence,
            clause: tagged[index].clause,
        });
        index += 1;
    }
    kept
}

fn expected_words(text: &str) -> Vec<String> {
    let tagged = stripped_words(text);
    let mut kept: Vec<String> = Vec::new();
    let mut index = 0;
    while index < tagged.len() {
        if !kept.is_empty() {
            let previous = kept.last().map(String::as_str);
            if let Some(length) = match_cue(&tagged, index, previous) {
                kept.pop();
                index += length;
                continue;
            }
        }
        kept.push(tagged[index].word.clone());
        index += 1;
    }
    kept
}

fn is_negation(word: &str) -> bool {
    matches!(
        word,
        "no" | "not"
            | "never"
            | "none"
            | "nothing"
            | "neither"
            | "nor"
            | "cannot"
    ) || word.ends_with("n't")
}

fn match_cue(tagged: &[Token], index: usize, previous: Option<&str>) -> Option<usize> {
    if tagged.get(index).is_none_or(|token| token.sentence) {
        return None;
    }
    if previous.is_some_and(is_negation) {
        return None;
    }
    CUES.iter()
        .filter(|cue| {
            tagged.len() >= index + cue.len()
                && cue
                    .iter()
                    .enumerate()
                    .all(|(offset, word)| tagged[index + offset].word == **word)
        })
        .filter(|cue| {
            cue.iter().enumerate().all(|(offset, _)| {
                offset == 0 || !tagged[index + offset].sentence
            }) && tagged
                .get(index + cue.len())
                .is_some_and(|next| !next.sentence)
        })
        .map(|cue| cue.len())
        .next()
}

fn is_list_marker(actual: &[Token], position: usize) -> bool {
    let token = &actual[position];
    token.word.len() == 1
        && matches!(
            token.word.as_str(),
            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"
        )
        && actual.get(position + 1).is_some_and(|next| next.sentence)
}

fn http_request(
    address: &str,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + timeout;
    let socket = address
        .to_socket_addrs()
        .map_err(|_| "The cleanup runtime address was invalid")?
        .next()
        .ok_or("The cleanup runtime address was invalid")?;
    let mut stream = TcpStream::connect_timeout(&socket, CONNECT_TIMEOUT.min(timeout))
        .map_err(|_| "The cleanup runtime is unavailable".to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|_| "The cleanup runtime is unavailable".to_string())?;
    let length = body.map(|value| value.len()).unwrap_or(0);
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(head.as_bytes())
        .map_err(|_| "The cleanup runtime is unavailable".to_string())?;
    if let Some(body) = body {
        stream
            .write_all(body)
            .map_err(|_| "The cleanup runtime is unavailable".to_string())?;
    }
    let mut raw = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("Cleaning the transcript took too long".into());
        }
        stream
            .set_read_timeout(Some(remaining.min(Duration::from_secs(5))))
            .map_err(|_| "Cleaning the transcript took too long".to_string())?;
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => raw.extend_from_slice(&chunk[..read]),
            Err(_) => {
                if Instant::now() >= deadline {
                    return Err("Cleaning the transcript took too long".into());
                }
                return Err("The cleanup runtime stopped early".into());
            }
        }
    }
    if raw.is_empty() {
        return Err("The cleanup runtime gave no text".into());
    }
    Ok(raw)
}

fn split_response(raw: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("The cleanup runtime gave no text")?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or("The cleanup runtime gave no text")?;
    Ok((status, raw[split + 4..].to_vec()))
}

fn threads() -> usize {
    std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(4)
        .min(8)
}

fn resolve_server(runtime_path: &str) -> Result<PathBuf, String> {
    let path = Path::new(runtime_path);
    if path.is_dir() {
        for candidate in [
            path.join("llama-server.exe"),
            path.join("llama-server"),
            path.join("bin").join("llama-server.exe"),
            path.join("server.exe"),
        ] {
            if candidate.is_file() {
                return absolute(candidate);
            }
        }
        return Err(format!("llama-server was not found in {}", path.display()));
    }
    if path.is_file() {
        return absolute(path.to_path_buf());
    }
    Err(format!(
        "The cleanup runtime path does not exist: {}",
        path.display()
    ))
}

fn check_model(model_path: &str) -> Result<PathBuf, String> {
    let path = Path::new(model_path);
    if path.is_file() {
        return absolute(path.to_path_buf());
    }
    Err(format!(
        "The cleanup model was not found: {}",
        path.display()
    ))
}

fn absolute(path: PathBuf) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path);
    }
    std::env::current_dir()
        .map(|directory| directory.join(path))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_accepts_filler_removal() {
        assert!(guard(
            "Um, uh, I think we should ship the release soon",
            "I think we should ship the release soon"
        )
        .is_ok());
        assert!(guard("So, we should ship soon", "So, we should ship soon").is_ok());
        assert!(guard("So called Alice", "So called Alice").is_ok());
        assert!(guard("So, we should ship soon", "we should ship soon").is_err());
        assert!(guard("So called Alice", "called Alice").is_err());
        assert!(guard(
            "I think we should ship soon",
            "Um, I think we should ship soon"
        )
        .is_err());
        assert!(guard(
            "we should ship soon",
            "Basically, we should ship soon"
        )
        .is_err());
    }

    #[test]
    fn guard_rejects_dropped_conjunctions() {
        assert!(guard("buy milk or bread", "Buy: 1. milk 2. bread").is_err());
        assert!(guard(
            "buy milk eggs and bread",
            "Buy: 1. milk 2. eggs 3. bread"
        )
        .is_err());
        assert!(guard("buy milk or bread", "buy milk or bread").is_ok());
        assert!(guard(
            "buy milk eggs and bread",
            "buy milk eggs and bread"
        )
        .is_ok());
    }

    #[test]
    fn guard_rejects_symbol_and_number_changes() {
        assert!(guard("set depth to -3 meters", "set depth to 3 meters").is_err());
        assert!(guard("value is 1.5 volts", "value is 1 5 volts").is_err());
        assert!(guard("run /force now", "run force now").is_err());
        assert!(guard(
            "run --dry-run tonight",
            "run dry run tonight"
        )
        .is_err());
        assert!(guard(
            "email alice@example.com tonight",
            "email alice tonight"
        )
        .is_err());
        assert!(guard("value is 1.5 volts", "value is 1.5 volts").is_ok());
        assert!(guard("set depth to -3 meters", "Set depth to -3 meters.").is_ok());
    }

    #[test]
    fn guard_keeps_ambiguous_fillers() {
        assert!(guard("that kind of bird is rare", "that bird is rare").is_err());
        assert!(guard("do you know the answer", "do the answer").is_err());
        assert!(guard("it is sort of blue", "it is blue").is_err());
        assert!(guard("send the notes to Ah", "send the notes to").is_err());
        assert!(guard("ask Er about staging", "ask about staging").is_err());
        assert!(guard("I like apples", "I apples").is_err());
    }

    #[test]
    fn guard_rejects_cross_sentence_and_trailing_corrections() {
        assert!(guard(
            "Meet on Tuesday. No wait, Wednesday at 4",
            "Meet on Wednesday at 4"
        )
        .is_err());
        assert!(guard("Meet Tuesday, no wait", "Meet").is_err());
        assert!(guard(
            "Do not delete the report, I mean keep it",
            "Do keep it"
        )
        .is_err());
    }

    #[test]
    fn guard_accepts_resolved_self_correction() {
        assert!(guard(
            "Let's meet on Tuesday, no wait, Wednesday at 4",
            "Let's meet on Wednesday at 4"
        )
        .is_ok());
        assert!(guard(
            "I'll send the report Friday, I mean Monday",
            "I'll send the report Monday"
        )
        .is_ok());
    }

    #[test]
    fn guard_rejects_added_negation() {
        assert!(guard(
            "I can start with the token",
            "I cannot start with the token"
        )
        .is_err());
        assert!(guard(
            "I can't start without the token",
            "I can start with the token"
        )
        .is_err());
        assert!(guard("Do not delete the report", "Delete the report").is_err());
    }

    #[test]
    fn guard_rejects_changed_numbers() {
        assert!(guard("Send 3 files to John", "Send 30 files to John").is_err());
        assert!(guard("Send 3 files to John", "Send files to John").is_err());
        assert!(guard("Meet at 4 on Tuesday", "Meet at 4 on Tuesday").is_ok());
    }

    #[test]
    fn guard_rejects_swapped_names() {
        assert!(guard("Send the notes to Alice", "Send the notes to Bob").is_err());
        assert!(guard(
            "Deploy Voxen to the staging cluster tonight",
            "Deploy the app to the staging cluster tonight"
        )
        .is_err());
    }

    #[test]
    fn guard_rejects_swapped_days() {
        assert!(guard(
            "We will ship the release tomorrow morning",
            "We will ship the release yesterday morning"
        )
        .is_err());
    }

    #[test]
    fn guard_rejects_truncation_and_addition() {
        assert!(guard(
            "We will ship the release tomorrow morning to everyone",
            "We will ship the release"
        )
        .is_err());
        assert!(guard(
            "We will ship the release tomorrow morning to everyone",
            "We will ship the release tomorrow morning to everyone and then send a summary"
        )
        .is_err());
        assert!(guard("", "").is_err());
        assert!(guard("Hello there friend", "```\nHello there friend\n```").is_err());
    }

    #[test]
    fn guard_rejects_broad_correction_deletion() {
        assert!(guard(
            "I went to the store on Tuesday, no wait, Wednesday",
            "Wednesday"
        )
        .is_err());
    }

    #[test]
    fn guard_accepts_polished_punctuation_and_lists() {
        assert!(guard(
            "the api returns 42 records to Voxen and we never retry",
            "The API returns 42 records to Voxen, and we never retry."
        )
        .is_ok());
        assert!(guard("buy milk eggs bread", "Buy: 1. milk 2. eggs 3. bread").is_ok());
    }

    #[test]
    fn guard_rejects_verb_filler_removal() {
        assert!(guard("I like apples", "I apples").is_err());
    }

    #[test]
    fn load_rejects_missing_runtime_and_model() {
        assert!(Cleaner::load("C:\\no-such-dir-voxen", "C:\\no-such-model.gguf").is_err());
    }

    #[test]
    fn load_accepts_present_files_without_starting_a_server() {
        let directory = std::env::temp_dir().join("voxen-cleanup-test");
        let _ = std::fs::create_dir_all(&directory);
        let server = directory.join("llama-server.exe");
        let model = directory.join("cleanup.gguf");
        std::fs::write(&server, b"stub").unwrap();
        std::fs::write(&model, b"stub").unwrap();
        let cleaner = Cleaner::load(server.to_str().unwrap(), model.to_str().unwrap()).unwrap();
        assert!(cleaner.matches(server.to_str().unwrap(), model.to_str().unwrap()));
        assert!(!cleaner.matches(server.to_str().unwrap(), server.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    #[ignore]
    fn real_runtime_cleans_spoken_text() {
        let runtime =
            std::env::var("VOXEN_CLEANUP_RUNTIME").expect("VOXEN_CLEANUP_RUNTIME is not set");
        let model = std::env::var("VOXEN_CLEANUP_MODEL").expect("VOXEN_CLEANUP_MODEL is not set");
        let mut cleaner =
            Cleaner::load(&runtime, &model).expect("Failed to load the cleanup model");
        let raw = "Um, uh, I think we should deploy Voxen to staging on Tuesday at 4.";
        let light = cleaner
            .clean(raw, Mode::Light)
            .expect("Light cleanup was rejected");
        assert!(
            !light.to_lowercase().contains("um,"),
            "fillers survived: {light}"
        );
        assert!(light.contains("Voxen"), "name was not preserved: {light}");
        assert!(light.contains('4'), "number was not preserved: {light}");
        assert!(guard(raw, &light).is_ok());
        let corrected = cleaner
            .clean(
                "Let's meet on Tuesday, no wait, Wednesday at 4",
                Mode::Light,
            )
            .expect("Correction cleanup was rejected");
        assert!(
            corrected.contains("Wednesday"),
            "correction lost: {corrected}"
        );
        assert!(
            !corrected.contains("Tuesday"),
            "correction kept both days: {corrected}"
        );
        let polished = cleaner
            .clean(
                "the api returns 42 records and we never retry",
                Mode::Polished,
            )
            .expect("Polished cleanup was rejected");
        assert!(
            polished.contains("42"),
            "number was not preserved: {polished}"
        );
        assert!(
            !polished.to_lowercase().contains("<think"),
            "reasoning leaked: {polished}"
        );
    }
}
