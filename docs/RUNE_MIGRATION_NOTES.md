# Rune migration notes (research)

Verified against the pinned checkout
`%USERPROFILE%\.cargo\git\checkouts\rune-c9b805e9ed8a3730\eb5ae6c`
(`git rev-parse HEAD` = `eb5ae6c9f77b1a0051d8e1e5ee936ebea086bb90`).
Paths below are relative to that checkout unless they start with `src-tauri/`.
Public re-exports are in `crates/rune/src/lib.rs`; `Waker`, `Shortcut`, `ShortcutId`,
`ShortcutError`, `PhysicalKeyCode` are all exported from the `rune` root.

## 1. App / AppHandle lifetime

`crates/rune/src/app.rs:54` (`App`), `:669` (`AppHandle`), `crates/rune/src/tooling.rs:17` (`AppSpec`).

```rust
impl<S: 'static> App<S> {
    pub fn new(spec: AppSpec<S>) -> Self;                       // app.rs:116
    pub fn init(mut self, f: impl FnOnce() -> S + 'static) -> Self;  // app.rs:369
    pub fn keep_running(mut self) -> Self;                      // app.rs:391
    pub fn windowless(mut self) -> Self;                        // app.rs:383 (implies keep_running)
    pub fn release_gpu_when_closed(mut self) -> Self;           // app.rs:399
    pub fn on_close(mut self, f: fn(&mut S, &mut Tx)) -> Self;  // app.rs:446
    pub fn on_quit(mut self, f: fn(&mut S, &mut Tx)) -> Self;   // app.rs:461
    pub fn overlay<T: 'static>(&mut self, spec: impl Into<ConfiguredOverlay<T>>, state: T) -> OverlayHandle<T>; // :344
    pub fn handle(&self) -> AppHandle<S>;                       // app.rs:574, usable before run
    pub fn run(self);                                           // app.rs:609 (exits 1 on failure)
    pub fn try_run(self) -> Result<(), String>;                 // app.rs:624
}

impl<S: 'static> AppHandle<S> {                                 // Clone + Send + Sync
    pub fn open(&self);          // show + focus, recreates a closed window      :692
    pub fn show(&self);          // show, no focus, keeps minimized minimized     :698
    pub fn hide(&self);          // keeps state, does NOT run on_close            :703
    pub fn focus(&self);         // :709
    pub fn close(&self);         // as the close button; on_close runs            :737
    pub fn quit(&self);          // on_quit runs, windows close without on_close  :757
    pub fn is_visible(&self) -> bool;                             // :777
    pub fn window_error(&self) -> Option<String>;                 // :783 no display/adapter/device
    pub fn update(&self, f: impl FnOnce(&mut S, &mut Tx) + Send + 'static);          // :789
    pub fn update_if(&self, f: impl FnOnce(&mut S, &mut Tx) -> bool + Send + 'static); // :801
}
```

- `AppSpec::init` is a plain `fn() -> S`; a captured handle (the `Engine`) requires
  `App::init(move || VoxenApp::new(engine))`. `init` runs when the loop starts, not at
  construction (`app.rs:658` `into_parts`).
- `AppHandle` calls are queued and applied on the UI thread inside the event loop, in call order,
  also while no window shows; window requests collapse to the latest (`app.rs:667`). This is why
  `handle.update(move |_, _| startup.ensure_shortcut())` called before `run` registers the
  global shortcut once the host exists (`src-tauri/src/lib.rs:1266`).
