#[cfg(target_os = "windows")]
mod windows_insertion {
    use std::{
        mem::ManuallyDrop,
        ptr, thread,
        time::{Duration, Instant},
    };
    use windows::Win32::{
        Foundation::{GetLastError, GlobalFree, HANDLE, HWND},
        Graphics::Gdi::{HBITMAP, HENHMETAFILE},
        System::{
            Com::{STGMEDIUM, STGMEDIUM_0},
            DataExchange::{
                EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardSequenceNumber,
                SetClipboardData,
            },
            Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
            Ole::{OleDuplicateData, ReleaseStgMedium},
        },
        UI::{
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
                KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY,
            },
            WindowsAndMessaging::{GetForegroundWindow, IsWindow},
        },
    };

    const CF_BITMAP: u32 = 2;
    const CF_METAFILEPICT: u32 = 3;
    const CF_PALETTE: u32 = 9;
    const CF_ENHMETAFILE: u32 = 14;
    const CF_DSPBITMAP: u32 = 0x82;
    const CF_DSPMETAFILEPICT: u32 = 0x83;
    const CF_DSPENHMETAFILE: u32 = 0x8e;
    const CF_UNICODETEXT: u32 = 13;
    const TYMED_HGLOBAL: u32 = 1;
    const TYMED_MFPICT: u32 = 32;
    const TYMED_GDI: u32 = 16;
    const TYMED_ENHMF: u32 = 64;

    struct ClipboardBackup(Vec<(u32, STGMEDIUM)>);

    impl Drop for ClipboardBackup {
        fn drop(&mut self) {
            for (_, medium) in &mut self.0 {
                unsafe { ReleaseStgMedium(medium) };
            }
        }
    }

    fn medium_type(format: u32) -> u32 {
        match format {
            CF_BITMAP | CF_PALETTE | CF_DSPBITMAP => TYMED_GDI,
            CF_METAFILEPICT | CF_DSPMETAFILEPICT => TYMED_MFPICT,
            CF_ENHMETAFILE | CF_DSPENHMETAFILE => TYMED_ENHMF,
            _ => TYMED_HGLOBAL,
        }
    }

    fn capture_clipboard() -> Result<ClipboardBackup, String> {
        let mut backup = ClipboardBackup(Vec::new());
        let mut format = 0;
        loop {
            unsafe {
                windows::Win32::Foundation::SetLastError(windows::Win32::Foundation::WIN32_ERROR(0))
            };
            let next = unsafe { EnumClipboardFormats(format) };
            if next == 0 {
                let error = unsafe { GetLastError() };
                if error.0 != 0 {
                    return Err(format!("Could not enumerate clipboard formats: {error:?}"));
                }
                break;
            }
            let data = unsafe { GetClipboardData(next) }
                .map_err(|error| format!("Could not read clipboard format {next}: {error}"))?;
            let duplicate = unsafe {
                OleDuplicateData(
                    data,
                    windows::Win32::System::Ole::CLIPBOARD_FORMAT(next as u16),
                    Default::default(),
                )
            };
            if duplicate.is_invalid() {
                return Err(format!(
                    "Could not preserve clipboard format {next}; insertion was cancelled"
                ));
            }
            let tymed = medium_type(next);
            let union = match tymed {
                TYMED_GDI => STGMEDIUM_0 {
                    hBitmap: HBITMAP(duplicate.0),
                },
                TYMED_MFPICT => STGMEDIUM_0 {
                    hMetaFilePict: duplicate.0,
                },
                TYMED_ENHMF => STGMEDIUM_0 {
                    hEnhMetaFile: HENHMETAFILE(duplicate.0),
                },
                _ => STGMEDIUM_0 {
                    hGlobal: windows::Win32::Foundation::HGLOBAL(duplicate.0),
                },
            };
            backup.0.push((
                next,
                STGMEDIUM {
                    tymed,
                    u: union,
                    pUnkForRelease: ManuallyDrop::new(None),
                },
            ));
            format = next;
        }
        Ok(backup)
    }

    fn restore_clipboard(backup: &mut ClipboardBackup) -> Result<(), String> {
        unsafe { EmptyClipboard() }
            .map_err(|error| format!("Could not clear clipboard for restoration: {error}"))?;
        for (format, medium) in &mut backup.0 {
            let handle = unsafe {
                match medium.tymed {
                    TYMED_GDI => HANDLE(medium.u.hBitmap.0),
                    TYMED_MFPICT => HANDLE(medium.u.hMetaFilePict),
                    TYMED_ENHMF => HANDLE(medium.u.hEnhMetaFile.0),
                    _ => HANDLE(medium.u.hGlobal.0),
                }
            };
            if let Err(error) = unsafe { SetClipboardData(*format, handle) } {
                return Err(format!(
                    "Could not restore clipboard format {format}: {error}"
                ));
            }
            match medium.tymed {
                TYMED_GDI => medium.u.hBitmap.0 = ptr::null_mut(),
                TYMED_MFPICT => medium.u.hMetaFilePict = ptr::null_mut(),
                TYMED_ENHMF => medium.u.hEnhMetaFile.0 = ptr::null_mut(),
                _ => medium.u.hGlobal.0 = ptr::null_mut(),
            }
        }
        Ok(())
    }

    fn insert_clipboard_text(text: &str) -> Result<(), String> {
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        wide.push(0);
        let bytes = wide.len() * std::mem::size_of::<u16>();
        let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) }
            .map_err(|error| format!("Could not allocate clipboard text: {error}"))?;
        let target = unsafe { GlobalLock(memory) };
        if target.is_null() {
            let _ = unsafe { GlobalFree(memory) };
            return Err("Could not lock memory for clipboard text".into());
        }
        unsafe {
            ptr::copy_nonoverlapping(wide.as_ptr(), target.cast::<u16>(), wide.len());
            let _ = GlobalUnlock(memory);
        }
        if let Err(error) = unsafe { SetClipboardData(CF_UNICODETEXT, HANDLE(memory.0)) } {
            let _ = unsafe { GlobalFree(memory) };
            return Err(format!("Could not set transcript on clipboard: {error}"));
        }
        Ok(())
    }

    fn modifiers_released() -> bool {
        [
            0x10, 0x11, 0x12, 0x5b, 0x5c, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5,
        ]
        .iter()
        .all(|key| unsafe { GetAsyncKeyState(*key) } >= 0)
    }

    fn ensure_original_foreground(hwnd: HWND) -> Result<(), String> {
        if unsafe { IsWindow(hwnd).as_bool() } && unsafe { GetForegroundWindow() } == hwnd {
            Ok(())
        } else {
            Err("The original window is no longer active; transcript was preserved without insertion".into())
        }
    }

    pub fn insert_text(foreground: isize, text: &str) -> Result<(), String> {
        let hwnd = HWND(foreground as *mut _);
        ensure_original_foreground(hwnd)?;

        let release_deadline = Instant::now() + Duration::from_secs(2);
        while !modifiers_released() && Instant::now() < release_deadline {
            thread::sleep(Duration::from_millis(20));
        }
        if !modifiers_released() {
            return Err(
                "Release Ctrl, Shift, Alt, and Windows keys before inserting the transcript".into(),
            );
        }
        ensure_original_foreground(hwnd)?;

        let _clipboard = clipboard_win::Clipboard::new_for(hwnd.0)
            .map_err(|error| format!("Could not open clipboard: {error}"))?;
        let mut backup = capture_clipboard()?;
        let setup = unsafe { EmptyClipboard() }
            .map_err(|error| format!("Could not prepare clipboard: {error}"))
            .and_then(|()| insert_clipboard_text(text));
        if let Err(error) = setup {
            let restore = restore_clipboard(&mut backup);
            return match restore {
                Ok(()) => Err(error),
                Err(restore_error) => Err(format!("{error}; {restore_error}")),
            };
        }
        let sequence = unsafe { GetClipboardSequenceNumber() };
        drop(_clipboard);

        let paste = (|| {
            ensure_original_foreground(hwnd)?;
            let inputs = [
                key_input(0x11, false),
                key_input(0x56, false),
                key_input(0x56, true),
                key_input(0x11, true),
            ];
            let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
            if sent != 4 {
                return Err("Windows accepted only part of the paste keystrokes".into());
            }
            thread::sleep(Duration::from_millis(250));
            Ok(())
        })();

        let restore = if unsafe { GetClipboardSequenceNumber() } == sequence {
            let clipboard = clipboard_win::Clipboard::new_for(hwnd.0)
                .map_err(|error| format!("Could not reopen clipboard for restoration: {error}"))?;
            let result = if unsafe { GetClipboardSequenceNumber() } == sequence {
                restore_clipboard(&mut backup)
            } else {
                Ok(())
            };
            drop(clipboard);
            result
        } else {
            Ok(())
        };

        match (paste, restore) {
            (Err(paste_error), Err(restore_error)) => {
                Err(format!("{paste_error}; {restore_error}"))
            }
            (Err(error), _) | (_, Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    fn key_input(key: u16, key_up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(key),
                    wScan: 0,
                    dwFlags: if key_up {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn unicode_input(unit: u16, key_up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: unit,
                    dwFlags: if key_up {
                        KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
                    } else {
                        KEYEVENTF_UNICODE
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn send_inputs(inputs: &[INPUT]) -> Result<(), String> {
        let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
        if sent as usize != inputs.len() {
            return Err("Windows accepted only part of the live keystrokes".into());
        }
        Ok(())
    }

    fn send_key(key: u16, key_up: bool) -> Result<(), String> {
        send_inputs(&[key_input(key, key_up)])
    }

    fn held_modifiers() -> Vec<u16> {
        [0x10, 0x11, 0x12, 0x5b, 0x5c, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5]
            .into_iter()
            .filter(|key| unsafe { GetAsyncKeyState(*key) } < 0)
            .map(|key| key as u16)
            .collect()
    }

    fn backspace(count: usize) -> Result<(), String> {
        const BATCH: usize = 32;
        let mut remaining = count;
        while remaining > 0 {
            let batch = remaining.min(BATCH);
            let mut inputs = Vec::with_capacity(batch * 2);
            for _ in 0..batch {
                inputs.push(key_input(0x08, false));
                inputs.push(key_input(0x08, true));
            }
            send_inputs(&inputs)?;
            remaining -= batch;
        }
        Ok(())
    }

    fn type_unicode(text: &str) -> Result<(), String> {
        const BATCH: usize = 32;
        let units: Vec<u16> = text.encode_utf16().collect();
        for chunk in units.chunks(BATCH) {
            let mut inputs = Vec::with_capacity(chunk.len() * 2);
            for unit in chunk {
                inputs.push(unicode_input(*unit, false));
                inputs.push(unicode_input(*unit, true));
            }
            send_inputs(&inputs)?;
        }
        Ok(())
    }

    fn live_edit(previous: &str, next: &str) -> (usize, String) {
        let previous: Vec<char> = previous.chars().collect();
        let next: Vec<char> = next.chars().collect();
        let shared = previous
            .iter()
            .zip(next.iter())
            .take_while(|(before, after)| before == after)
            .count();
        (previous.len() - shared, next[shared..].iter().collect())
    }

    pub fn replace_text(foreground: isize, previous: &str, next: &str) -> Result<(), String> {
        let hwnd = HWND(foreground as *mut _);
        ensure_original_foreground(hwnd)?;
        let (deletes, inserted) = live_edit(previous, next);
        if deletes == 0 && inserted.is_empty() {
            return Ok(());
        }

        let modifiers = held_modifiers();
        for key in &modifiers {
            send_key(*key, true)?;
        }
        let typed = (|| {
            ensure_original_foreground(hwnd)?;
            backspace(deletes)?;
            type_unicode(&inserted)
        })();
        let restored = modifiers
            .iter()
            .rev()
            .try_for_each(|key| send_key(*key, false));
        match (typed, restored) {
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn live_edits_only_replace_the_changed_suffix() {
            assert_eq!(live_edit("", "hello"), (0, "hello".into()));
            assert_eq!(live_edit("hello", "hello world"), (0, " world".into()));
            assert_eq!(live_edit("hello world", "hello, world"), (6, ", world".into()));
            assert_eq!(live_edit("same", "same"), (0, String::new()));
            assert_eq!(live_edit("surplus", "sur"), (4, String::new()));
            assert_eq!(
                live_edit("Um, meet Tuesday, no wait, Wednesday at 4", "Meet Wednesday at 4."),
                (41, "Meet Wednesday at 4.".into())
            );
        }

        #[test]
        fn live_edits_keep_whole_surrogate_pairs() {
            assert_eq!(live_edit("a😀", "a😁"), (1, "😁".into()));
            assert_eq!(live_edit("a😀b", "a😁b"), (2, "😁b".into()));
            assert_eq!(live_edit("😀", "😀 done"), (0, " done".into()));
        }

        #[test]
        #[ignore]
        fn rich_and_empty_clipboards_are_restored() {
            let owner = unsafe { GetForegroundWindow() };
            let _clipboard = clipboard_win::Clipboard::new_for(owner.0).unwrap();
            let original = capture_clipboard().unwrap();
            struct RestoreOriginal(ClipboardBackup);
            impl Drop for RestoreOriginal {
                fn drop(&mut self) {
                    restore_clipboard(&mut self.0).expect("Could not restore original clipboard");
                }
            }
            let _original = RestoreOriginal(original);
            let html = unsafe {
                windows::Win32::System::DataExchange::RegisterClipboardFormatW(windows::core::w!(
                    "HTML Format"
                ))
            };
            assert_ne!(html, 0);
            unsafe {
                EmptyClipboard().unwrap();
            }
            insert_clipboard_text("Clipboard sentinel ✓").unwrap();
            let rich = b"<b>Clipboard sentinel</b>\0";
            clipboard_win::raw::set_without_clear(html, rich).unwrap();
            let mut backup = capture_clipboard().unwrap();
            unsafe {
                EmptyClipboard().unwrap();
            }
            insert_clipboard_text("Temporary transcript").unwrap();
            restore_clipboard(&mut backup).unwrap();
            let mut text = Vec::new();
            clipboard_win::raw::get_string(&mut text).unwrap();
            assert_eq!(String::from_utf8(text).unwrap(), "Clipboard sentinel ✓");
            let mut restored_html = Vec::new();
            clipboard_win::raw::get_vec(html, &mut restored_html).unwrap();
            assert_eq!(&restored_html[..rich.len()], rich);
            unsafe {
                EmptyClipboard().unwrap();
            }
            let mut empty = capture_clipboard().unwrap();
            insert_clipboard_text("Temporary transcript").unwrap();
            restore_clipboard(&mut empty).unwrap();
            assert_eq!(unsafe { EnumClipboardFormats(0) }, 0);
        }

        #[test]
        #[ignore]
        fn paste_reaches_a_windows_edit_control() {
            use windows::Win32::UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, DispatchMessageW, GetWindowTextW, PeekMessageW,
                SetForegroundWindow, TranslateMessage, MSG, PM_REMOVE, WINDOW_EX_STYLE,
                WS_OVERLAPPEDWINDOW, WS_VISIBLE,
            };
            let previous = unsafe { GetForegroundWindow() };
            let window = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    windows::core::w!("EDIT"),
                    windows::core::w!("Voxen paste test"),
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                    100,
                    100,
                    480,
                    180,
                    None,
                    None,
                    None,
                    None,
                )
            }
            .unwrap();
            struct TestWindow(HWND, HWND);
            impl Drop for TestWindow {
                fn drop(&mut self) {
                    unsafe {
                        let _ = DestroyWindow(self.0);
                        let _ = SetForegroundWindow(self.1);
                    }
                }
            }
            let _window = TestWindow(window, previous);
            unsafe {
                let _ = SetForegroundWindow(window);
                let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(window);
            }
            assert_eq!(unsafe { GetForegroundWindow() }, window);
            let (sender, receiver) = std::sync::mpsc::channel();
            let target = window.0 as isize;
            thread::spawn(move || {
                let _ = sender.send(insert_text(target, "Voxen paste ✓"));
            });
            let deadline = Instant::now() + Duration::from_secs(5);
            let result = loop {
                let mut message = MSG::default();
                while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() } {
                    unsafe {
                        let _ = TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
                if let Ok(result) = receiver.try_recv() {
                    break result;
                }
                assert!(Instant::now() < deadline, "Paste did not complete");
                thread::sleep(Duration::from_millis(5));
            };
            result.unwrap();
            let mut text = [0u16; 128];
            let length = unsafe { GetWindowTextW(window, &mut text) } as usize;
            assert!(String::from_utf16_lossy(&text[..length]).contains("Voxen paste ✓"));
        }

        #[test]
        #[ignore]
        fn live_typing_revises_unicode_without_duplicating() {
            use windows::Win32::UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, DispatchMessageW, GetWindowTextW, PeekMessageW,
                SetForegroundWindow, TranslateMessage, MSG, PM_REMOVE, WINDOW_EX_STYLE,
                WS_OVERLAPPEDWINDOW, WS_VISIBLE,
            };
            let previous = unsafe { GetForegroundWindow() };
            let window = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    windows::core::w!("EDIT"),
                    windows::core::w!(""),
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                    100,
                    100,
                    480,
                    180,
                    None,
                    None,
                    None,
                    None,
                )
            }
            .unwrap();
            struct TestWindow(HWND, HWND);
            impl Drop for TestWindow {
                fn drop(&mut self) {
                    unsafe {
                        let _ = DestroyWindow(self.0);
                        let _ = SetForegroundWindow(self.1);
                    }
                }
            }
            let _window = TestWindow(window, previous);
            unsafe {
                let _ = SetForegroundWindow(window);
                let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(window);
            }
            assert_eq!(unsafe { GetForegroundWindow() }, window);
            let (sender, receiver) = std::sync::mpsc::channel();
            let target = window.0 as isize;
            thread::spawn(move || {
                let mut result = send_key(0x11, false).and_then(|()| send_key(0x10, false));
                let mut typed = "";
                for next in [
                    "Hel",
                    "Hello",
                    "Hello wor",
                    "Hello world",
                    "Um, Hello world",
                    "Hello, world",
                    "Hello, world ✓ café",
                    "Hello, world ✓ café 😀",
                    "Hello, world ✓ café 😀",
                ] {
                    if result.is_err() {
                        break;
                    }
                    result = replace_text(target, typed, next);
                    typed = next;
                }
                let _ = send_key(0x10, true);
                let _ = send_key(0x11, true);
                let _ = sender.send(result);
            });
            let deadline = Instant::now() + Duration::from_secs(5);
            let result = loop {
                let mut message = MSG::default();
                while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() } {
                    unsafe {
                        let _ = TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
                if let Ok(result) = receiver.try_recv() {
                    break result;
                }
                assert!(Instant::now() < deadline, "Live typing did not complete");
                thread::sleep(Duration::from_millis(5));
            };
            result.unwrap();
            let mut text = [0u16; 128];
            let length = unsafe { GetWindowTextW(window, &mut text) } as usize;
            assert_eq!(
                String::from_utf16_lossy(&text[..length]),
                "Hello, world ✓ café 😀"
            );
        }
    }
}

#[cfg(target_os = "windows")]
pub use windows_insertion::{insert_text, replace_text};

#[cfg(not(target_os = "windows"))]
pub fn insert_text(_: isize, _: &str) -> Result<(), String> {
    Err("Text insertion is supported on Windows only".into())
}

#[cfg(not(target_os = "windows"))]
pub fn replace_text(_: isize, _: &str, _: &str) -> Result<(), String> {
    Err("Text insertion is supported on Windows only".into())
}
