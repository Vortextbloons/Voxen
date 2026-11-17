use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use voxen_lib::cleanup::{guard, Cleaner, Mode};

const BENCH_PORT: u16 = 18241;
const COLD_PORT: u16 = 18242;
const MARKERS: [char; 8] = ['.', ',', '!', '?', ':', ';', '"', '\''];

#[derive(Deserialize)]
struct CaseSet {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    name: String,
    raw: String,
    expected: String,
    #[serde(default)]
    #[allow(dead_code)]
    tests: String,
}

#[derive(Deserialize)]
struct ModelSet {
    runtime: String,
    root: String,
    models: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    label: String,
    file: String,
}

struct Options {
    cases: PathBuf,
    models: PathBuf,
    modes: Vec<Mode>,
    repeats: usize,
    check_expected: bool,
    out: PathBuf,
    prompt: Option<PathBuf>,
    runtime: Option<String>,
}

struct Stats {
    id: String,
    name: String,
    guard_error: Option<String>,
    error: bool,
    f1: f64,
    punct: f64,
    caps: f64,
    over_edit: f64,
    extra: f64,
    accepted_f1: f64,
    accepted_punct: f64,
    accepted_caps: f64,
    fallback: bool,
    word_match: bool,
    missing: Vec<String>,
    added: Vec<String>,
    median_ms: u128,
    unstable: bool,
    raw: String,
    expected: String,
    got: String,
    accepted: String,
}

struct ModeReport {
    mode: Mode,
    stats: Vec<Stats>,
    times: Vec<u128>,
}