- `keep_running`: closing hides the window on Windows and `on_close` runs; state, `Runtime`,
  GPU device and renderer stay, so reopening is settled on its first frame (README "App
  lifetime").
- `Tx::quit` / `Tx::close_window` from a handler: `crates/rune/src/el/tx.rs:381,387`.
- Exit is a *fast exit* by default: the process ends without unwinding (`crates/rune/src/quit.rs`,
  `fast_exit`). Destructors do not run and buffered file writes are at risk, so persist config
  synchronously in the `on_quit` hook. Rune removes tray icons itself (`quit.rs:146` ->
  `tray::remove_all`).

Working shape (`src-tauri/src/lib.rs:1230`):

```rust
let mut app = rune::App::new(spec)
    .keep_running()
    .allow_unmodified_shortcuts()
    .on_close(|state, tx| state.on_close(tx))
    .on_quit(|state, _| state.engine.shutdown());
let handle = app.handle();
let engine = Engine::new(handle.clone());
let indicator = app.overlay(overlay_spec(), indicator::Indicator::new());
engine.attach_indicator(indicator);
let init_engine = engine.clone();
app = app.init(move || ui::VoxenApp::new(init_engine));
app.run();
```

## 2. Global shortcuts

`crates/rune/src/shortcuts/mod.rs`, `parse.rs`, `keys.rs`, `windows.rs`; prose in
`crates/rune/src/shortcuts/README.md`.

```rust
impl<S: 'static> App<S> {
    pub fn global_shortcut(self, accelerator: &str, handler: fn(&mut S, &mut Tx)) -> Self; // :110
    pub fn allow_unmodified_shortcuts(self) -> Self;                                       // :118
}
impl<S: 'static> AppHandle<S> {
    pub fn register_shortcut(&self, shortcut: Shortcut,
        handler: impl Fn(&mut S, &mut Tx) + Send + Sync + 'static) -> Result<ShortcutId, ShortcutError>; // :130
    pub fn unregister_shortcut(&self, id: ShortcutId) -> Result<(), ShortcutError>;        // :135
    pub fn shortcut_owner(&self, shortcut: &Shortcut) -> Option<String>;                   // :144
    pub fn take_shortcut_errors(&self) -> Vec<ShortcutError>;                              // :149
}
pub struct ShortcutId(u32);        // Copy + Eq + Hash, opaque, process-unique (mod.rs:28)
pub enum ShortcutError { Conflict, Unsupported, InvalidAccelerator(String), NotRunning, UnknownId, PermissionDenied, Backend(String) } // :33
impl Shortcut {                    // FromStr<Err = ShortcutError> + Display
    pub fn parse_on(input: &str, platform: KeyPlatform) -> Result<Self, ShortcutError>;    // parse.rs:29
    pub fn from_physical_key(key: PhysicalKeyCode, mods: Mods) -> Result<Self, ShortcutError>; // keys.rs:28
    pub fn physical_key(&self) -> PhysicalKeyCode;                                         // keys.rs:38
    pub fn require_modifier(&self) -> Result<(), ShortcutError>;                           // keys.rs:43
    pub fn modifiers(&self) -> Mods;                                                       // keys.rs:50
    pub fn keycaps(&self) -> Vec<String>;                                                  // keys.rs:77
    pub fn menu_keycaps(&self) -> Vec<String>;                                             // keys.rs:88
}
```

- `PhysicalKeyCode` is `winit::keyboard::KeyCode` re-exported (`keys.rs:3`). `Shortcut` stores the
  physical key (`parse.rs:13-20`); `from_physical_key` refuses modifier keys alone.
- Parsing: `cmd|command|super|meta|logo|win` are Super, `mod` is Ctrl on Windows
  (`parse.rs:36-68`). Display normalizes to `ctrl+alt+shift+super` + lowercase key, so persist
  `Display`/`FromStr`, never `keycaps()`.
- A chord needs Ctrl/Alt/Super or F1–F24 or registration fails with `InvalidAccelerator`
  (`keys.rs:43`, enforced in `service.rs`); `allow_unmodified_shortcuts()` lifts it, which a
  hold-to-talk binding such as a bare key needs.
- Runtime registration must happen after the host starts; before/after the loop it returns
  `NotRunning`. On Windows it runs `RegisterHotKey` on the event-loop thread, so
  `handle.update(..)` (as `lib.rs` does) or `on_window`/`on_ready` is the place. Off-thread calls
  block until the loop processes them; never join such a caller from the app thread.
- Windows backend (`shortcuts/windows.rs`): a message-only window (`HWND_MESSAGE`),
  `RegisterHotKey(.., MOD_NOREPEAT, ..)`, one `WM_HOTKEY` per activation. Reserved combinations
  (Win+L etc.) fail; `ERROR_HOTKEY_ALREADY_REGISTERED` maps to `Conflict`. The physical scan code
  is resolved to a VK with `MapVirtualKeyW(MAPVK_VSC_TO_VK_EX)` (`:63-67`), so bindings follow the
  current layout; re-register after a layout change.
- **Press only on Windows.** `WM_HOTKEY` has no release counterpart and repeat is off, so there
  is no release event to consume. Hold-to-talk must poll:
  `windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(vk) & 0x8000 != 0` for the key
  and for each modifier, repeated from `poll` every ~40 ms while starting/recording. This is what
  `src-tauri/src/hotkey.rs:5` does, driven from `Engine::shortcut_down` (`src-tauri/src/lib.rs:577`).
  A `WH_KEYBOARD_LL` hook would give true down/up, but needs its own message loop, a global hook
  install and timeout handling, for no gain here.
- Caveat for the probe: `GetAsyncKeyState(VK_SHIFT | VK_CONTROL | VK_MENU)` is resolved through
  the layout's scan-code mapping (effectively one side), so testing `VK_LSHIFT || VK_RSHIFT`
  (and the L/R Control/Menu pairs) is the reliable form. Also remember Left Ctrl + Left Alt is
  AltGr on Windows; requiring an exact modifier match avoids treating AltGr as the chord.
- Modifier **release** events reach `on_key` only with `App::modifier_keys()` (`app.rs:256`,
  README "Modifier keys alone"): Shift/Control/Alt/Super/Fn arrive as
  `Key::Modifier { modifier, pressed }`. Shortcut capture that ends "when all modifiers are
  released" therefore needs `.modifier_keys()` on the app (or the overlay's
  `ConfiguredOverlay::modifier_keys`, `overlay/options.rs:183`); otherwise only real key presses
  arrive and capture can only end on Escape/Tab.
