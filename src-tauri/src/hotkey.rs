#[cfg(target_os = "windows")]
use rune::PhysicalKeyCode;

#[cfg(target_os = "windows")]
pub fn shortcut_down(shortcut: &str) -> bool {
    let Ok(shortcut) = shortcut.parse::<rune::Shortcut>() else {
        return false;
    };
    let Some(key) = virtual_key(shortcut.physical_key()) else {
        return false;
    };
    let mods = shortcut.modifiers();
    let held = |code: i32| unsafe {
        windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(code) as u16 & 0x8000 != 0
    };
    let ctrl = held(windows::Win32::UI::Input::KeyboardAndMouse::VK_LCONTROL.0 as i32)
        || held(windows::Win32::UI::Input::KeyboardAndMouse::VK_RCONTROL.0 as i32);
    let alt = held(windows::Win32::UI::Input::KeyboardAndMouse::VK_LMENU.0 as i32)
        || held(windows::Win32::UI::Input::KeyboardAndMouse::VK_RMENU.0 as i32);
    let shift = held(windows::Win32::UI::Input::KeyboardAndMouse::VK_LSHIFT.0 as i32)
        || held(windows::Win32::UI::Input::KeyboardAndMouse::VK_RSHIFT.0 as i32);
    let logo = held(windows::Win32::UI::Input::KeyboardAndMouse::VK_LWIN.0 as i32)
        || held(windows::Win32::UI::Input::KeyboardAndMouse::VK_RWIN.0 as i32);
    held(key) && ctrl == mods.ctrl && alt == mods.alt && shift == mods.shift && logo == mods.logo
}

#[cfg(not(target_os = "windows"))]
pub fn shortcut_down(_: &str) -> bool {
    true
}

#[cfg(target_os = "windows")]
fn virtual_key(key: PhysicalKeyCode) -> Option<i32> {
    use PhysicalKeyCode::*;
    let code = match key {
        KeyA => 0x41,
        KeyB => 0x42,
        KeyC => 0x43,
        KeyD => 0x44,
        KeyE => 0x45,
        KeyF => 0x46,
        KeyG => 0x47,
        KeyH => 0x48,
        KeyI => 0x49,
        KeyJ => 0x4A,
        KeyK => 0x4B,
        KeyL => 0x4C,
        KeyM => 0x4D,
        KeyN => 0x4E,
        KeyO => 0x4F,
        KeyP => 0x50,
        KeyQ => 0x51,
        KeyR => 0x52,
        KeyS => 0x53,
        KeyT => 0x54,
        KeyU => 0x55,
        KeyV => 0x56,
        KeyW => 0x57,
        KeyX => 0x58,
        KeyY => 0x59,
        KeyZ => 0x5A,
        Digit0 => 0x30,
        Digit1 => 0x31,
        Digit2 => 0x32,
        Digit3 => 0x33,
        Digit4 => 0x34,
        Digit5 => 0x35,
        Digit6 => 0x36,
        Digit7 => 0x37,
        Digit8 => 0x38,
        Digit9 => 0x39,
        F1 => 0x70,
        F2 => 0x71,
        F3 => 0x72,
        F4 => 0x73,
        F5 => 0x74,
        F6 => 0x75,
        F7 => 0x76,
        F8 => 0x77,
        F9 => 0x78,
        F10 => 0x79,
        F11 => 0x7A,
        F12 => 0x7B,
        F13 => 0x7C,
        F14 => 0x7D,
        F15 => 0x7E,
        F16 => 0x7F,
        F17 => 0x80,
        F18 => 0x81,
        F19 => 0x82,
        F20 => 0x83,
        F21 => 0x84,
        F22 => 0x85,
        F23 => 0x86,
        F24 => 0x87,
        Escape => 0x1B,
        Enter => 0x0D,
        Space => 0x20,
        Tab => 0x09,
        Backspace => 0x08,
        Delete => 0x2E,
        Insert => 0x2D,
        Home => 0x24,
        End => 0x23,
        PageUp => 0x21,
        PageDown => 0x22,
        ArrowLeft => 0x25,
        ArrowUp => 0x26,
        ArrowRight => 0x27,
        ArrowDown => 0x28,
        Minus => 0xBD,
        Equal => 0xBB,
        Comma => 0xBC,
        Period => 0xBE,
        Slash => 0xBF,
        Semicolon => 0xBA,
        Quote => 0xDE,
        BracketLeft => 0xDB,
        BracketRight => 0xDD,
        Backslash => 0xDC,
        Backquote => 0xC0,
        _ => return None,
    };
    Some(code)
}
