use serde::{Deserialize, Serialize};
use std::sync::{mpsc, OnceLock};
use std::time::Duration;

pub const MAX_CHARS: usize = 800;
const CAPTURE_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Off,
    Selection,
    Nearby,
}

impl Mode {
    pub fn enabled(self) -> bool {
        self != Mode::Off
    }
}

pub fn capture(window: isize, mode: Mode, exclusions: &[String]) -> Option<String> {
    if !mode.enabled() || window == 0 {
        return None;
    }
    #[cfg(target_os = "windows")]
    {
        windows_capture(window, mode, exclusions)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = exclusions;
        None
    }
}

pub fn terms(context: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    let mut sentence_start = true;
    for chunk in context.split_whitespace() {
        let token = chunk.trim_matches(|value: char| {
            !(value.is_alphanumeric()
                || matches!(value, '_' | '-' | '.' | '/' | '@' | '+' | '#' | ':'))
        });
        let length = token.chars().count();
        if (2..=48).contains(&length) {
            let uppercase = token.chars().filter(|value| value.is_uppercase()).count();
            let internal_uppercase = token
                .chars()
                .skip(1)
                .any(|value| value.is_uppercase());
            let technical = token.chars().any(|value| value.is_ascii_digit())
                || token
                    .chars()
                    .any(|value| matches!(value, '_' | '-' | '.' | '/' | '@' | '+' | '#' | ':'))
                || uppercase > 1
                || internal_uppercase
                || (uppercase == 1 && !sentence_start);
            if technical && !terms.iter().any(|known| known.eq_ignore_ascii_case(token)) {
                terms.push(token.to_string());
                if terms.len() == 24 {
                    break;
                }
            }
        }
        sentence_start = chunk
            .chars()
            .last()
            .is_some_and(|value| matches!(value, '.' | '!' | '?' | ':' | ';'));
    }
    terms
}

pub fn excluded(process: &str, exclusions: &[String]) -> bool {
    let process = process.trim().to_lowercase();
    exclusions
        .iter()
        .map(|entry| entry.trim().to_lowercase())
        .filter(|entry| !entry.is_empty())
        .any(|entry| {
            process == entry
                || process == format!("{entry}.exe")
                || process.strip_suffix(".exe") == Some(entry.as_str())
                || entry.strip_suffix(".exe") == Some(process.as_str())
        })
}

fn bounded(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.chars().take(MAX_CHARS).collect())
    }
}

#[cfg(target_os = "windows")]
struct Request {
    window: isize,
    mode: Mode,
    exclusions: Vec<String>,
    reply: mpsc::Sender<Option<String>>,
}

#[cfg(target_os = "windows")]
static WORKER: OnceLock<mpsc::Sender<Request>> = OnceLock::new();

#[cfg(target_os = "windows")]
fn windows_capture(window: isize, mode: Mode, exclusions: &[String]) -> Option<String> {
    let (tx, rx) = mpsc::channel();
    let request = Request {
        window,
        mode,
        exclusions: exclusions.to_vec(),
        reply: tx,
    };
    WORKER
        .get_or_init(|| {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || run_worker(rx));
            tx
        })
        .send(request)
        .ok()?;
    rx.recv_timeout(CAPTURE_TIMEOUT).ok().flatten()
}

#[cfg(target_os = "windows")]
fn run_worker(receiver: mpsc::Receiver<Request>) {
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Accessibility::{CUIAutomation8, IUIAutomation2};

    let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    let automation: Option<IUIAutomation2> =
        unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER) }.ok();
    if let Some(automation) = &automation {
        let _ = unsafe { automation.SetConnectionTimeout(250) };
        let _ = unsafe { automation.SetTransactionTimeout(250) };
    }
    while let Ok(request) = receiver.recv() {
        let text = automation
            .as_ref()
            .and_then(|automation| read_context(automation, &request));
        let _ = request.reply.send(text);
    }
    if initialized {
        unsafe { CoUninitialize() };
    }
}

#[cfg(target_os = "windows")]
fn read_context(
    automation: &windows::Win32::UI::Accessibility::IUIAutomation,
    request: &Request,
) -> Option<String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowLongW, GWL_STYLE};

    let window = HWND(request.window as *mut _);
    if !request.exclusions.is_empty() {
        let process = process_name(window)?;
        if excluded(&process, &request.exclusions) {
            return None;
        }
    }
    let style = unsafe { GetWindowLongW(window, GWL_STYLE) };
    if style & 0x20 != 0 {
        return None;
    }
    let active = unsafe { GetForegroundWindow() };
    if active.0 != window.0 {
        return None;
    }
    let element = unsafe { automation.GetFocusedElement() }.ok()?;
    if element_process(window) != unsafe { element.CurrentProcessId() }.ok() {
        return None;
    }
    if password_field(automation, &element) {
        return None;
    }
    read_text(element, request.mode)
}