- Capture a chord with `rune::Shortcut::from_physical_key(tx.physical_key()?, ev.mods)`
  (`el/tx/keys.rs:20` for `physical_key`); `tx.key_repeat()` filters autorepeat.

## 3. Tray

`crates/rune/src/tray.rs` (Windows backend `tray/win32.rs`; macOS unsupported, `tray.rs:9`).

```rust
pub struct TrayIcon;                                    // tray.rs:22
impl TrayIcon { pub fn rgba(width: u32, height: u32, pixels: Vec<u8>) -> Self; }        // :24

pub enum TrayItem { Action { id: String, label: String, enabled: bool }, Separator }      // :29
impl TrayItem {
    pub fn action(id: impl Into<String>, label: impl Into<String>) -> Self;             // :35
    pub fn enabled(self, enabled: bool) -> Self;                                        // :38
}

pub enum TrayEvent { Activate { token: Option<String> }, Menu(String) }                 // :45

pub struct TraySpec { /* private fields */ }                                            // :55
impl TraySpec {
    pub fn new(id: impl Into<String>, title: impl Into<String>, icon: TrayIcon) -> Self; // :65
    pub fn tooltip(self, tooltip: impl Into<String>) -> Self;                            // :68
    pub fn icon_name(self, name: impl Into<String>) -> Self;                             // :70 (Linux only)
    pub fn menu(self, menu: Vec<TrayItem>) -> Self;                                      // :71
}

pub struct Tray;                                                                        // :88
impl Tray {
    pub fn new(spec: TraySpec, on_event: impl Fn(TrayEvent) + Send + Sync + 'static) -> Result<Self, TrayError>; // :103
    pub fn set_menu(&self, menu: Vec<TrayItem>);                                        // :112
    pub fn set_tooltip(&self, tooltip: impl Into<String>);                              // :118
}
pub enum TrayError { Unsupported, Backend(String) }                                     // :76
```

- `TrayIcon::rgba` takes straight (not premultiplied) RGBA, exactly `width * height * 4` bytes;
  a mismatch is `TrayError::Backend` (`tray.rs:104`). Windows converts to BGRA and builds the
  mask itself (`tray/win32.rs:23-29`).
