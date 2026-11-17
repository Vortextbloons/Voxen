# Rune migration contracts

Pinned Rune revision: `eb5ae6c9f77b1a0051d8e1e5ee936ebea086bb90`
(`rune = { git = "https://github.com/anomalyco/rune", rev = "..." }`).
Read `.rune-docs/rune-README.md` and `.rune-docs/rune-kit-README.md` before writing UI code.
Do not add dependencies. Prefer `rune::tok`, `rune::el`, `rune::text`, `rune::text_input`.

## Ownership

- `src-tauri/src/lib.rs` — backend, workers, config, shortcuts, tray, engine (owned by lead).
- `src-tauri/src/ui.rs` — settings window state and view (UI agent).
- `src-tauri/src/theme.rs` — colour/type tokens ported from `src/app.css` (UI agent).
- `src-tauri/src/widgets.rs` — shared controls for the settings window (UI agent).
- `src-tauri/src/indicator.rs` — recording indicator overlay (indicator agent).
- `src-tauri/src/updater.rs` — GitHub release check/install (updater agent).
- `.github/workflows/release.yml`, `src-tauri/build.rs`, `packaging/` — packaging agent.
- `docs/RUNE_MIGRATION_NOTES.md` — research agent only.

Never edit another owner's files. Do not edit `src/` Svelte files, `src/app.css`,
`src-tauri/src/cleanup.rs`, `src-tauri/src/nemo.rs`, `src-tauri/src/normalize.rs`,
`src-tauri/src/insertion.rs`, `bench/`, or `src-tauri/examples/`.

## Backend API (`crate`)

`Config`, `Replacement`, `AppSnapshot` are `pub`, `Clone`, fields `pub`, camelCase serde.

```rust
impl Engine {
    pub fn snapshot(&self) -> AppSnapshot;
    pub fn revision(&self) -> u64;
    pub fn list_microphones() -> Result<Vec<String>, String>;
    pub fn save_config(&self, config: Config) -> Result<AppSnapshot, String>;
    pub fn set_shortcut_capture(&self, active: bool) -> Result<(), String>;
    pub fn start_recording(&self) -> Result<(), String>;
    pub fn stop_recording(&self) -> Result<(), String>;
    pub fn unload_model(&self) -> Result<(), String>;
    pub fn open_window(&self);
    pub fn hide_window(&self);
    pub fn quit(&self);
    pub fn handle_shortcut(&self); // pressed; starts/stops per activation mode
    pub fn shortcut_down(&self) -> bool; // hold-mode release probe
}
```

`Engine` is `Clone + Send + Sync`. UI calls it from handlers and `poll`; each call
updates the snapshot and wakes the UI. `Engine::snapshot().phase` is one of
`idle | starting | recording | transcribing | inserting | error`.

## Settings state (`ui.rs`)

The app state type is `crate::ui::VoxenApp`; `lib.rs` constructs it with
`VoxenApp::new(engine)`.

Required functions used by `lib.rs`:

```rust
impl VoxenApp {
    pub engine: crate::Engine;
    pub fn new(engine: crate::Engine) -> Self;
    pub fn view(&self, cx: &mut rune::Cx<Self>) -> rune::El;
    pub fn poll(&mut self, clock: f64, tx: &mut rune::Tx) -> bool;
    pub fn on_key(&mut self, ev: &rune::KeyEvent, tx: &mut rune::Tx) -> bool;
    pub fn on_shortcut(&mut self, tx: &mut rune::Tx);
    pub fn on_close(&mut self, tx: &mut rune::Tx);
    pub fn on_focus(&mut self, focused: bool, tx: &mut rune::Tx);
}
```