#[cfg(target_os = "windows")]
fn element_process(window: windows::Win32::Foundation::HWND) -> Option<i32> {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

    let mut process = 0u32;
    unsafe {
        GetWindowThreadProcessId(window, Some(&mut process));
    }
    (process != 0).then_some(process as i32)
}

#[cfg(target_os = "windows")]
fn password_field(
    automation: &windows::Win32::UI::Accessibility::IUIAutomation,
    element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
) -> bool {
    let Ok(walker) = (unsafe { automation.RawViewWalker() }) else {
        return unsafe { element.CurrentIsPassword() }
            .map(|password| password.as_bool())
            .unwrap_or(false);
    };
    let mut current = element.clone();
    for _ in 0..8 {
        if unsafe { current.CurrentIsPassword() }
            .map(|password| password.as_bool())
            .unwrap_or(false)
        {
            return true;
        }
        match unsafe { walker.GetParentElement(&current) } {
            Ok(parent) => current = parent,
            Err(_) => break,
        }
    }
    false
}

#[cfg(target_os = "windows")]
fn read_text(
    element: windows::Win32::UI::Accessibility::IUIAutomationElement,
    mode: Mode,
) -> Option<String> {
    use windows::Win32::UI::Accessibility::{
        IUIAutomationTextPattern, UIA_TextPatternId,
    };

    let pattern =
        unsafe { element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) }
            .ok()?;
    let selection = unsafe { pattern.GetSelection() }.ok()?;
    let length = unsafe { selection.Length() }.ok()?;
    if length <= 0 {
        return None;
    }
    let range = unsafe { selection.GetElement(0) }.ok()?;
    let selected = bounded(&unsafe { range.GetText(MAX_CHARS as i32) }.ok()?.to_string());
    if mode == Mode::Selection || selected.is_some() {
        return selected;
    }
    nearby(&range)
}

#[cfg(target_os = "windows")]
const NEARBY_CHARS: i32 = 400;

#[cfg(target_os = "windows")]
fn nearby(range: &windows::Win32::UI::Accessibility::IUIAutomationTextRange) -> Option<String> {
    use windows::Win32::UI::Accessibility::{
        TextPatternRangeEndpoint_End, TextPatternRangeEndpoint_Start, TextUnit_Character,
    };

    let window = unsafe { range.Clone() }.ok()?;
    let _ = unsafe {
        window.MoveEndpointByUnit(
            TextPatternRangeEndpoint_Start,
            TextUnit_Character,
            -NEARBY_CHARS,
        )
    };
    let _ = unsafe {
        window.MoveEndpointByUnit(
            TextPatternRangeEndpoint_End,
            TextUnit_Character,
            NEARBY_CHARS,
        )
    };
    bounded(
        &unsafe { window.GetText(MAX_CHARS as i32) }
            .ok()?
            .to_string(),
    )
}

#[cfg(target_os = "windows")]
fn process_name(window: windows::Win32::Foundation::HWND) -> Option<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

    let mut process = 0u32;
    unsafe {
        GetWindowThreadProcessId(window, Some(&mut process));
    }
    if process == 0 {
        return None;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process) }.ok()?;
    let mut buffer = [0u16; 1024];
    let mut length = buffer.len() as u32;
    let result = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    };
    let _ = unsafe { CloseHandle(handle) };
    result.ok()?;
    let path = String::from_utf16_lossy(&buffer[..length as usize]);
    path.rsplit(['\\', '/']).next().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusions_match_executable_names_with_or_without_the_extension() {
        let exclusions = vec!["KeepassXC.exe".into(), "1Password".into()];
        assert!(excluded("keepassxc.exe", &exclusions));
        assert!(excluded("1password.exe", &exclusions));
        assert!(excluded("KeePassXC", &exclusions));
        assert!(!excluded("notepad.exe", &exclusions));
    }

    #[test]
    fn captured_context_is_bounded() {
        let long = "a".repeat(MAX_CHARS + 40);
        assert_eq!(bounded(&long).unwrap().chars().count(), MAX_CHARS);
        assert!(bounded("   ").is_none());
    }

    #[test]
    fn technical_terms_are_extracted_for_word_boosting() {
        let terms = terms("Use PostgreSQL and tokio::spawn with C:\\work\\voxen, not the ordinary words.");
        assert!(terms.contains(&"PostgreSQL".to_string()));
        assert!(terms.contains(&"tokio::spawn".to_string()));
        assert!(!terms.contains(&"ordinary".to_string()));
        assert!(terms.len() <= 24);
    }
}