- Each tray owns a thread with its own message-only window and loop (`tray.rs:2-5`), so the
  handler runs *off* the app thread: use `AppHandle` (`open`, `quit`), never `&mut` state.
  A left click is `TrayEvent::Activate { token: None }`; a right click builds and shows a
  `TrackPopupMenu` from the current menu, and a pick is `TrayEvent::Menu(id)`
  (`tray/win32.rs:82-104`). The icon re-adds itself when Explorer restarts (`TaskbarCreated`).
- **The `Tray` value owns the icon**: dropping it removes it, so keep it alive for the process
  (the current code uses `std::mem::forget`, `src-tauri/src/lib.rs:1225`). A fast exit removes
  icons for all live trays (`quit.rs:146`).
- Icon bytes from `src-tauri/icons/icon.png` with the already-declared `png` crate (no new
  dependency). `src-tauri/icons/icon.png` is 512x512; `Shell_NotifyIcon` uses the pixels as
  given and lets Explorer scale them, so a 32x32 (or 16/32/48 per DPI) source is sharper. The
  current `tray_icon()` in `src-tauri/src/lib.rs:1179` decodes the 512 px file and rejects
  indexed PNGs.

```rust
fn tray_icon() -> Option<TrayIcon> {
    let bytes = include_bytes!("../icons/icon.png");
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).ok()?;
    buffer.truncate(info.buffer_size());
    Some(TrayIcon::rgba(info.width, info.height, buffer))
}
```

## 4. Overlays (recording indicator)

`crates/rune/src/overlay.rs`, `overlay/options.rs`, `overlay/placement.rs`,
`platform/win32.rs:376-412`.

```rust
pub struct OverlaySpec<T: 'static> {           // overlay.rs:75
    pub name: &'static str,
    pub size: (f32, f32),                       // points
    pub edge: ScreenEdge,                       // Top | Bottom (overlay.rs:69)
    pub margin: f32,                            // points from the edge
    pub align: Align,                           // Start | Center | End
    pub inset: f32,
    pub interactive: bool,                      // false = every click passes through
    pub view: fn(&T, &mut Cx<T>) -> El,
}
impl<T: 'static> OverlaySpec<T> {
    pub fn new(name: &'static str, size: (f32, f32), view: fn(&T, &mut Cx<T>) -> El) -> Self; // options.rs:226
    pub fn options(self, options: OverlayOptions) -> ConfiguredOverlay<T>;                     // options.rs:230
}

impl<T: 'static> OverlayHandle<T> {            // Clone
    pub fn show(&self);                        // overlay.rs:185
    pub fn hide(&self);                        // :189
    pub fn fade_in(&self, duration: Duration); // :195 macOS only, else at once
    pub fn fade_out(&self, duration: Duration); // :205 macOS + Windows alpha fade
    pub fn update(&self, f: impl FnOnce(&mut T, &mut Tx) + Send + 'static);          // :242
    pub fn update_if(&self, f: impl FnOnce(&mut T, &mut Tx) -> bool + Send + 'static); // :253
    pub fn resize(&self, size: (f32, f32));    // :233
    pub fn place(&self, edge: ScreenEdge, margin: f32);   // :220
    pub fn set_align(&self, align: Align, inset: f32);    // :228
    pub fn set_input(&self, input: OverlayInput);         // :148
    pub fn availability(&self) -> Availability;           // :257
    pub fn failure(&self) -> Option<String>;              // :266
    pub fn surface_description(&self) -> Option<String>;  // :272
}
```

- Overlays are created hidden by `App::overlay` and need `show()`; `update` works while hidden
  and the frame that shows builds once with the latest state (`app.rs:340`, `overlay.rs:237`).
- **Click-through**: `interactive: false` (equivalently `OverlayInput::None`) makes Windows set
  `WS_EX_TRANSPARENT | WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST`
  and show with `SW_SHOWNOACTIVATE` (`platform/win32.rs:381-390`, `overlay.rs:17-18`); the
  overlay never takes focus and never activates the app. `interactive: true` takes every click;
  `OverlayInput::Regions` takes only elements marked `El::hit()` (`options.rs:82-98`).
- **Transparency**: the overlay surface is transparent by construction; the view's root must
  have no fill, and whatever it leaves clear stays transparent (`overlay.rs:88-90`). Use
  translucent fills (`.fill(color.alpha(..))`) and `.radius(..)` for a pill.