`on_shortcut` calls `self.engine.handle_shortcut()`. `on_close` ends shortcut
capture with `self.engine.set_shortcut_capture(false)` and ignores errors.
`on_focus(false)` does the same; gaining focus does nothing.
```

`poll` must:
- check for updates once when the app starts (`updater.check()` from `new` or the
  first `poll`), and let the Updates tab check again;
- drain `Updater` (`updater.poll()`), returning true when its visible state changed;
- quit through `tx.quit()` once `updater.status() == UpdateStatus::Ready`
  (the helper replaces the exe after this process exits);
- keep hold-mode working: while `phase == "starting" | "recording"` and
  `activation_mode == "hold"`, call `engine.stop_recording()` when
  `engine.shortcut_down()` is false, and `tx.wake_after(Duration::from_millis(40))`;
- return true when the draft/snapshot/updater/visible state changed.

`on_key` must implement shortcut capture:
- focus/capture starts with `engine.set_shortcut_capture(true)`, stores the old
  shortcut, captures modifiers plus a non-modifier physical key using
  `rune::Shortcut::from_physical_key(tx.physical_key()?, ev.mods)`, ignores Tab,
  Escape restores the old shortcut and ends capture, and ends capture once a chord is
  chosen (or when modifiers are all released; the app enables `modifier_keys`).
  Call `tx.blur()` when capture ends.
- Return true for keys consumed by capture.

## Theme (`theme.rs`)

Port exactly the palette from `src/app.css`: bg `#17191d`, surface `#202328`,
surface-soft `#131519`, surface-hover `#292d34`, ink `#f0f1f3`, ink-soft `#b4bac4`,
ink-faint `#969eab`, line `#30343c`, line-strong `#454c58`, accent `#648cf5`,
accent-hover `#7b9ef8`, accent-soft `#26334e`, accent-cyan `#2fb4ff`,
accent-blue `#3b82f6`, accent-violet `#9b5cff`, danger `#f87171`,
danger-soft `#2a181c`, ok `#4ade80`, radius `10.0`.
Use `rune::color::hex(..).unwrap()` or `rune::oklch` for overlays. Use Inter (Rune's
default) and a compact scale (11–21 pt).

## Indicator (`indicator.rs`)

```rust
pub struct Indicator { pub phase: String, pub error: bool }
impl Indicator { pub fn new() -> Self; pub fn set(&mut self, phase: &str, error: bool); }
pub fn view(s: &Indicator, cx: &mut rune::Cx<Indicator>) -> El;
```
A 220×76 transparent overlay, bottom-centre, click-through, modern minimalist:
dark translucent pill, cyan/violet hairline, mic glyph or dot, phase label.
Recording animates only through `rune_motion` loops or a live dot; reduced motion
must not break. Text must fit at 1× and 2×.

## Updater (`updater.rs`)

```rust
pub struct Updater { /* private */ }
pub enum UpdateStatus { Idle, Checking, Current, Available, Installing, Ready, Error }
impl Updater {
    pub fn new() -> Self;
    pub fn current_version() -> &'static str;
    pub fn status(&self) -> &UpdateStatus;
    pub fn info(&self) -> Option<&UpdateInfo>; // version, notes
    pub fn progress(&self) -> u8;
    pub fn error(&self) -> Option<&str>;
    pub fn check(&mut self);              // spawns a worker
    pub fn install(&mut self);            // spawns a worker; quits/replaces on success
    pub fn poll(&mut self) -> bool;       // drains worker events; true when visible state changed
}
```

Update source: GitHub releases for `Vortextbloons/Voxen`. Package format after this
migration is a portable `Voxen.exe` asset (plus zip). Install downloads to
`%TEMP%/voxen-update`, verifies the asset is a PE executable, writes a `.bat`
swap-and-relaunch helper, runs it hidden, and quits the current process. No
signature check is available without the old Tauri signer; document that in
`docs/RUNE_MIGRATION_NOTES.md` if you cannot preserve it.

## Packaging

- `src-tauri/build.rs`: on Windows set the exe icon from `icons/icon.ico` with
  `winres` (build-dependency). Do not use `tauri-build`.
- Release workflow: install Rust 1.99.1, `cargo build --release --manifest-path
  src-tauri/Cargo.toml`, publish `Voxen.exe` and `Voxen_<version>_x64.zip`,
  no Tauri action, no updater signing. Keep the tag trigger and GitHub release.