struct ModelReport {
    label: String,
    file_bytes: u64,
    cold_ms: u128,
    cold_load_ms: Option<u128>,
    server_rss_mib: Option<f64>,
    modes: Vec<ModeReport>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("bench: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let options = parse_args()?;
    let cases: CaseSet = read_json(&options.cases)?;
    if cases.cases.is_empty() {
        return Err(format!("{} has no cases", options.cases.display()));
    }
    if options.check_expected {
        check_expected(&cases);
        return Ok(());
    }
    let models: ModelSet = read_json(&options.models)?;
    if models.models.is_empty() {
        return Err(format!("{} has no models", options.models.display()));
    }
    std::env::set_var("VOXEN_CLEANUP_PORT", BENCH_PORT.to_string());
    let time = unix_time();
    let mut reports = Vec::new();
    let prompt = match &options.prompt {
        Some(path) => Some(
            std::fs::read_to_string(path)
                .map_err(|error| format!("{}: {error}", path.display()))?
                .trim()
                .to_string(),
        ),
        None => None,
    };
    if let Some(path) = &options.prompt {
        println!("prompt: {}\n", path.display());
    }
    let runtime = options.runtime.clone().unwrap_or_else(|| models.runtime.clone());
    if options.runtime.is_some() {
        println!("runtime: {runtime}\n");
    }
    for entry in &models.models {
        let model = PathBuf::from(&models.root).join(&entry.file);
        match run_model(&options, &cases, &runtime, &entry.label, &model, prompt.as_deref()) {
            Ok(report) => reports.push(report),
            Err(error) => println!("== {}\n   skipped: {error}\n", entry.label),
        }
    }
    if reports.is_empty() {
        return Err("no model produced results".into());
    }
    summarize(&reports);
    let written = write_report(&options.out, time, &options, &reports)?;
    println!("\nWrote {}", written.display());
    Ok(())
}

fn run_model(
    options: &Options,
    cases: &CaseSet,
    runtime: &str,
    label: &str,
    model: &Path,
    prompt: Option<&str>,
) -> Result<ModelReport, String> {
    let file_bytes = std::fs::metadata(model).map(|meta| meta.len()).unwrap_or(0);
    let cold_start = measure_cold_start(runtime, model);
    let (cold_load_ms, server_rss_mib) = match cold_start {
        Ok((ms, rss)) => (Some(ms), Some(rss)),
        Err(_) => (None, None),
    };
    let mut cleaner = Cleaner::load(runtime, &model.to_string_lossy())?;
    let cold = Instant::now();
    generate(&mut cleaner, &cases.cases[0].raw, options.modes[0], prompt)
        .map_err(|error| format!("did not start: {error}"))?;
    let cold_ms = cold.elapsed().as_millis();
    println!("== {label}  ({})", model.display());
    let mut modes = Vec::new();
    for mode in &options.modes {
        let mut stats = Vec::new();
        let mut all_times = Vec::new();
        for case in &cases.cases {
            let mut times = Vec::new();
            let mut text = String::new();
            let mut unstable = false;
            let mut failure = None;
            for repeat in 0..options.repeats {
                let start = Instant::now();
                match generate(&mut cleaner, &case.raw, *mode, prompt) {
                    Ok(output) => {
                        times.push(start.elapsed().as_millis());
                        if repeat == 0 {
                            text = output;
                        } else if output != text {
                            unstable = true;
                        }
                    }
                    Err(error) => {
                        failure = Some(error);
                        break;
                    }
                }
            }
            if let Some(error) = failure {
                println!("  [{}] {:<44} error: {error}", case.id, case.name);
                stats.push(Stats {
                    id: case.id.clone(),
                    name: case.name.clone(),
                    guard_error: Some(error),
                    error: true,
                    f1: 0.0,
                    punct: 0.0,
                    caps: 0.0,
                    over_edit: 0.0,
                    extra: 0.0,
                    accepted_f1: 0.0,
                    accepted_punct: 0.0,
                    accepted_caps: 0.0,
                    fallback: true,
                    word_match: false,
                    missing: Vec::new(),
                    added: Vec::new(),
                    median_ms: median(&times),
                    unstable: false,
                    raw: case.raw.clone(),
                    expected: case.expected.clone(),
                    got: String::new(),
                    accepted: case.raw.clone(),
                });
                continue;
            }
            let guard_error = guard(&case.raw, &text).err();
            let raw_score = score(&case.expected, &text);
            let fallback = guard_error.is_some();
            let accepted = if fallback { case.raw.clone() } else { text.clone() };
            let accepted_score = score(&case.expected, &accepted);
            let word_match = raw_score.missing.is_empty() && raw_score.added.is_empty();
            let median_ms = median(&times);
            all_times.extend(&times);
            println!(
                "  [{}] {:<44} {:<10} f1 {:.2} acc {:.2} over-edit {:.2} extra {:.2} punct {:.2} caps {:.2} {median_ms:>5} ms{}",
                case.id,
                case.name,
                if fallback { "fallback" } else { "accepted" },
                raw_score.f1(),
                accepted_score.f1(),
                raw_score.over_edit(),
                raw_score.extra_rate(),
                raw_score.punct(),
                raw_score.caps(),
                if unstable { "  UNSTABLE" } else { "" }
            );
            if let Some(reason) = &guard_error {
                println!("       guard:    {reason}");
            }
            if !raw_score.missing.is_empty() {
                println!("       dropped:  {}", quote_list(&raw_score.missing));
            }
            if !raw_score.added.is_empty() {
                println!("       added:    {}", quote_list(&raw_score.added));
            }
            if text != case.expected {
                println!("       expected: {}", case.expected);
                println!("       got:      {text}");
            }
            stats.push(Stats {
                id: case.id.clone(),
                name: case.name.clone(),
                guard_error,
                error: false,
                f1: raw_score.f1(),
                punct: raw_score.punct(),
                caps: raw_score.caps(),
                over_edit: raw_score.over_edit(),
                extra: raw_score.extra_rate(),
                accepted_f1: accepted_score.f1(),
                accepted_punct: accepted_score.punct(),
                accepted_caps: accepted_score.caps(),
                fallback,
                word_match,
                missing: raw_score.missing,
                added: raw_score.added,
                median_ms,
                unstable,
                raw: case.raw.clone(),
                expected: case.expected.clone(),
                got: text,
                accepted,
            });
        }
        println!("  {}", aggregate_line(*mode, &stats, &all_times));
        modes.push(ModeReport { mode: *mode, stats, times: all_times });
    }
    cleaner.unload();
    println!();
    Ok(ModelReport {
        label: label.to_string(),
        file_bytes,
        cold_ms,
        cold_load_ms,
        server_rss_mib,
        modes,
    })
}

fn measure_cold_start(runtime: &str, model: &Path) -> Result<(u128, f64), String> {
    let mut command = Command::new(runtime);
    command
        .arg("-m")
        .arg(model)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(COLD_PORT.to_string())
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
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let start = Instant::now();
    let mut ready = false;
    while start.elapsed() < Duration::from_secs(90) {
        match child.try_wait() {
            Ok(None) => {}
            _ => break,
        }
        if health_ok(COLD_PORT) {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let elapsed = start.elapsed().as_millis();
    if !ready {
        let _ = child.kill();
        let _ = child.wait();
        return Err("cold load did not become ready".into());
    }
    let rss = process_rss_mib(child.id());
    let _ = child.kill();
    let _ = child.wait();
    Ok((elapsed, rss))
}

fn health_ok(port: u16) -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_secs(1)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let request = "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut raw = Vec::new();
    let mut chunk = [0u8; 2048];
    while let Ok(read) = stream.read(&mut chunk) {
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..read]);
    }
    raw.windows(4).any(|window| window == b"\"ok\"")
}