- **Bottom-centre**: `edge: ScreenEdge::Bottom`, `align: Align::Center`, `margin` points above
  the bottom; the position is computed inside the display's work area on Windows, so it sits
  above the taskbar (`overlay/placement.rs:22-37`). Size and margin are logical points and get
  multiplied by the display scale (`placement.rs:20-21`).
- Overlays use the app's fonts, and inherit `AppSpec::poll` only when their state type matches the
  app's (`options.rs:168-172`). `Indicator` has its own state type, so it gets **no** poll:
  animate with `rune_motion` loops/transitions (`El::looping(LoopSpec::opacity(..))`,
  `crates/rune-motion/src/looping.rs:33`; reduced motion holds the first pose) or attach
  `.poll(fn(&mut Indicator, f64, &mut Tx) -> bool)` on a `ConfiguredOverlay`.
- `Tx::wake_after` deadlines run for hidden overlays too, without drawing (`overlay.rs:238`).
- A 220x76 indicator with `interactive: false` and no root fill is exactly what the contract
  describes; the lead already wires it at `src-tauri/src/lib.rs:1250-1263`.

## 5. AppSpec

`crates/rune/src/tooling.rs:17`.

```rust
pub struct AppSpec<S: 'static> {
    pub title: &'static str,
    pub size: (f32, f32),
    pub min_size: Option<(f32, f32)>,          // None keeps 420x400
    pub init: fn() -> S,
    pub view: fn(&S, &mut Cx<S>) -> El,
    pub scenes: &'static [(&'static str, fn(&mut S))],
    pub fonts: &'static [&'static [u8]],
    pub families: (&'static str, &'static str),
    pub setup: Option<fn(&mut S, Waker)>,       // once, on the event thread, before the first frame
    pub poll: Option<fn(&mut S, f64, &mut Tx) -> bool>,  // before every frame; clock in seconds
    pub on_key: Option<fn(&mut S, &KeyEvent, &mut Tx) -> bool>,
}
impl<S: 'static> AppSpec<S> {
    pub fn new(title: &'static str, size: (f32, f32), init: fn() -> S, view: fn(&S, &mut Cx<S>) -> El) -> Self;
}
```

- `Waker` is `Arc<dyn Fn() + Send + Sync>` (`crates/rune/src/runtime/cx.rs:36`); call it from a
  worker to make `poll` run (`setup` receives it).
- `poll` returns "visible state changed", not "another frame is due". A due
  `tx.wake_after(d)` (`el/tx.rs:355`) runs `poll` even when it returned false, and draws only if
  `poll` returns true at that wake (`README` "Draw nothing when idle");
  `wake_after(Duration::ZERO)` asks for the next frame; `tx.wake_at(due)` takes a `tx.clock()`
  deadline (`:374`). Deadlines coalesce and never use a timer thread. This is the mechanism for
  hold-mode's 40 ms release probe.
- `on_key` sees the key either before anything is focused, or after the focused control passed
  it on; `tx.focused()` distinguishes (`el/tx/keys.rs:60`). `KeyEvent { key: Key, mods: Mods }`
  (`edit/keys.rs:115`); `Key` is `#[non_exhaustive]`, so every `match` needs a `_` arm.

## 6. Text input and focus

`crates/rune/src/edit.rs`, `el/input.rs:59`, `el/text.rs:389-615`.

```rust
pub fn text_input(edit: &TextEdit, style: TextStyle) -> El;   // multi-line by default

text_input(&s.shortcut_edit, rune::TextStyle::new(13.0, 500.0))
    .key("shortcut")
    .single_line()
    .placeholder("Press a shortcut", t.ink_faint)
    .on_edit(|s: &mut VoxenApp| &mut s.shortcut_edit)
```

- `TextEdit { pub text: String, pub focused: bool, .. }` (`edit.rs:43-50`), `TextEdit::new`
  (`:217`), `replace_text` (`:514`), `clear` (`:566`), `.secure()` for secrets.
