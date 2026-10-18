# Voxen — application reference

Voxen is a **local Windows dictation** desktop app. You hold or toggle a global shortcut, speak into the microphone, and the transcript is pasted into whatever application had focus when recording started. Audio and recognition stay on your machine: there is no transcription server, cloud API, or Python runtime.

**Version:** 0.1.0  
**Stack:** Svelte 5, TypeScript, Vite 7, Tauri 2, Rust  
**Recognizer:** NVIDIA Parakeet TDT v3 (0.6B) via [NeMo-Speech.cpp](https://github.com/NVIDIA/NeMo-Speech.cpp) C ABI (`nemo_speech_asr_c.dll`)

**Platform:** Windows only (insertion and NeMo bindings target Windows today).

### Contents

- [What Voxen does](#what-voxen-does)
- [User workflow](#user-workflow)
- [Settings reference](#settings-reference)
- [Dictation state machine](#dictation-state-machine)
- [Live dictation](#live-dictation)
- [Architecture](#architecture)
- [Project layout](#project-layout)
- [Development](#development)
- [Testing](#testing)
- [Privacy and security](#privacy-and-security)
- [Troubleshooting](#troubleshooting)
- [Known limitations](#known-limitations)
- [Roadmap](#roadmap-from-project-intent)
- [Quick links](#quick-links)

---

## What Voxen does

| Capability | Status |
|------------|--------|
| Global shortcut (hold or toggle) | Yes |
| Microphone capture (mono, cpal) | Yes |
| Local ASR with automatic punctuation | Yes |
| Paste into foreground app (Ctrl+V batch fallback) | Yes (Windows) |
| Live interim text in target app while speaking | Yes (Windows; see [Live dictation](#live-dictation)) |
| Clipboard backup and restore | Yes (batch paste path) |
| Phrase replacements | Yes |
| Text cleanup (Raw / Light / Polished, local model) | Yes |
| System tray while settings closed | Yes |
| On-screen dictation indicator | Yes |
| Sidebar live transcript preview | Yes (`liveTranscript`) |
| Persisted settings | Yes (`config.json`) |
| Persisted transcript history | No (RAM only) |
| Streaming UI in other apps | No (live typing uses keystrokes, not app APIs) |
| Launch at login | Planned |
| ASR vocabulary boosting | Planned |
| macOS / Linux | Not supported |

The **release milestone** is: shortcut → capture → local transcription → insertion. Everything in the “Planned” row is explicitly later work.

---

## User workflow

### First-time setup

1. Install prerequisites (see [Development](#development)).
2. Download and extract the **NeMo-Speech.cpp** Windows Vulkan runtime (pinned to v0.2.0 in project docs). Keep all DLLs together. Dev layout: `runtime/nemo-speech-0.2.0-windows-x86_64-vulkan`.
3. Pull the model once with the bundled CLI:
   ```powershell
   & './runtime/nemo-speech-0.2.0-windows-x86_64-vulkan/bin/nemo-speech.exe' pull parakeet-tdt
   ```
4. In Voxen **Settings**, set:
   - **Runtime path** — SDK root or direct path to `nemo_speech_asr_c.dll`
   - **Model path** — absolute path to the local `.gguf` file
   - **Device** — e.g. `vulkan:0` for GPU, or `cpu`
   - **Microphone** — optional; default uses system default input
5. **Save changes**, then verify with the first-use checklist in the root [README](../README.md).

### Dictating into another app

1. Focus the target application (Notepad, browser, Discord, VS Code, etc.).
2. Use the configured shortcut:
   - **Hold mode:** press and hold to record; release to stop and transcribe.
   - **Toggle mode:** press once to start; press again to stop.
3. Voxen shows a small **indicator** window while recording, transcribing, or inserting.
4. While you speak, Voxen streams recognition and **types interim text** into the original foreground window when live dictation is active (see [Live dictation](#live-dictation)). If live dictation cannot start, it falls back to transcribing after you stop and pasting once with Ctrl+V.

### Settings window vs tray

- Closing the settings window **destroys** that window; dictation keeps running from the **system tray**.
- Tray menu: **Settings** (reopens settings), **Quit** (exits Voxen).
- Closing the app window does not quit the process; only **Quit** from the tray does.

### Test microphone (settings UI)

The **Test microphone** button records while Voxen is focused. The transcript appears in the sidebar and is **not** inserted elsewhere. Global dictation always targets the app that was focused when recording started.

If you switch away before insertion completes, Voxen keeps the transcript and reports that the original window is no longer active.

### Model memory

- The recognizer can stay loaded between utterances for faster follow-up dictation.
- **Unload model after** (seconds idle): set to `0` to keep the model resident; otherwise the inference thread unloads after idle timeout.
- **Unload model** button frees model memory immediately when idle.
- NeMo 0.2.0 DLLs remain loaded until Voxen exits (unloading/reloading the native runtime caused failures during verification). Restart Voxen before replacing runtime DLLs on disk.

---

## Settings reference

Settings are stored as JSON at **`config.json`** in Tauri’s app config directory:

| OS | Typical path |
|----|----------------|
| Windows | `%APPDATA%\com.voxen.app\config.json` |

Writes use a temp file (`config.new`) plus atomic rename. Invalid saved JSON or validation failures surface as an `error` phase on startup with a message to review settings.

Fields (camelCase in JSON):

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `microphone` | string or null | null | Device name from enumeration; null = system default |
| `shortcut` | string | `Ctrl+Shift+Space` | Tauri global shortcut syntax |
| `activationMode` | `"hold"` \| `"toggle"` | `"hold"` | How the shortcut starts/stops recording |
| `runtimePath` | string | `""` | NeMo runtime directory or DLL path |
| `modelPath` | string | `""` | Local GGUF model file |
| `device` | string | `vulkan:0` | `cpu` or `vulkan:<index>` |
| `replacements` | `{ from, to }[]` | `[]` | Literal substring replace on transcript (not regex); applied in array order; empty `from` rows are skipped at runtime but rejected on save |
| `unloadAfterSeconds` | number | `300` | Idle unload delay; max 86400; `0` = never auto-unload |
| `cleanupMode` | `"raw"` \| `"light"` \| `"polished"` | `"raw"` | Post-transcription cleanup pass; `raw` skips it |
| `cleanupRuntimePath` | string | `""` | `llama-server` binary or the folder containing it; required unless mode is `raw` |
| `cleanupModelPath` | string | `""` | Small local instruction GGUF for cleanup; required unless mode is `raw` |

Validation highlights:

- Empty `from` in replacements is rejected on save.
- Runtime and model paths must be set before recording (global or test).
- Shortcut cannot be changed while dictation is in progress; capture mode temporarily unregisters the global shortcut.

### Example `config.json`

```json
{
  "microphone": null,
  "shortcut": "Ctrl+Shift+Space",
  "activationMode": "hold",
  "runtimePath": "C:\\path\\to\\nemo-speech-0.2.0-windows-x86_64-vulkan",
  "modelPath": "C:\\Users\\you\\AppData\\Local\\NeMoSpeech\\models\\parakeet-tdt-0.6b-v3.q8_0.gguf",
  "device": "vulkan:0",
  "replacements": [
    { "from": "voxen", "to": "Voxen" }
  ],
  "unloadAfterSeconds": 300,
  "cleanupMode": "raw",
  "cleanupRuntimePath": "",
  "cleanupModelPath": ""
}
```

### Global shortcut behavior

The registered shortcut is handled in Rust (`tauri_plugin_global_shortcut`). While the shortcut field is focused in settings, capture mode ignores global events so rebinding does not start dictation.

| `activationMode` | Event | Current phase | Action |
|------------------|-------|---------------|--------|
| `hold` | Pressed | `idle` or `error` | Start recording |
| `hold` | Released | `recording` | Stop → transcribe → maybe insert |
| `toggle` | Pressed | `recording` | Stop → transcribe → maybe insert |
| `toggle` | Pressed | `idle` or `error` | Start recording |
| any | any | `transcribing` / `inserting` | Ignored |

Shortcut strings use [Tauri global shortcut](https://v2.tauri.app/reference/javascript/global-shortcut/) syntax (e.g. `Ctrl+Shift+Space`). The UI builds bindings from `Ctrl`, `Alt`, `Shift`, `Super`, and the key code (with `Key`/`Digit` prefixes stripped).

**Shortcut capture in settings:** Focusing the shortcut field unregisters the global binding until blur. Press the desired chord; release modifiers to finish, **Esc** restores the previous value, **Tab** is ignored. Saving while capture is active is rejected. Blurring the main window ends capture via `set_shortcut_capture(false)`.

### Phrase replacements

Replacements are simple literal `from` → `to` substitutions applied to every hypothesis (live and batch), in list order. They are not regular expressions. Use them for names, product terms, or fixing systematic misrecognitions. Empty `from` values are skipped at runtime but cannot be saved.

---

## Dictation state machine

Phases exposed to the UI via `dictation-state` events and `get_snapshot`:

```
idle ──(start)──► recording ──(stop)──► transcribing ──(insert?)──► inserting ──► idle
  ▲                    │                         │
  └──── error ◄────────┴─────────────────────────┘
```

| Phase | Meaning |
|-------|---------|
| `idle` | Ready for a new recording |
| `recording` | Microphone stream active |
| `transcribing` | NeMo ASR running on captured samples |
| `inserting` | Pasting into target window (Windows) |
| `error` | Last operation failed; message in `error` |

New recordings are allowed only in `idle` or `error`. `transcribing`, `inserting`, and `recording` block overlapping work.

`modelLoaded` reflects whether the inference worker currently holds a loaded `Recognizer` instance.

### `AppSnapshot` (UI and events)

Every `dictation-state` event and `get_snapshot` response uses the same shape (camelCase JSON):

| Field | Type | Description |
|-------|------|-------------|
| `config` | `Config` | Current settings (see table above) |
| `phase` | string | One of `idle`, `recording`, `transcribing`, `inserting`, `error` |
| `error` | string or null | Human-readable message when `phase` is `error` |
| `lastTranscript` | string or null | Most recent final transcript after replacements (sidebar preview) |
| `liveTranscript` | string or null | Combined final + interim text during recording when the live stream is active; cleared when the utterance finishes |
| `modelLoaded` | boolean | Whether the inference thread still holds a loaded NeMo recognizer |

The Svelte store in `src/lib/state.svelte.ts` mirrors this snapshot for reactive UI updates.

### Edge cases

- **Empty audio** — NeMo returns an error (`No audio was captured`); phase becomes `error`.
- **Whitespace-only transcript** — Replacements run; batch paste is skipped when trimmed text is empty (`foreground != 0` required for any insertion).
- **Live dictation unavailable** — If `StartLive` fails (model load or `Recognizer::stream`), the snapshot may show an error while recording continues with no live session; audio still accumulates. On stop, `live` is absent so the **batch** `transcribe` + `insert_text` path runs instead.
- **Live dictation failure mid-utterance** — A stream or `replace_text` error is stored on the live session; finishing the recording surfaces that error instead of silently falling back.
- **Voxen settings focused at record start** — Foreground HWND is cleared (`foreground = 0`); transcription still runs and `lastTranscript` updates, but nothing is pasted (same as test microphone from the sidebar).
- **Target app loses focus before paste** — Insertion aborts with: *The original window is no longer active; transcript was preserved without insertion*.
- **Modifiers still held** — After up to 2s wait: *Release Ctrl, Shift, Alt, and Windows keys before inserting the transcript*.
- **Save while busy** — Settings changes are rejected until phase is `idle` or `error`; saving also sends `Unload` so the next utterance reloads the model with new paths/device.

---

## Live dictation

During **recording**, Voxen tries to run NeMo’s **streaming** ASR in parallel with microphone capture:

1. `start_recording` queues `InferenceCommand::StartLive` with the foreground HWND captured at record start (same rules as batch insertion: `0` when Voxen’s main window is focused).
2. The audio worker forwards new mono samples to the inference thread every ~100 ms (`forward_audio` → `LiveAudio`).
3. On the first chunk, the inference worker loads the model if needed and opens a `nemo::Stream`.
4. Each `stream.push` is followed by `pump_live`, which reads interim/final hypotheses from `stream.next`.
5. For each update, `apply_replacements` runs, then:
   - **Final** segments append to the session’s accumulated `text`.
   - **Interim** text is the latest non-final hypothesis for the current phrase.
   - If `foreground != 0`, `insertion::replace_text` revises what is already in the target field (backspace shared prefix, type the suffix, Unicode-aware).
6. `liveTranscript` in the snapshot is the visible combination of finalized text plus current interim text (for the settings sidebar).
7. On **stop**, `Transcribe` finishes the stream. If a live session was active, it sets `lastTranscript`, clears `liveTranscript`, and **does not** run batch `insert_text` (the target app already has the text). If live never started, the worker uses one-shot `Recognizer::transcribe` and `insert_text` as before.

Live typing temporarily releases held modifier keys (Ctrl/Shift/Alt/Win), sends keystrokes, then restores modifiers so shortcut chords do not block insertion.

**Test microphone** uses the same pipeline but with `foreground = 0`, so you see `liveTranscript` / `lastTranscript` in the sidebar only.

---

## Text cleanup

After batch transcription (and after replacements), Voxen can run an optional cleanup pass with a small local instruction model. Live interim text is never cleaned; only the final transcript is.

| Mode | Effect |
|------|--------|
| `raw` | No cleanup; transcript is kept unchanged |
| `light` | Remove only the filler interjections um/uh and resolve an adjacent same-sentence self-correction (`Tuesday, no wait, Wednesday` → `Wednesday`) |
| `polished` | Light cleanup plus punctuation, capitalization, sentence splits, and numbered-list markers only when every listed word is kept and no conjunction is dropped |

**Runtime:** Voxen manages a warm local `llama-server` child process (no FFI, no cloud, localhost only). The server starts lazily on the first cleanup with a fixed 2048-token context, reasoning disabled, and greedy decoding, using the configured port only. It makes a single bounded startup attempt (60s); a failed start keeps the original transcript and reports an error. A running server is reused across utterances with no per-call model poll, and stopped when settings are saved, the model is unloaded, or Voxen exits. Changing the cleanup runtime or model path unloads the previous server.

**Pinned setup (verified October 18, 2026):** `llama-server` from the llama.cpp Vulkan backend (2.51.0) with `Qwen3.5-0.8B` rejected as too weak (it either echoed fillers or dropped hedging) and `Qwen3.5-2B-Q4_K_S` (~1.2 GB) pinned instead: it removes fillers, resolves adjacent corrections, and preserves names, numbers, negations, and commands. Set:

- **Cleanup runtime path** — `llama-server.exe` or its folder, e.g. the LM Studio backend folder `llama.cpp-win-x86_64-vulkan-avx2-2.51.0`
- **Cleanup model path** — e.g. `Qwen3.5-2B-Q4_K_S.gguf`

**Safety guard:** every model reply passes an ordered lexical check before it replaces the transcript. Only clear um/uh deletions, narrow comma-delimited edge fillers (er/ah/basically), single-word adjacent same-sentence self-correction resolutions, punctuation/case changes, and numbered-list markers are allowed, and fillers may only be removed, never added; ambiguous phrases (you know, sort of, kind of, like), sentence-initial So, names (Ah, Er), conjunctions (and/or), negations, numbers, symbols, paths, and commands are preserved exactly, including hyphens, slashes, dots, and other symbols inside words. Any other added, dropped, or swapped word (added fillers, extra negations, changed numbers, swapped names or days, cross-sentence or trailing-cue corrections, truncation, hallucinations) rejects the reply. Rejected and failed cleanups keep the original transcript and show a warning in the sidebar (`Cleanup kept the original transcript: …`). Requests are bounded to 2000 input characters with a 30-second timeout; over-long or cut-off replies are rejected the same way.

---

## Architecture

### High-level data flow

```
Global shortcut / UI "Test microphone"
        │
        ▼
  start_recording ──► capture foreground HWND ──► StartLive (inference)
        │                    │
        ▼                    ▼
  audio thread (cpal) ──► mono f32 ──► LiveAudio chunks ──► nemo::Stream
        │                              │
        │                              ├──► apply_replacements
        │                              ├──► replace_text (if foreground ≠ 0)
        │                              └──► liveTranscript in snapshot
        ▼
  stop_recording ──► Transcribe job
        │
        ├── live session? ──► stream.finish ──► lastTranscript, no batch paste
        │
        └── else ──► Recognizer::transcribe ──► insert_text (Ctrl+V + clipboard restore)
        │
        ▼
  dictation-state event ──► Svelte UI + indicator window
```

### Process structure (Rust)

- **Main Tauri thread:** commands, global shortcut handler, tray, window lifecycle.
- **Audio worker thread:** owns the cpal input stream; handles `Start` / `Stop` / stream errors.
- **Inference worker thread:** owns optional `nemo::Recognizer` and optional live `Live` session; handles `StartLive`, `LiveAudio`, `Transcribe`, and `Unload`; implements idle unload deadline.

Shared state: `AppState` → `Backend` (mutex) with snapshot, channel senders, captured foreground HWND, shortcut-capture flag.

### Worker message protocol

The main thread never blocks on mic I/O or NeMo inference. It sends commands on two unbounded `std::sync::mpsc` channels:

**Audio worker (`AudioCommand`)**

| Message | Payload | Effect |
|---------|---------|--------|
| `Start` | optional mic name, reply channel | Open cpal input stream, reset buffer |
| `Stop` | reply channel | Stop stream, forward tail audio, return `(samples, sample_rate)` |
| `Failed` | error string | Tear down stream, set phase `error` |

While a stream is active, the audio worker wakes every **100 ms** (recv timeout) to call `forward_audio` even if no new command arrived.

**Inference worker (`InferenceCommand`)**

| Message | Payload | Effect |
|---------|---------|--------|
| `StartLive` | `Config`, foreground HWND | Load model if needed, open `nemo::Stream`, reset live session |
| `LiveAudio` | `Vec<f32>`, sample rate | `stream.push` + `pump_live` when a session exists |
| `Transcribe` | `Config`, samples, rate, HWND | Finish live stream **or** batch transcribe + optional `insert_text` |
| `Unload` | — | Drop live session and recognizer; clear idle deadline; `modelLoaded = false` |

After a successful `Transcribe`, if `unload_after_seconds > 0` and a model remains loaded, the worker sets a recv timeout to `Unload` when idle time elapses.

### Application lifecycle

**Startup (`setup` in `lib.rs`):**

1. Spawn audio and inference worker threads.
2. Load `config.json` from the app config directory if present; invalid JSON or failed validation sets phase `error` with a settings message.
3. Register the global shortcut from saved config; registration failure surfaces as `error` (choose another binding in settings).
4. Create the system tray with **Settings** and **Quit**; tooltip: `Voxen — local dictation`.

**While running:**

- `RunEvent::ExitRequested` with no exit code is **prevented** so closing the last window does not quit the process; only tray **Quit** calls `app.exit(0)`.
- Closing the main settings window **prevents** default close, ends shortcut-capture if active, and **destroys** the window. Dictation and the tray keep running; **Settings** from the tray recreates the main window from `tauri.conf.json` if needed.
- The indicator webview is created on demand during `recording` / `transcribing` / `inserting` and destroyed when phase returns to `idle` or `error`.

**Saving settings:**

- Atomic write via `config.new` → rename to `config.json`.
- Shortcut is re-registered only when the binding changed or was missing.
- An `Unload` is always sent to the inference thread so path/device changes take effect on the next utterance.
- If writing `config.json` fails after a new shortcut was registered, the previous shortcut is re-registered and the new one is unregistered so the app does not stay on a broken binding.

### Audio capture

- **Library:** [cpal](https://github.com/RustAudio/cpal) on the default host.
- **Device:** Named microphone from settings, or system default input.
- **Format:** Device default stream config; samples supported as **F32**, **I16**, or **U16** (other formats error at stream build).
- **Channels:** Multi-channel input is averaged to **mono** per frame (`append_mono`).
- **Buffer:** Samples accumulate in a `Vec<f32>` until `Stop`; sample rate is passed through to NeMo as captured.

Mic stream failures post `AudioCommand::Failed` to the audio worker, clear the stream, and set phase `error`.

### Frontend (Svelte 5)

| Path | Role |
|------|------|
| `src/App.svelte` | Settings shell, test record, form |
| `src/lib/Indicator.svelte` | Minimal overlay UI (`?indicator=1`) |
| `src/lib/Replacements.svelte` | Replacement rows editor |
| `src/lib/tauri.ts` | `invoke` wrappers and event listener |
| `src/lib/types.ts` | Shared TypeScript types |
| `src/lib/state.svelte.ts` | Reactive snapshot store |

The same `index.html` serves the main window and the indicator via query string.

**Bootstrap:** `src/main.ts` mounts `App.svelte` with global styles from `src/app.css`. The Rust binary is a thin `main.rs` that calls `voxen_lib::run()`.

**Theming:** Dark UI via `color-scheme: dark` and CSS variables in `app.css` (`--bg`, `--surface`, `--accent`, `--danger`, etc.). Segoe UI stack; main window uses Tauri `theme: "Dark"`.

**Main window layout**

- **Sidebar:** Brand, phase chip, model-loaded indicator, **Test microphone** button, errors, optional **Live** block (`liveTranscript` while streaming), and **Last transcription** (`lastTranscript`).
- **Content form:** Microphone, Activation (shortcut + hold/toggle), Local model (paths, device, idle unload + **Unload model**), Replacements table, sticky **Save changes**.
- **Browser-only (`npm run dev`):** Stub card explaining that mic, shortcuts, and tray require `npm run tauri dev`.

**Indicator window** (`190×44`, bottom-center of primary monitor, frameless, always on top, click-through): shows phase label with a colored dot; destroyed when phase returns to `idle` or `error`.

### Native modules (Rust)

| Module | Role |
|--------|------|
| `src-tauri/src/lib.rs` | Tauri app, workers, config, commands |
| `src-tauri/src/nemo.rs` | Dynamic load of `nemo_speech_asr_c.dll`, C ABI bindings, transcribe |
| `src-tauri/src/cleanup.rs` | Managed `llama-server` child process, bounded chat requests, lexical safety guard |
| `src-tauri/src/insertion.rs` | Windows `insert_text` (clipboard + Ctrl+V) and `replace_text` (live Unicode keystrokes) |

### Tauri IPC commands

| Command | Purpose |
|---------|---------|
| `get_snapshot` | Current `AppSnapshot` |
| `list_microphones` | Sorted input device names |
| `save_config` | Validate, atomic write `config.json`, refresh shortcut, unload model |
| `set_shortcut_capture` | Pause global shortcut while user picks a new binding |
| `start_recording` | Capture foreground HWND (unless Voxen main), start mic |
| `stop_recording` | Stop mic, queue transcription job |
| `unload_model` | Drop recognizer in inference thread |

Events:

- `dictation-state` — payload is full `AppSnapshot` after meaningful state changes.

### Text insertion (Windows)

Two paths:

**Batch (`insert_text`)** — used when live dictation did not run for the utterance:

1. Verify the target HWND is still foreground.
2. Wait up to 2s for modifier keys (Ctrl/Shift/Alt/Win) to release after shortcut use.
3. Open clipboard in context of target window; duplicate all formats for backup.
4. Set Unicode text, send Ctrl+V via `SendInput`, brief sleep.
5. If clipboard sequence number unchanged, restore all backed-up formats; otherwise assume the target app consumed clipboard and skip restore.

**Live (`replace_text`)** — used during streaming when `foreground != 0`:

1. Verify foreground unchanged.
2. Release any held modifiers, backspace characters not shared with the previous hypothesis prefix, type the new suffix with `KEYEVENTF_UNICODE`, restore modifiers.
3. Common-prefix optimization avoids retyping unchanged characters.

Privilege note: injection cannot reliably reach apps running at a higher integrity level. Run Voxen and the target app at the same privilege level. Apps that do not treat backspace/Unicode input like a normal text field may behave poorly with live dictation.

### NeMo integration

- DLL resolved from runtime directory: `bin/nemo_speech_asr_c.dll` or root.
- Loaded with `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS`.
- GPU index: `vulkan:N` → `N`; `cpu` → `-1`.
- Recognition uses default options with `enable_automatic_punctuation = true`.
- **Batch path:** `nemo_speech_asr_recognize_f32` via `Recognizer::transcribe` when live streaming did not handle the utterance.
- **Streaming path (dictation):** `nemo_speech_asr_streaming_recognize`, `stream_push_f32`, `stream_finish`, and `stream_next` via `Recognizer::stream()` during recording. Options use `enable_automatic_punctuation = true` for both paths.

Exported C symbols loaded from `nemo_speech_asr_c.dll` (see `nemo.rs`):

| Symbol | Rust wrapper |
|--------|----------------|
| `nemo_speech_asr_create` / `nemo_speech_asr_destroy` | `Recognizer::load`, drop |
| `nemo_speech_asr_recognize_f32` | `Recognizer::transcribe` |
| `nemo_speech_asr_streaming_recognize` | `Recognizer::stream` |
| `nemo_speech_asr_stream_push_f32` | `Stream::push` |
| `nemo_speech_asr_stream_finish` | `Stream::finish` |
| `nemo_speech_asr_stream_next` | `Stream::next` |
| `nemo_speech_asr_stream_close` | `Stream` drop |
| `nemo_speech_asr_result_*` | Read transcript / `is_final` from results |
| `nemo_speech_asr_options_default` | Punctuation-enabled options |
| `nemo_speech_asr_last_error` | Error strings after failed calls |

Model license: [CC-BY-4.0](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3) for the pinned Parakeet model.

**Pinned runtime:** NeMo-Speech.cpp **v0.2.0** Windows Vulkan x86_64. The CLI `nemo-speech.exe pull parakeet-tdt` downloads the matching GGUF (Q8 variant used in project verification).

---

## Project layout

```
Voxen/
├── docs/
│   └── APP.md            # Full application reference (this file)
├── src/                  # Svelte frontend
│   ├── App.svelte
│   ├── main.ts
│   ├── app.css
│   └── lib/              # tauri IPC, types, state, Indicator, Replacements
├── src-tauri/            # Rust / Tauri backend
│   ├── src/main.rs       # Entry → voxen_lib::run()
│   ├── src/lib.rs        # App state, workers, Tauri commands
│   ├── src/nemo.rs       # NeMo DLL bindings
│   ├── src/insertion.rs  # Windows paste + live typing (stubs on other OS)
│   ├── Cargo.toml
│   └── tauri.conf.json
├── runtime/              # Local NeMo runtime (optional in repo); JFK sample WAV for tests
├── dist/                 # Vite production build (generated)
├── package.json
├── vite.config.ts        # Dev server port 1420
├── AGENTS.md             # Contributor / agent editing principles
└── README.md             # Quick start and verification notes
```

Installer: `npm run tauri build -- --bundles nsis`. Runtime and model files are **not** bundled; users configure paths after install.

---

## Development

### Prerequisites

- Node.js
- Rust (MSVC toolchain)
- Visual Studio C++ Build Tools
- WebView2

### Commands

| Command | Purpose |
|---------|---------|
| `npm install` | Frontend dependencies |
| `npm run tauri dev` | Full native app with hot reload |
| `npm run dev` | Vite only — **no** mic or global shortcuts |
| `npm run check` | `svelte-check` |
| `npm run build` | Production frontend to `dist/` |
| `cargo check --manifest-path src-tauri/Cargo.toml` | Rust compile check |
| `cargo test --manifest-path src-tauri/Cargo.toml` | Unit tests |

### Dependencies

| Layer | Packages |
|-------|----------|
| Frontend | `svelte` 5, `vite` 7, `@tauri-apps/api` 2, TypeScript 5 |
| Rust | `tauri` 2 (tray), `tauri-plugin-global-shortcut` 2, `serde`/`serde_json`, `cpal` 0.15, `libloading` 0.8 |
| Windows only | `windows` 0.58, `clipboard-win` 5 |

External (not npm/cargo): NeMo-Speech.cpp runtime DLLs and Parakeet GGUF on disk.

### Tauri configuration

- App ID: `com.voxen.app`
- Main window: 900×680, dark theme
- `beforeDevCommand`: `npm run dev` at `http://localhost:1420`
- Tray icon enabled; exit on last window close is **prevented** so tray keeps running

### Agent / contribution notes

See [AGENTS.md](../AGENTS.md) for editing principles (minimal diffs, no drive-by refactors, tests only when meaningful).

---

## Testing

### Always-on Rust tests

| Test | Module | What it checks |
|------|--------|----------------|
| `stereo_capture_is_averaged_to_mono` | `lib.rs` | Multi-channel mic samples average to mono |
| `empty_replacements_are_rejected_and_cannot_expand_text` | `lib.rs` | Save validation and runtime skip for empty `from` |
| `transcribing_and_inserting_block_new_recordings` | `lib.rs` | `is_available` only allows `idle` and `error` |
| `live_edits_only_replace_the_changed_suffix` | `insertion.rs` | Common-prefix logic for live typing |
| `cleanup` guard tests | `cleanup.rs` | Clear-filler/correction acceptance; conjunction, symbol/number, ambiguous-filler, name, negation, day-swap, cross-sentence-correction, truncation, and hallucination rejection |
| `live_edits_keep_whole_surrogate_pairs` | `insertion.rs` | Unicode surrogate-safe backspace boundaries |

### Opt-in integration tests

Require local runtime, model, GPU, or interactive clipboard:

```powershell
$env:VOXEN_RUNTIME = (Resolve-Path runtime/nemo-speech-0.2.0-windows-x86_64-vulkan).Path
$env:VOXEN_MODEL = 'C:\path\to\parakeet-tdt-0.6b-v3.q8_0.gguf'
$env:VOXEN_CLEANUP_RUNTIME = 'C:\path\to\llama.cpp-win-x86_64-vulkan-avx2-2.51.0'
$env:VOXEN_CLEANUP_MODEL = 'C:\path\to\Qwen3.5-2B-Q4_K_S.gguf'
cargo test --manifest-path src-tauri/Cargo.toml real_runtime_transcribes_sample -- --ignored
cargo test --manifest-path src-tauri/Cargo.toml real_runtime_cleans_spoken_text -- --ignored
cargo test --manifest-path src-tauri/Cargo.toml real_runtime_streams_sample_incrementally -- --ignored
cargo test --manifest-path src-tauri/Cargo.toml rich_and_empty_clipboards_are_restored -- --ignored
cargo test --manifest-path src-tauri/Cargo.toml paste_reaches_a_windows_edit_control -- --ignored
cargo test --manifest-path src-tauri/Cargo.toml live_typing_revises_unicode_without_duplicating -- --ignored
```

`real_runtime_streams_sample_incrementally` feeds the JFK sample in small chunks and asserts interim streaming results appear before the final transcript (NeMo C ABI only).

`live_typing_revises_unicode_without_duplicating` drives `replace_text` through progressive hypotheses into a native `EDIT` control, including Unicode text.

Documented verification (October 18, 2026) on the primary dev machine: Vulkan + RX 9060 XT, JFK sample transcription, C ABI reload, native shortcut/mic tests, clipboard restore tests. Cross-app spoken dictation in Discord, VS Code, and browsers still needs manual interactive validation.

---

## Privacy and security

- Audio samples exist in RAM during capture and inference; they are not written to disk by Voxen.
- Transcripts live in memory (`lastTranscript` in snapshot) until replaced or the app exits.
- Settings file contains paths and preferences only, not audio.
- No network calls are required for dictation once runtime and model are local. Text cleanup talks only to a `llama-server` child process on localhost.

---

## Troubleshooting

| Symptom | Likely cause | What to do |
|---------|----------------|------------|
| Shortcut does nothing | Browser-only dev, or shortcut not registered | Run `npm run tauri dev`; open Settings and pick another shortcut if startup reported registration failure |
| *Set and save the local runtime and model paths first* | Paths empty in saved config | Fill **Runtime path** and **Model path**, **Save changes** |
| *Failed to load … nemo_speech_asr_c.dll* | Wrong runtime path or DLLs split | Point at SDK root or DLL; keep NeMo `bin` DLLs together |
| *No microphone is available* | No input device or wrong name | Refresh mic list; use system default or reconnect device |
| Transcript in sidebar only | Recording started while Voxen was focused | Focus target app before using the global shortcut |
| Paste missing in target app | Focus changed, live disabled and batch skipped, or higher integrity target | Keep target focused; run Voxen and target at same privilege level |
| Garbled or duplicated live text | Target app handles keys differently | Try another editor; batch fallback applies only if streaming never started |
| Clipboard not restored | Target consumed clipboard (sequence number changed) | Expected for some apps; backup skipped intentionally |
| Model slow first time, fast after | Cold load vs resident recognizer | Lower **Unload model after** or set `0`; use **Unload model** to free RAM |
| *llama-server was not found…* | Wrong cleanup runtime path | Point at `llama-server.exe` or its folder; keep the backend DLLs together |
| *The cleanup model was not found* | Wrong cleanup model path | Point at the local GGUF file |
| *Cleanup kept the original transcript…* | Guard rejected the model reply (or the request timed out) | Original text was preserved; try Light mode or a shorter utterance |
| Settings invalid on startup | Hand-edited `config.json` | Fix JSON/values in UI and save again |

---

## Known limitations

- **Windows only** for paste insertion; other platforms would need alternate insertion strategies.
- **Paste-based batch fallback** — apps without standard Ctrl+V behavior may need future app-specific formatters.
- **Live keystroke insertion** — revises text via backspace + Unicode typing; exotic inputs (vim, remote desktop, custom widgets) may not work reliably.
- **Foreground capture** — focus must remain on the target through insertion for paste to succeed.
- **NeMo DLL lifetime** — full native unload of DLLs is avoided; restart app after runtime updates.
- **Browser-only dev** — `npm run dev` cannot exercise native features.

---

## Roadmap (from project intent)

Not implemented yet; listed for context:

- Launch at login
- ASR vocabulary boosting
- Persisted dictation history
- Further streaming polish (non–text-field apps, latency tuning)
- Application-specific formatting
- Performance tuning beyond current idle model retention

Phrase replacements, offline local inference, and the local text cleanup pass are the main post-processing surface today.

---

## Glossary

| Term | Meaning |
|------|---------|
| ASR | Automatic speech recognition (Parakeet via NeMo-Speech.cpp) |
| NeMo runtime | Extracted `nemo-speech-*` SDK with `nemo_speech_asr_c.dll` and CLI |
| Utterance | One record cycle from shortcut start to stop |
| Foreground HWND | Windows window handle captured when recording starts; insertion target |
| Phase | Coarse dictation state: `idle`, `recording`, `transcribing`, `inserting`, `error` |
| Live session | Open NeMo streaming recognizer for the current utterance |

---

## Quick links

- Root [README](../README.md) — setup, first-use check, build installer
- [NeMo-Speech.cpp v0.2.0 release](https://github.com/NVIDIA/NeMo-Speech.cpp/releases/tag/v0.2.0)
- [Parakeet TDT 0.6B v3 model card](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3)