fn process_rss_mib(pid: u32) -> f64 {
    let script = format!("(Get-Process -Id {pid}).WorkingSet64");
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output();
    match output {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<f64>()
            .map(|bytes| bytes / 1_048_576.0)
            .unwrap_or(0.0),
        _ => 0.0,
    }
}

fn threads() -> usize {
    std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(4)
        .min(8)
}

fn generate(
    cleaner: &mut Cleaner,
    raw: &str,
    mode: Mode,
    prompt: Option<&str>,
) -> Result<String, String> {
    match prompt {
        Some(system) => cleaner.clean_with_prompt(raw, system),
        None => cleaner.clean_unguarded(raw, mode),
    }
}

fn summarize(reports: &[ModelReport]) {
    println!(
        "{:<18} {:<9} {:>7} {:>6} {:>6} {:>7} {:>7} {:>8} {:>7} {:>8} {:>7} {:>6}",
        "model", "mode", "guard", "raw f1", "acc f1", "fallbk", "p50", "p95", "cold", "load", "rss", "size"
    );
    for report in reports {
        for entry in &report.modes {
            let passed = entry.stats.iter().filter(|stat| stat.guard_error.is_none()).count();
            let fallbacks = entry.stats.iter().filter(|stat| stat.fallback).count();
            println!(
                "{:<18} {:<9} {:>7} {:>6.2} {:>6.2} {:>7} {:>6} ms {:>6} ms {:>6} ms {:>6} MiB {:>6} MiB {:>6.1} MiB",
                report.label,
                mode_name(entry.mode),
                format!("{passed}/{}", entry.stats.len()),
                mean(&entry.stats, |stat| stat.f1),
                mean(&entry.stats, |stat| stat.accepted_f1),
                format!("{fallbacks}/{}", entry.stats.len()),
                percentile(&entry.times, 0.50),
                percentile(&entry.times, 0.95),
                report.cold_ms,
                report.cold_load_ms.map_or("-".to_string(), |value| value.to_string()),
                report.server_rss_mib.map_or("-".to_string(), |value| format!("{value:.0}")),
                report.file_bytes as f64 / 1_048_576.0,
            );
        }
    }
}

fn aggregate_line(mode: Mode, stats: &[Stats], times: &[u128]) -> String {
    let passed = stats.iter().filter(|stat| stat.guard_error.is_none()).count();
    let fallbacks = stats.iter().filter(|stat| stat.fallback).count();
    let unstable = stats.iter().filter(|stat| stat.unstable).count();
    let errors = stats.iter().filter(|stat| stat.error).count();
    let word_matches = stats.iter().filter(|stat| stat.word_match).count();
    format!(
        "{}: accepted {passed}/{}, raw f1 {:.2}, accepted f1 {:.2}, word match {word_matches}/{}, over-edit {:.2}, extra {:.2}, punct {:.2}, caps {:.2}, p50 {} ms, p95 {} ms{}{}{}",
        mode_name(mode),
        stats.len(),
        mean(stats, |stat| stat.f1),
        mean(stats, |stat| stat.accepted_f1),
        stats.len(),
        mean(stats, |stat| stat.over_edit),
        mean(stats, |stat| stat.extra),
        mean(stats, |stat| stat.punct),
        mean(stats, |stat| stat.caps),
        percentile(times, 0.50),
        percentile(times, 0.95),
        if fallbacks > 0 {
            format!(", {fallbacks} fallback")
        } else {
            String::new()
        },
        if errors > 0 {
            format!(", {errors} errored")
        } else {
            String::new()
        },
        if unstable > 0 {
            format!(", {unstable} unstable")
        } else {
            String::new()
        }
    )
}

fn check_expected(cases: &CaseSet) {
    println!("Checking the benchmark itself against Voxen's guard (no model needed)\n");
    let mut failures = Vec::new();
    for case in &cases.cases {
        match guard(&case.raw, &case.expected) {
            Ok(()) => println!("  [{}] {:<44} guard ok", case.id, case.name),
            Err(reason) => {
                println!("  [{}] {:<44} guard FAIL: {reason}", case.id, case.name);
                failures.push(case.id.clone());
            }
        }
    }
    if failures.is_empty() {
        println!("\nAll {} expected outputs satisfy the shipped guard.", cases.cases.len());
    } else {
        println!(
            "\n{} of {} expected outputs are rejected by the shipped guard: cases {}",
            failures.len(),
            cases.cases.len(),
            failures.join(", ")
        );
    }
}