- `.on_edit(|s| &mut s.field)` (`el/text.rs:605`), `.on_submit(|s, tx| ..)` (`:615`),
  `.on_input_change(|s, tx| ..)` (`el/input.rs:192`), `.on_input_key(|s, ev, tx| -> bool)`
  (`:216`), `.single_line()` (`el/text.rs:389`), `.placeholder(text, color)` (`:446`),
  `.selection_color(color)` (`:598`).
- **Focus** is runtime-owned: set `edit.focused = true` once to request focus, then let the
  runtime clear it; do not force it on every rebuild (README "Text fields"). A handler drops
  focus with `tx.blur()` (`el/tx.rs:496`). A view built only once it can read a rect still takes
  the focus it asks for in that commit.
- Key routing for a focused field (README "Key routing"): Return goes to `on_submit`, Tab moves
  focus, Escape dismisses modals — so shortcut capture must run before/around those, and while
  capturing, do not keep a text field focused unless you want it to eat the chord.

## 7. dirs and app icon

`crates/rune/src/dirs.rs:40`, `platform/app_icon.rs:26`.

```rust
pub struct AppDirs;                                     // Clone + Debug + PartialEq + Eq
impl AppDirs {
    pub fn new(app_id: &str) -> Option<AppDirs>;        // dirs.rs:53
    pub fn from_env(app_id: &str, var: impl Fn(&str) -> Option<OsString>) -> Option<AppDirs>; // :59
    pub fn rooted(root: impl Into<PathBuf>, app_id: &str) -> AppDirs;  // :65
    pub fn id(&self) -> &str;
    pub fn config(&self) -> &Path;   // %APPDATA%\<id>
    pub fn data(&self) -> &Path;     // %APPDATA%\<id>
    pub fn cache(&self) -> &Path;    // %LOCALAPPDATA%\<id>\Cache
    pub fn state(&self) -> &Path;    // %LOCALAPPDATA%\<id>
    pub fn logs(&self) -> &Path;     // %LOCALAPPDATA%\<id>\Logs
    pub fn home(&self) -> Option<&Path>;
    pub fn downloads(&self) -> Option<&Path>;
}
pub const ROOT_ENV: &str = "RUNE_DIRS_ROOT";            // moves every folder under one root
pub fn set_app_icon(bytes: impl Into<Vec<u8>>) -> Result<(), String>;   // PNG/JPEG
```

- `AppDirs` never creates folders; whoever writes makes them. The id is sanitized for the
  filesystem (`dirs.rs:169`). `Voxen`'s old Tauri config location is *not* reused automatically;
  `src-tauri/src/lib.rs:213` resolves `AppDirs::new(APP_ID)`.
- `set_app_icon` on Windows decodes the bytes, resizes to 32 and 256 px, and applies them to
  windows created **after** the call, so call it before `run`
  (`platform/app_icon.rs:16-43`). The exe icon itself still comes from `build.rs`/`winres`.
  Windows also offers `rune::set_app_icon_resource(id)` to reuse the exe's own icon resource
  (`:50`), avoiding the decode at launch.

## 8. `[patch.crates-io]` requirements

Cargo does not inherit patches from a dependency, so the app must patch every vendored crate at
the same `rev` (Rune root `Cargo.toml` `[patch.crates-io]`; README "Depending on Rune"). At this
rev those are `wgpu-hal`, `winit`, `parley_engine`, `accesskit_windows`, `accesskit_macos`.
On a Windows-only app the first four are the required set (already present in
`src-tauri/Cargo.toml`); `accesskit_macos` is not compiled there. Pin all of them, and `rune`
itself, to `eb5ae6c9f77b1a0051d8e1e5ee936ebea086bb90`; bump them together.

## 9. Packaging an external Rune app

- No bundler, no Node, no Tauri CLI. One `cargo build --release --manifest-path
  src-tauri/Cargo.toml` produces a self-contained `Voxen.exe`: fonts and shaders are compiled in
  (`embedded-fonts` and the uber shaders are default features of `crates/rune/Cargo.toml`), and
  the exe icon comes from `build.rs`/`winres` plus `set_app_icon`. No assets or DLLs ship beside
  the exe.