fn write_report(
    path: &Path,
    time: u128,
    options: &Options,
    reports: &[ModelReport],
) -> Result<PathBuf, String> {
    let payload = json!({
        "time": time,
        "repeats": options.repeats,
        "modes": options.modes.iter().map(|mode| mode_name(*mode)).collect::<Vec<_>>(),
        "models": reports.iter().map(|report| json!({
            "label": report.label,
            "fileBytes": report.file_bytes,
            "coldMs": report.cold_ms,
            "coldLoadMs": report.cold_load_ms,
            "serverRssMiB": report.server_rss_mib,
            "modes": report.modes.iter().map(|entry| json!({
                "mode": mode_name(entry.mode),
                "p50Ms": percentile(&entry.times, 0.50),
                "p95Ms": percentile(&entry.times, 0.95),
                "cases": entry.stats.iter().map(|stat| json!({
                    "id": stat.id,
                    "name": stat.name,
                    "guard": stat.guard_error,
                    "error": stat.error,
                    "f1": stat.f1,
                    "acceptedF1": stat.accepted_f1,
                    "acceptedPunct": stat.accepted_punct,
                    "acceptedCaps": stat.accepted_caps,
                    "fallback": stat.fallback,
                    "wordMatch": stat.word_match,
                    "overEdit": stat.over_edit,
                    "extra": stat.extra,
                    "punct": stat.punct,
                    "caps": stat.caps,
                    "dropped": stat.missing,
                    "added": stat.added,
                    "medianMs": stat.median_ms,
                    "unstable": stat.unstable,
                    "raw": stat.raw,
                    "expected": stat.expected,
                    "got": stat.got,
                    "accepted": stat.accepted,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(&payload).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(path.to_path_buf())
}

struct Score {
    matched: usize,
    expected: usize,
    actual: usize,
    punct: f64,
    caps_matches: usize,
    missing: Vec<String>,
    added: Vec<String>,
}

impl Score {
    fn f1(&self) -> f64 {
        if self.expected + self.actual == 0 {
            return 0.0;
        }
        2.0 * self.matched as f64 / (self.expected + self.actual) as f64
    }

    fn over_edit(&self) -> f64 {
        if self.expected == 0 {
            return 0.0;
        }
        self.missing.len() as f64 / self.expected as f64
    }

    fn extra_rate(&self) -> f64 {
        if self.expected == 0 {
            return 0.0;
        }
        self.added.len() as f64 / self.expected as f64
    }

    fn punct(&self) -> f64 {
        self.punct
    }

    fn caps(&self) -> f64 {
        if self.matched == 0 {
            return 0.0;
        }
        self.caps_matches as f64 / self.matched as f64
    }
}

struct Word {
    core: String,
    capital: bool,
}

fn marks(text: &str) -> BTreeMap<char, usize> {
    let mut counts = BTreeMap::new();
    for chunk in text.split_whitespace() {
        let start = chunk.trim_start_matches(['"', '\'', '(', '[', '{', '<']);
        let core = start.trim_end_matches(['.', ',', '!', '?', ':', ';', '"', '\'', ')', ']', '}', '>']);
        for mark in start[core.len()..].chars().filter(|mark| MARKERS.contains(mark)) {
            *counts.entry(mark).or_insert(0) += 1;
        }
    }
    counts
}

fn punct_f1(expected: &str, actual: &str) -> f64 {
    let expected = marks(expected);
    let actual = marks(actual);
    let expected_total: usize = expected.values().sum();
    let actual_total: usize = actual.values().sum();
    if expected_total + actual_total == 0 {
        return 1.0;
    }
    let shared: usize = expected
        .iter()
        .map(|(mark, count)| actual.get(mark).copied().unwrap_or(0).min(*count))
        .sum();
    2.0 * shared as f64 / (expected_total + actual_total) as f64
}

fn score(expected: &str, actual: &str) -> Score {
    let punct = punct_f1(expected, actual);
    let expected = words(expected);
    let actual = words(actual);
    let mut table = vec![vec![0usize; actual.len() + 1]; expected.len() + 1];
    for i in (0..expected.len()).rev() {
        for j in (0..actual.len()).rev() {
            table[i][j] = if expected[i].core == actual[j].core {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    let mut missing = Vec::new();
    let mut added = Vec::new();
    let mut caps_matches = 0;
    let (mut i, mut j) = (0, 0);
    while i < expected.len() && j < actual.len() {
        if expected[i].core == actual[j].core {
            if expected[i].capital == actual[j].capital {
                caps_matches += 1;
            }
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            missing.push(expected[i].core.clone());
            i += 1;
        } else {
            added.push(actual[j].core.clone());
            j += 1;
        }
    }
    missing.extend(expected[i..].iter().map(|word| word.core.clone()));
    added.extend(actual[j..].iter().map(|word| word.core.clone()));
    Score {
        matched: table[0][0],
        expected: expected.len(),
        actual: actual.len(),
        punct,
        caps_matches,
        missing,
        added,
    }
}

fn words(text: &str) -> Vec<Word> {
    text.split_whitespace()
        .map(|chunk| {
            let start = chunk.trim_start_matches(['"', '\'', '(', '[', '{', '<']);
            let core = start.trim_end_matches(['.', ',', '!', '?', ':', ';', '"', '\'', ')', ']', '}', '>']);
            Word {
                core: core.to_lowercase(),
                capital: start.chars().next().is_some_and(char::is_uppercase),
            }
        })
        .filter(|word| !word.core.is_empty())
        .collect()
}

fn median(times: &[u128]) -> u128 {
    if times.is_empty() {
        return 0;
    }
    let mut sorted = times.to_vec();
    sorted.sort_unstable();
    sorted[sorted.len() / 2]
}

fn percentile(times: &[u128], fraction: f64) -> u128 {
    if times.is_empty() {
        return 0;
    }
    let mut sorted = times.to_vec();
    sorted.sort_unstable();
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn mean(stats: &[Stats], value: impl Fn(&Stats) -> f64) -> f64 {
    if stats.is_empty() {
        return 0.0;
    }
    stats.iter().map(value).sum::<f64>() / stats.len() as f64
}

fn quote_list(words: &[String]) -> String {
    let mut rendered: Vec<String> = words.iter().take(12).map(|word| format!("\"{word}\"")).collect();
    if words.len() > 12 {
        rendered.push(format!("and {} more", words.len() - 12));
    }
    rendered.join(", ")
}

fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Raw => "raw",
        Mode::Light => "light",
        Mode::Polished => "polished",
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("{}: {error}", path.display()))
}

fn parse_args() -> Result<Options, String> {
    let mut options = Options {
        cases: repo_file("bench/cases.json"),
        models: repo_file("bench/models.json"),
        modes: vec![Mode::Light, Mode::Polished],
        repeats: 3,
        check_expected: false,
        out: repo_file(&format!("runtime/bench-{}.json", unix_time())),
        prompt: None,
        runtime: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--cases" => options.cases = PathBuf::from(next_value(&mut args, &flag)?),
            "--models" => options.models = PathBuf::from(next_value(&mut args, &flag)?),
            "--out" => options.out = PathBuf::from(next_value(&mut args, &flag)?),
            "--repeats" => {
                let value = next_value(&mut args, &flag)?;
                options.repeats = value
                    .parse::<usize>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| format!("--repeats needs a positive number, got {value}"))?;
            }
            "--mode" => {
                options.modes = match next_value(&mut args, &flag)?.as_str() {
                    "light" => vec![Mode::Light],
                    "polished" => vec![Mode::Polished],
                    "both" => vec![Mode::Light, Mode::Polished],
                    other => return Err(format!("--mode expects light, polished or both, got {other}")),
                };
            }
            "--runtime" => options.runtime = Some(next_value(&mut args, &flag)?),
            "--prompt" => {
                options.prompt = Some(PathBuf::from(next_value(&mut args, &flag)?));
                options.modes.truncate(1);
            }
            "--check-expected" => options.check_expected = true,
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(format!("unknown option {other}, try --help")),
        }
    }
    Ok(options)
}

fn print_help() {
    println!(
        "Cleanup benchmark\n\n\
         cargo run --example bench -- [options]\n\n\
         --check-expected    validate bench/cases.json against Voxen's guard, no model needed\n\
         --cases <path>      cases file (default bench/cases.json)\n\
         --models <path>     models file (default bench/models.json)\n\
         --mode <which>      light, polished or both (default both)\n\
         --prompt <path>     use this file as the system prompt instead of a shipped mode\n\
         --runtime <path>    llama-server binary to use instead of the models file default\n\
         --repeats <n>       runs per case for timing (default 3)\n\
         --out <path>        json report path (default runtime/bench-<time>.json)"
    );
}

fn next_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn repo_file(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|root| root.join(relative))
        .unwrap_or_else(|| PathBuf::from(relative))
}

fn unix_time() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs() as u128)
        .unwrap_or(0)
}