- Required environment: Windows + MSVC. `rust-toolchain.toml` pins `1.99.1`, so any
  `rustup`-style setup (for example `dtolnay/rust-toolchain@1.99.1`, or the file itself) is enough;
  `build.rs` needs a resource compiler for `winres` (present with MSVC) and `icons/icon.ico`. The
  workflow does not need `npm ci`.
- Console window: `src-tauri/src/main.rs` carries
  `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`, so release builds have no
  console; keep it, or the packaged exe flashes a console. Debug builds keep stderr, which is
  where Rune's own perf and warning lines go.
- Runtime DLLs: D3D12 comes from the OS; WGSL is compiled to DXIL through
  `d3dcompiler_47.dll` (Windows component), so nothing extra is shipped
  (`crates/rune/Cargo.toml` Windows `wgpu` features `["dx12"]`,
  `vendor/wgpu-hal/src/dx12/shader_compilation.rs:166`).
- Release assets: `Voxen.exe` plus `Voxen_<version>_x64.zip`, keeping the `v*` tag trigger and the
  GitHub release. `tauri-action`, `includeUpdaterJson` and `--bundles nsis` no longer apply
  (`.github/workflows/release.yml` is the packaging agent's file).
- Updating: the old mechanism verified a Tauri-signed `.nsis.zip` with `TAURI_SIGNING_PRIVATE_KEY`.
  There is no Rune equivalent and no signing key after the migration, so the portable exe asset is
  downloaded, checked to be a PE executable, swapped by a `.bat` helper and relaunched. Document
  that the update is therefore unsigned and unverified beyond the PE header and HTTPS.
- Optional Windows polish: `rune::set_app_id` (Linux/compositor metadata only, `tooling.rs:116`)
  and `rune::single_instance("...")` (README "Links and relaunches on Windows") if a second launch
  should reopen the running instance instead of starting a second tray icon.

## 10. Windows-specific gotchas

- **No press/release pair** from `RegisterHotKey`; see §2. Hold-to-talk = `GetAsyncKeyState`
  polling from `poll` with `wake_after(40ms)` while recording.
- **Modifier-only events** are off by default (`App::modifier_keys`), so shortcut capture cannot
  see modifier *releases* without opting in (§2).
- **AltGr** is Left Ctrl + Left Alt (`edit/keys.rs:59-63`); a global chord that requires Ctrl+Alt
  also fires for AltGr-typed characters in other apps.
- **Fast exit**: `Tx::quit`/`AppHandle::quit` end the process without unwinding; flush state
  synchronously. `on_quit` is the place; Rune strips tray icons for you.
- **Tray lifetime and DPI**: keep the `Tray` alive and feed a small (32 px) RGBA bitmap; the icon
  is created at the given size and scaled by Explorer.
- **Taskbar identity**: Rune does not set an AppUserModelID, so the taskbar groups by exe path;
  a rebuilt/updated exe keeps grouping with the old one only while the path is the same.
- **GPU**: Windows draws with wgpu's DX12 backend and picks the adapter that drives the primary
  display, else the next display's (`crates/rune/src/gpu/pick.rs:1-22`, `:112-121`).
  `force_fallback_adapter` is `false` (`pick.rs:127`), so there is no WARP fallback: a machine
  without a usable D3D12 adapter gets "no GPU adapter: ..." and no window. Overrides:
  `WGPU_ADAPTER_NAME`, `WGPU_POWER_PREF`; `RUNE_GPU_LOG=1` prints the choice. The failure reaches
  `AppHandle::window_error()`.
- **Click-through overlays** are `WS_EX_TRANSPARENT` layered tool windows above the taskbar but
  inside the work area (§4); they never activate, so they cannot steal focus from the app the
  user is dictating into.
- **Release builds hide stderr** (`windows_subsystem`), and Rune's release builds have the
  headless tooling only with the `tooling` feature; a headless flag in a release build otherwise
  fails with a clear message (`tooling.rs:231`).
- **Windows session end**: `WM_QUERYENDSESSION`/`WM_ENDSESSION` run `on_quit` before the process
  may be ended (`app.rs:451-460`), so last-chance persistence belongs there.
