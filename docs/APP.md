# Voxen — application reference

Voxen is a **local Windows dictation** desktop app. You hold or toggle a global shortcut, speak into the microphone, and the transcript is pasted into whatever application had focus when recording started. Audio and recognition stay on your machine: there is no transcription server, cloud API, or Python runtime.

**Version:** 0.1.0
**Stack:** Rust 2021, Rune 0.0.1 (pinned git revision `eb5ae6c9f77b1a0051d8e1e5ee936ebea086bb90`), single native Windows process
**Recognizer:** NVIDIA Nemotron 3.5 multilingual, Nemotron English, or Parakeet TDT v3 via [NeMo-Speech.cpp](https://github.com/NVIDIA/NeMo-Speech.cpp) C ABI (`nemo_speech_asr_c.dll`)

**Platform:** Windows only (insertion and NeMo bindings target Windows today).

### Contents

- [What Voxen does](#what-voxen-does)
- [User workflow](#user-workflow)
- [Settings reference](#settings-reference)
- [Dictation state machine](#dictation-state-machine)
- [Live dictation](#live-dictation)
- [Text cleanup](#text-cleanup)
- [Architecture](#architecture)
- [Project layout](#project-layout)
- [Development](#development)
- [Testing](#testing)
- [Privacy and security](#privacy-and-security)
- [Troubleshooting](#troubleshooting)
- [Known limitations](#known-limitations)
- [Roadmap](#roadmap-from-project-intent)
- [Glossary](#glossary)
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
| Spoken-number, decimal, and code normalization (built-in) | Yes |
| System tray while settings closed | Yes |
| On-screen dictation indicator | Yes (Rune overlay) |
| In-app updates from GitHub releases | Yes (portable `Voxen.exe`) |
| Live transcript preview in settings | Yes (`liveTranscript`) |
| Persisted settings | Yes (`config.json`) |
| Persisted transcript history | No (RAM only) |
| Streaming UI in other apps | No (live typing uses keystrokes, not app APIs) |
| Installer or signed update artifacts | No (portable exe; see [Known limitations](#known-limitations)) |
| Launch at login | Planned |
| Managed runtime and model library | Yes (download, progress, remove) |
| Technical context word hints | Yes with Nemotron 3.5 (embedded tokenizer); no with Nemotron English or Parakeet |
| macOS / Linux | Not supported |

The **release milestone** is: shortcut → capture → local transcription → insertion. Everything in the “Planned” row is explicitly later work.

---

## User workflow

### First-time setup

1. Download the latest `Voxen.exe` from GitHub releases and keep it anywhere you like. It is portable: there is no installer, and running it needs neither Node, WebView2, nor Rust.
2. Open **Settings → Model**. Choose a language and compute device, then download the official **NeMo-Speech.cpp v0.2.0 Vulkan** runtime and one model. Voxen verifies the archive or GGUF checksum and installs it under `%LOCALAPPDATA%\com.voxen.app\Cache`:
   - **Nemotron 3.5 multilingual** streams live and supports 32 production locales.
   - **Nemotron English** streams live and is English-only.
   - **Parakeet TDT v3** covers 25 European languages but is offline-only: the transcript is inserted after you stop.
3. Use **Test microphone**, say one sentence, and stop. The transcript stays in the sidebar; the Model tab marks the sample complete.
4. If you already have a NeMo runtime, model, or a custom device string, use **Advanced manual setup** on the same tab. **Save changes** applies it.

### Dictating into another app

1. Focus the target application (Notepad, browser, Discord, VS Code, etc.).
2. Use the configured shortcut:
   - **Hold mode:** press and hold to record; release to stop and transcribe.
   - **Toggle mode:** press once to start; press again to stop.
3. Voxen shows a small **indicator** overlay while starting, recording, transcribing, or inserting.
4. While you speak, Voxen streams recognition and **types interim text** into the original foreground window when live dictation is active (see [Live dictation](#live-dictation)). If live dictation cannot start, it falls back to transcribing after you stop and pasting once with Ctrl+V.
5. Pressing **Enter** in the target app while a dictation session is in progress cancels any remaining insertion for that utterance (see [Keyboard submission protection](#keyboard-submission-protection)).

### Settings window vs tray

- Closing the settings window **ends shortcut capture** and leaves dictation running; Rune keeps the process, state, and tray alive (`keep_running`).
- Tray menu: **Settings** (reopens settings), **Quit** (exits Voxen). Clicking the tray icon also opens settings.
- Closing the window does not quit the process; use **Quit** from the tray (or finish an update, which quits to relaunch).

### Test microphone (settings)

The **Test microphone** button records while Voxen is focused. The transcript appears in the settings preview and is **not** inserted elsewhere. Global dictation always targets the app that was focused when recording started.

If you switch away before insertion completes, Voxen keeps the transcript and reports that the original window is no longer active.

### Model memory

- The recognizer can stay loaded between utterances for faster follow-up dictation.
- **Unload model after** (seconds idle): set to `0` to keep the model resident; otherwise the inference thread unloads after the idle timeout.
- **Unload model** button frees model memory immediately when idle.
- NeMo 0.2.0 DLLs remain loaded until Voxen exits (unloading/reloading the native runtime caused failures during verification). Restart Voxen before replacing runtime DLLs on disk.

---

## Settings reference

Settings are stored as JSON at **`config.json`** in the app config directory resolved by `rune::dirs::AppDirs::new("com.voxen.app").config()`:

| OS | Typical path |
|----|----------------|
| Windows | `%APPDATA%\com.voxen.app\config.json` |

Writes use a temp file (`config.new`) plus atomic rename. Invalid saved JSON or validation failures surface as an `error` phase on startup with a message to review settings; the in-memory config then falls back to defaults.

Fields (camelCase in JSON):

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `microphone` | string or null | null | Device name from enumeration; null = system default |
| `shortcut` | string | `Ctrl+Shift+Space` | Rune global shortcut syntax |
| `activationMode` | `"hold"` \| `"toggle"` | `"hold"` | How the shortcut starts/stops recording |
| `runtimePath` | string | `""` | NeMo runtime directory or DLL path |
| `modelPath` | string | `""` | Local GGUF model file |
| `device` | string | `vulkan:0` | `cpu` or `vulkan:<index>` |
| `replacements` | `{ from, to }[]` | `[]` | Literal substring replace on transcript (not regex); applied in array order; empty `from` rows are skipped at runtime but rejected on save |
| `unloadAfterSeconds` | number | `300` | Idle unload delay; max 86400; `0` = never auto-unload |
| `cleanupMode` | `"raw"` \| `"light"` \| `"polished"` | `"raw"` | Post-transcription cleanup pass; `raw` skips it |
| `cleanupRuntimePath` | string | `""` | `llama-server` binary or the folder containing it; required unless mode is `raw` |
| `cleanupModelPath` | string | `""` | Small local instruction GGUF for cleanup; required unless mode is `raw` |
| `language` | string | `"auto"` | Nemotron locale such as `en-US`; `auto` detects. Parakeet ignores this hint |
| `contextMode` | `"off"` \| `"selection"` \| `"nearby"` | `"off"` | Read selected or caret-adjacent accessibility text for word hints and cleanup; word hints need the tokenizer-equipped Nemotron 3.5 GGUF |
| `excludedApps` | string[] | `[]` | Executable names skipped before any context is read |

Validation highlights:

- Empty `from` in replacements is rejected on save.
- Runtime and model paths must be set before recording (global or test).
- Cleanup modes other than `raw` require both cleanup paths.
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

The registered shortcut is handled by Rune (`AppHandle::register_shortcut`). While the shortcut field is focused in settings, capture mode ignores global events so rebinding does not start dictation.

| `activationMode` | Event | Current phase | Action |
|------------------|-------|---------------|--------|
| `hold` | Pressed | `idle` or `error` | Start recording |
| `hold` | Pressed | `starting` or `recording` | Cancel start / stop → transcribe → maybe insert |
| `hold` | Released | `starting` | Cancel start |
| `hold` | Released | `recording` | Stop → transcribe → maybe insert |
| `toggle` | Pressed | `recording` or `starting` | Cancel start / stop → transcribe → maybe insert |
| `toggle` | Pressed | `idle` or `error` | Start recording |
| any | any | `transcribing` / `inserting` | Ignored |
| any | any | shortcut capture active | Ignored |

Shortcut strings use Rune shortcut syntax (e.g. `Ctrl+Shift+Space`). Unmodified bindings are allowed because the app opts into Rune's unmodified shortcuts. The settings UI builds bindings from `Ctrl`, `Alt`, `Shift`, `Super`, and a physical key (`rune::Shortcut::from_physical_key`). Registration conflicts surface as errors such as `Ctrl+Shift+Space is already in use`.

**Shortcut capture in settings:** Focusing the shortcut field unregisters the global binding until blur. Press the desired chord; release modifiers to finish, **Esc** restores the previous value, **Tab** is ignored. Saving while capture is active is rejected. Losing window focus ends capture (`VoxenApp::on_focus(false)`), as does closing the window (`on_close`).

### Phrase replacements

Replacements are simple literal `from` → `to` substitutions applied to every hypothesis (live and batch), in list order. They are not regular expressions. Use them for names, product terms, or fixing systematic misrecognitions. Empty `from` values are skipped at runtime but cannot be saved.

---

## Dictation state machine

Phases exposed through `Engine::snapshot()`:

```
idle ──(start)──► starting ──(microphone ready)──► recording ──(stop)──► transcribing ──(insert?)──► inserting ──► idle
                    │
                    └──(cancel start)──► idle

any active phase ──(failure)──► error ──(start)──► starting
```

| Phase | Meaning |
|-------|---------|
| `idle` | Ready for a new recording |
| `starting` | Microphone stream is opening; the shortcut can still cancel |
| `recording` | Microphone stream active |
| `transcribing` | NeMo ASR running on captured samples |
| `inserting` | Pasting or revising text in the target window (Windows) |
| `error` | Last operation failed; message in `error` |

New recordings are allowed only in `idle` or `error`. `starting`, `recording`, `transcribing`, and `inserting` block overlapping work.

`modelLoaded` reflects whether the inference worker currently holds a loaded `Recognizer` instance.

### `AppSnapshot` (Engine state)

`Engine::snapshot()` and the snapshot returned by `save_config` use the same shape (camelCase JSON):

| Field | Type | Description |
|-------|------|-------------|
| `config` | `Config` | Current settings (see table above) |
| `phase` | string | One of `idle`, `starting`, `recording`, `transcribing`, `inserting`, `error` |
| `error` | string or null | Human-readable message when `phase` is `error`, or a non-fatal live/cleanup warning |
| `lastTranscript` | string or null | Most recent final transcript after replacements, cleanup, and normalization (settings preview) |
| `liveTranscript` | string or null | Combined final + interim text during recording when the live stream is active; cleared when the utterance finishes |
| `modelLoaded` | boolean | Whether the inference thread still holds a loaded NeMo recognizer |

There is no IPC event stream. `Engine::publish()` bumps a revision counter and wakes the Rune UI (`AppHandle::update`); `ui::VoxenApp::poll` compares the latest snapshot and redraws when it changed. The settings state keeps the draft settings form plus the latest snapshot.

### Edge cases

- **Empty audio** — NeMo returns an error (`No audio was captured`); phase becomes `error`.
- **Whitespace-only transcript** — Replacements run; batch paste is skipped when trimmed text is empty (`foreground != 0` required for any insertion).
- **Start cancelled** — Pressing the shortcut while `starting` sets the cancel flag; if the microphone had just opened, it is stopped immediately. Phase returns to `idle`, `liveTranscript` clears, and the indicator hides.
- **Live dictation unavailable** — If `StartLive` fails (model load or `Recognizer::stream`), the snapshot shows `Live typing is unavailable: …` while recording continues with no live session; audio still accumulates. On stop, `live` is absent so the **batch** `transcribe` + `insert_text` path runs instead.
- **Live dictation failure mid-utterance** — A stream or `replace_text` error is stored on the live session; finishing the recording surfaces that error instead of silently falling back.
- **Voxen focused at record start** — Foreground HWND is cleared (`foreground = 0`) whenever the foreground window belongs to the Voxen process; transcription still runs and `lastTranscript` updates, but nothing is pasted (same as test microphone from the settings window).
- **Target app loses focus before paste** — Insertion aborts with: *The original window is no longer active; transcript was preserved without insertion*.
- **Modifiers still held** — After up to 2s wait: *Release Ctrl, Shift, Alt, and Windows keys before inserting the transcript*.
- **Enter pressed in the target app** — A physical Enter (without Shift) cancels the session's insertion. No batch paste runs, and live text stops being revised; the transcript is still recorded in settings.
- **Keyboard observation unavailable** — Starting a recording with a target window fails with `Keyboard submission protection unavailable: …` rather than recording without it.
- **Save while busy** — Settings changes are rejected until phase is `idle` or `error`; saving also sends `Unload` so the next utterance reloads the model with new paths/device.

---

## Live dictation

During **recording**, Voxen tries to run NeMo’s **streaming** ASR in parallel with microphone capture when the selected model supports it. Parakeet TDT is offline-only and skips straight to batch recognition:

1. `start_recording` captures the foreground HWND (same rules as batch insertion: `0` when Voxen itself is focused), installs the keyboard submission observer when the target is non-zero, asks the audio worker to open the microphone, and reads optional technical context before publishing `starting` or showing the indicator. Phase becomes `starting`.
2. When the stream opens, a completion thread queues `InferenceCommand::StartLive` with the config, target HWND, context terms, and the session’s cancellation token for streaming models; phase becomes `recording`.
3. The audio worker forwards new mono samples to the inference thread every ~100 ms (`forward_audio` → `LiveAudio`).
4. On the first chunk, the inference worker loads the model if needed and opens a `nemo::Stream`.
5. Each `stream.push` is followed by `pump_live`, which reads interim/final hypotheses from `stream.next`.
6. For each update, `apply_replacements` runs, then:
   - **Final** segments append to the session’s accumulated `text`.
   - **Interim** text is the latest non-final hypothesis for the current phrase.
   - If `foreground != 0` and the session is not cancelled, `insertion::replace_text` revises what is already in the target field (backspace shared prefix, type the suffix, Unicode-aware).
7. `liveTranscript` in the snapshot is the visible combination of finalized text plus current interim text (for the settings preview).
8. On **stop**, `Transcribe` finishes the stream. If a live session was active, it cleans and normalizes the final text, revises the already-typed text when either changed (unless cancelled), sets `lastTranscript`, clears `liveTranscript`, and **does not** run batch `insert_text` (the target app already has the text). If live never started, the worker uses one-shot `Recognizer::transcribe` and `insert_text` as before.

Live typing temporarily releases held modifier keys (Ctrl/Shift/Alt/Win), sends keystrokes, then restores modifiers so shortcut chords do not block insertion.

**Test microphone** uses the same pipeline but with `foreground = 0`, so you see `liveTranscript` / `lastTranscript` in settings only.

---

## Text cleanup

Text appears live while speaking in every cleanup mode. After recording stops, Voxen can clean the final transcript with a small local instruction model and replace the already-typed text. Live interim text is never cleaned; only the final transcript is. The final text then passes the deterministic normalizer below in every mode. If streaming is unavailable, cleanup runs after batch transcription.

| Mode | Effect |
|------|--------|
| `raw` | No model cleanup; the deterministic normalizer still runs |
| `light` | Remove only the filler interjections um/uh and resolve an adjacent same-sentence self-correction (`Tuesday, no wait, Wednesday` → `Wednesday`) |
| `polished` | Light cleanup plus punctuation, capitalization, sentence splits, and numbered-list markers only when every listed word is kept and no conjunction is dropped |

**Runtime:** Voxen manages a warm local `llama-server` child process (no FFI, no cloud, localhost only). The server starts lazily on the first cleanup with a fixed 2048-token context, reasoning disabled, and greedy decoding, on `127.0.0.1:18231` (override with `VOXEN_CLEANUP_PORT`). It makes a single bounded startup attempt (60s); a failed start keeps the original transcript and reports an error. A running server is reused across utterances with no per-call model poll, and stopped when settings are saved, the model is unloaded, Voxen exits, or the idle unload delay expires. When `unloadAfterSeconds` is `0` the model stays loaded but the cleanup server still stops after the default 300 seconds. Changing the cleanup runtime or model path unloads the previous server.

**Pinned setup (verified October 18, 2026):** `llama-server` from the llama.cpp Vulkan backend (2.51.0) with `Qwen3.5-0.8B` rejected as too weak (it either echoed fillers or dropped hedging) and `Qwen3.5-2B-Q4_K_S` (~1.2 GB) pinned instead: it removes fillers, resolves adjacent corrections, and preserves names, numbers, negations, and commands. Set:

- **Cleanup runtime path** — `llama-server.exe` or its folder, e.g. the LM Studio backend folder `llama.cpp-win-x86_64-vulkan-avx2-2.51.0`
- **Cleanup model path** — e.g. `Qwen3.5-2B-Q4_K_S.gguf`

**Technical context (optional, off by default):** Voxen can read the focused control's explicitly selected text, or up to 400 characters either side of the caret when nothing is selected, through Windows UI Automation once at recording start, before the indicator appears and before any focus change. Context is bounded to 800 characters, read on a long-lived MTA worker through the `CUIAutomation8` client with 250 ms automation timeouts, and never uses screenshots or the clipboard. Password fields are detected on the focused element and its ancestors; an `edit` window with `ES_PASSWORD` is skipped. Executable names in **Excluded applications** are checked before any read. On the pinned Nemotron 3.5 GGUF, whose embedded SentencePiece tokenizer NVIDIA added in a re-converted checkpoint, the extracted technical terms are also passed as `speech_contexts` word hints. Voxen checks the GGUF for the embedded tokenizer key before reading context, so an older tokenizer-less 3.5 file also stays cleanup-only. The pinned Nemotron English GGUF has no embedded tokenizer and Parakeet TDT has no word boosting, so no accessibility read happens in Raw mode with either of them. With Light or Polished cleanup the same bounded text is added to the local cleanup prompt as a limited spelling and casing reference. The cleanup guard still preserves every transcript word, so it can adjust capitalization and punctuation but cannot replace a misspelled word or introduce a technical term. The English model still receives the language hint; Parakeet ignores it. If context is off, no accessibility read happens at all.

**Safety guard:** every model reply passes an ordered lexical check before it replaces the transcript. Only clear um/uh deletions, narrow comma-delimited edge fillers (er/ah/basically), single-word adjacent same-sentence self-correction resolutions, punctuation/case changes, and numbered-list markers are allowed, and fillers may only be removed, never added; ambiguous phrases (you know, sort of, kind of, like), sentence-initial So, names (Ah, Er), conjunctions (and/or), negations, numbers, symbols, paths, and commands are preserved exactly, including hyphens, slashes, dots, and other symbols inside words. Any other added, dropped, or swapped word (added fillers, extra negations, changed numbers, swapped names or days, cross-sentence or trailing-cue corrections, truncation, hallucinations) rejects the reply. Rejected and failed cleanups keep the original transcript and show a warning in the settings preview (`Cleanup kept the original transcript: …`). Requests are bounded to 2000 input characters with a 30-second timeout; over-long or cut-off replies are rejected the same way.

**Normalization:** after cleanup — and independently of the cleanup mode, including `raw` — the final transcript passes a deterministic local pass with no model calls. It converts spoken numbers and model codes to digits (`three ninety five` → `395`, `ninety nine fifty X` → `9950X`, `M Five` → `M5`, `RTX fifty eighty` → `RTX 5080`, `three hundred ninety five` → `395`, `four oh two` → `402`, `four thirty` → `430`), attaches a single code letter to an adjacent number (`five K` → `5K`), collapses an adjacent repeated function word (`a a` → `a`), and joins spoken decimal readings (`three point one four` → `3.14`, `V four point one` → `V4.1`, `three point twenty five` → `3.25`). Number words only convert when the run is unambiguous: lone number words (`three files`), cue-marked times (`at four thirty`, `four thirty am`), comma- or conjunction-separated numbers (`fifty, eighty`), sentence breaks, and repeated pairs (`fifty fifty`) stay as spoken. It runs on the finished utterance only, never on live interim text, and because it runs after the guard the guard rules for model replies are unchanged.

---

## Architecture

### Single native process

Voxen is one Rust executable. Rune owns the event loop, GPU rendering, native windows, the indicator overlay, the tray icon, and global shortcuts. There is no webview, no JavaScript, no Vite dev server, no Tauri runtime, and no IPC layer.

```text
Rune settings window                 Rune global shortcut
        │ Engine calls                       │
        └──────────────┬─────────────────────┘
                       ▼
                    Engine (Clone + Send + Sync)
                       │  Arc<Mutex<Backend>> + revision counter
        ┌──────────────┼──────────────────────┬────────────────────┐
        ▼              ▼                      ▼                    ▼
  audio worker   inference worker      keyboard observer      updater workers
   (cpal)         (NeMo, cleanup)      (per recording)        (check/install)
```

### Data flow

```text
Global shortcut / settings "Test microphone"
        │
        ▼
  start_recording ──► capture foreground HWND ──► install keyboard observer
        │
        ▼
  audio worker (cpal) ──► mono f32 ──► LiveAudio chunks ──► nemo::Stream
        │                              │
        │                              ├──► apply_replacements
        │                              ├──► replace_text (if foreground ≠ 0)
        │                              └──► liveTranscript in snapshot
        ▼
  stop_recording ──► Transcribe job
        │
        ├── live session? ──► stream.finish ──► cleanup + normalize ──► lastTranscript, no batch paste
        │
        └── else ──► Recognizer::transcribe ──► insert_text (Ctrl+V + clipboard restore)
        │
        ▼
  Engine::publish ──► revision bumps, Rune UI wakes ──► settings window + indicator overlay
```

### Process structure (Rust)

- **Rune UI thread:** settings view, input, global shortcut callbacks, tray callbacks, overlay updates, `poll`/`on_key` hooks, updater draining.
- **Audio worker thread:** owns the cpal input stream; handles `Start` / `Stop` / stream errors.
- **Inference worker thread:** owns optional `nemo::Recognizer`, optional live `Live` session, and optional `cleanup::Cleaner`; handles `StartLive`, `LiveAudio`, `Transcribe`, `Unload`, and `UnloadCleanup`; implements the idle unload deadline.
- **Keyboard observer thread:** installed per recording with a non-zero target; runs a low-level keyboard hook and cancels the session on a physical Enter. Dropped when the session ends.
- **Start completion thread:** waits for the audio worker's start reply, then sets `recording` or `error` and queues `StartLive`.
- **Updater workers:** one short-lived thread per GitHub check or install.

Shared state: `Engine` → `Arc<Mutex<Backend>>` with snapshot, channel senders, captured foreground HWND, shortcut-capture flag, start-cancel flag, cancellation/submission state, shortcut id, and indicator handle.

### Worker message protocol

The Rune UI thread does not run cpal callbacks or NeMo inference itself. It sends commands on two unbounded `std::sync::mpsc` channels:

**Audio worker (`AudioCommand`)**

| Message | Payload | Effect |
|---------|---------|--------|
| `Start` | optional mic name, reply channel | Open cpal input stream, reset buffer |
| `Stop` | reply channel | Stop stream, forward tail audio, return `(samples, sample_rate)` |
| `Failed` | error string | Tear down stream, finish the session, set phase `error` |

While a stream is active, the audio worker wakes every **100 ms** (recv timeout) to call `forward_audio` even if no new command arrived.

**Inference worker (`InferenceCommand`)**

| Message | Payload | Effect |
|---------|---------|--------|
| `StartLive` | `Config`, foreground HWND, `Cancellation` | Load model if needed, open `nemo::Stream`, reset live session |
| `LiveAudio` | `Vec<f32>`, sample rate | `stream.push` + `pump_live` when a session exists |
| `Transcribe` | `Config`, samples, rate, HWND, `Cancellation` | Finish live stream **or** batch transcribe + optional `insert_text` |
| `Unload` | — | Drop live session, recognizer, and cleanup server; clear idle deadline; `modelLoaded = false` |
| `UnloadCleanup` | — | Drop only the cleanup server and its deadline |

After a successful `Transcribe`, if `unload_after_seconds > 0` and a model remains loaded, the worker sets a recv timeout to `Unload` when idle time elapses. Otherwise a loaded cleanup server gets the default 300-second `UnloadCleanup` deadline.

### Application lifecycle

**Startup (`run` in `lib.rs`):**

1. Set the Rune app id (`com.voxen.app`) and app icon.
2. Build the Rune app: 900×680 window, 720×520 minimum, `poll` and `on_key` hooks, `keep_running`, unmodified shortcuts allowed, and close/focus/quit callbacks.
3. Create `Engine`, which loads `config.json` and spawns the audio and inference workers.
4. Create the indicator overlay (`220×76`, bottom edge, centred, 40 pt margin, non-interactive).
5. Start the tray with **Settings** and **Quit**; tooltip: `Voxen — local dictation`.
6. Register the saved global shortcut on the UI thread; registration failure surfaces as `error`.
7. Build `ui::VoxenApp` from the `Engine` and run the app.

**While running:**

- Closing the settings window ends shortcut capture and keeps the process, state, and tray alive (`keep_running`). **Settings** from the tray (or clicking the tray icon) shows and focuses the window again.
- Dictation keeps running while the window is closed.
- The indicator overlay is shown for `starting` (unless the start was cancelled), `recording`, `transcribing`, and `inserting`, and hidden otherwise.
- Tray **Quit** calls `AppHandle::quit`; the `on_quit` hook sends `Unload` and waits up to 300 ms for worker shutdown.
- When the updater reports `Ready`, `VoxenApp::poll` calls `tx.quit()` so the swap helper can replace the executable after the process exits.

**Saving settings:**

- Validate the shortcut and config first; save is rejected while busy or during shortcut capture.
- Atomic write via `config.new` → rename to `config.json`.
- Shortcut is re-registered when the binding changed or was missing; the new binding is registered before the old one is unregistered.
- An `Unload` is always sent to the inference thread so path/device changes take effect on the next utterance.
- If writing `config.json` fails after a new shortcut was registered, the previous shortcut is re-registered and the new one is unregistered so the app does not stay on a broken binding.

### Audio capture

- **Library:** [cpal](https://github.com/RustAudio/cpal) on the default host.
- **Device:** Named microphone from settings, or system default input.
- **Format:** Device default stream config; samples supported as **F32**, **I16**, or **U16** (other formats error at stream build).
- **Channels:** Multi-channel input is averaged to **mono** per frame (`append_mono`).
- **Buffer:** Samples accumulate in a `Vec<f32>` until `Stop`; sample rate is passed through to NeMo as captured.

Mic stream failures post `AudioCommand::Failed` to the audio worker, clear the stream, and set phase `error`.

### Settings window (Rune)

The settings UI is native Rune code, not HTML:

| Path | Role |
|------|------|
| `src-tauri/src/ui.rs` | `VoxenApp`: draft config, snapshot, view, `poll`, `on_key`, shortcut capture |
| `src-tauri/src/theme.rs` | Colour and type tokens ported from the old stylesheet |
| `src-tauri/src/widgets.rs` | Shared controls for the settings window |
| `src-tauri/src/indicator.rs` | Recording indicator overlay state and view |

`VoxenApp` shows the phase and model-loaded state, live and last transcript previews, **Test microphone**, microphone selection, activation mode and shortcut capture, model paths/device/idle unload and **Unload model**, cleanup mode and paths, replacement rows, update check/install, and a save action. Draft edits are local until saved; each Engine call refreshes the snapshot.

`poll` drains the updater, keeps hold-mode working by checking `Engine::shortcut_down()` every ~40 ms while `starting`/`recording`, and reports whether visible state changed. `on_key` implements shortcut capture; `on_shortcut` forwards to `Engine::handle_shortcut`; `on_close` and focus loss end capture.

### Indicator overlay

A Rune edge overlay (`OverlaySpec`) created at startup:

- **Size:** 220×76, transparent, bottom-centre of the primary display, 40 pt margin, click-through.
- **Hidden** when phase is `idle` or `error`.
- **Labels:** `Starting`, `Listening...`, `Transcribing`, `Inserting`; `Error` in red when the overlay is updated with an error.
- **Motion:** a soft glow around the dot pulses only while `recording`.

### Native modules (Rust)

| Module | Role |
|--------|------|
| `src-tauri/src/lib.rs` | `Engine` facade, workers, config, shortcuts, tray, app/overlay setup |
| `src-tauri/src/ui.rs` | Settings window state and view |
| `src-tauri/src/theme.rs` | Colour/type tokens |
| `src-tauri/src/widgets.rs` | Shared settings controls |
| `src-tauri/src/indicator.rs` | Recording indicator overlay |
| `src-tauri/src/hotkey.rs` | Hold-mode key-state probe (`GetAsyncKeyState`) |
| `src-tauri/src/updater.rs` | GitHub release check, download, swap helper |
| `src-tauri/src/nemo.rs` | Dynamic load of `nemo_speech_asr_c.dll`, C ABI bindings, transcribe |
| `src-tauri/src/cleanup.rs` | Managed `llama-server` child process, bounded chat requests, lexical safety guard |
| `src-tauri/src/normalize.rs` | Deterministic final-text normalizer (numbers, decimals, code letters, duplicated function words) |
| `src-tauri/src/insertion.rs` | Windows `insert_text` (clipboard + Ctrl+V), `replace_text` (live Unicode keystrokes), keyboard submission observer |

### Engine API

The UI calls `Engine` directly; there are no commands or events to serialize.

| Method | Purpose |
|--------|---------|
| `snapshot` | Current `AppSnapshot` |
| `revision` | Monotonic change counter bumped by `publish` |
| `list_microphones` | Sorted, deduplicated input device names |
| `save_config` | Validate, atomic write `config.json`, refresh shortcut, unload model |
| `set_shortcut_capture` | Pause global shortcut while the user picks a new binding |
| `start_recording` | Capture foreground HWND (unless Voxen), install submission observer, start mic |
| `stop_recording` | Cancel a pending start, or stop mic and queue transcription |
| `unload_model` | Drop recognizer in the inference thread |
| `handle_shortcut` | Apply press semantics for the current activation mode |
| `shortcut_down` | Probe whether the configured chord is physically held |
| `open_window` / `hide_window` / `quit` | Window and process control for tray/UI paths |
| `report_error` / `attach_indicator` / `shutdown` | Backend/overlay wiring and quit cleanup |

### Text insertion (Windows)

Two paths:

**Batch (`insert_text`)** — used when live dictation did not run for the utterance:

1. Return immediately if the session was cancelled.
2. Verify the target HWND is still valid and foreground.
3. Wait up to 2s for modifier keys (Ctrl/Shift/Alt/Win) to release after shortcut use, checking the cancellation flag every 20 ms.
4. Re-check foreground and cancellation.
5. Open clipboard in context of target window; duplicate all formats for backup (insertion aborts if a format cannot be preserved).
6. Set Unicode text, send Ctrl+V via `SendInput`, brief sleep.
7. If clipboard sequence number unchanged, restore all backed-up formats; otherwise assume the target app consumed clipboard and skip restore.

**Live (`replace_text`)** — used during streaming when `foreground != 0`:

1. Return immediately if cancelled; verify foreground unchanged.
2. Compute the character-level common prefix with the previous hypothesis.
3. Release any held modifiers, backspace characters not shared with that prefix, type the new suffix with `KEYEVENTF_UNICODE`, restore modifiers, with cancellation checks between batches.
4. Common-prefix optimization avoids retyping unchanged characters.

Privilege note: injection cannot reliably reach apps running at a higher integrity level. Run Voxen and the target app at the same privilege level. Apps that do not treat backspace/Unicode input like a normal text field may behave poorly with live dictation.

### Keyboard submission protection

While recording into a target window, Voxen installs a low-level keyboard hook (`WH_KEYBOARD_LL`) for that session:

- Only a **physical** Enter keypress counts; injected keystrokes are ignored.
- Enter must arrive while the original target window is foreground, and the session must not have Shift held. **Shift+Enter** does not cancel and reaches the target app normally.
- Repeated keydowns submit only once, and an Enter that was already held when observation started is ignored until it is released and pressed again.
- On a valid submit, the session's cancellation token is set. `insert_text` returns without touching the target, `replace_text` stops revising typed text, and the transcript is still stored as `lastTranscript`.
- The observer is dropped with the session; if it cannot be installed, recording does not start.

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

Model licenses: [Nemotron 3.5](https://huggingface.co/nvidia/nemotron-3.5-asr-streaming-0.6b) and [Nemotron English](https://huggingface.co/nvidia/nemotron-speech-streaming-en-0.6b) under the NVIDIA Open Model License; [Parakeet TDT v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3) under CC-BY-4.0. The managed runtime is Apache-2.0.

**Pinned runtime:** NeMo-Speech.cpp **v0.2.0** Windows Vulkan x86_64. Managed model URLs and checksums are pinned in `library.rs`; the Nemotron 3.5 entry points at NVIDIA's re-converted GGUF (`ea30d66`) that embeds the SentencePiece tokenizer required for word boosting, because the v0.2.0 and v0.3.0 `models/index.json` entries still reference the older tokenizer-less revision. The Parakeet entry is retained for offline recognition and existing installations.

---

## Project layout

```text
Voxen/
├── .github/
│   └── workflows/
│       └── release.yml      # Tag-triggered cargo release: Voxen.exe, zip, checksums
├── docs/
│   ├── APP.md               # Full application reference (this file)
│   └── RUNE_UI_CONTRACT.md  # Migration ownership and Rune API contracts
├── packaging/
│   └── build.ps1            # Local release staging: exe + zip + SHA256SUMS
├── src-tauri/
│   ├── src/
│   │   ├── main.rs          # Entry → voxen_lib::run()
│   │   ├── lib.rs           # App setup, Engine, workers, tray, shortcuts
│   │   ├── ui.rs            # Settings window
│   │   ├── theme.rs         # Colour/type tokens
│   │   ├── widgets.rs       # Shared controls
│   │   ├── indicator.rs     # Indicator overlay
│   │   ├── hotkey.rs        # Hold-mode key probe
│   │   ├── updater.rs       # GitHub release check/install
│   │   ├── nemo.rs          # NeMo DLL bindings
│   │   ├── cleanup.rs       # llama-server cleanup + guard
│   │   ├── normalize.rs     # Deterministic normalizer
│   │   └── insertion.rs     # Paste, live typing, submission observer
│   ├── build.rs             # winres icon/version resource
│   ├── Cargo.toml
│   └── icons/
├── runtime/                 # Local NeMo runtime (optional in repo); JFK sample WAV
├── bench/                   # Cleanup prompt/model benchmarks
├── src/                     # Legacy Svelte frontend — reference only, not built
├── package.json             # Legacy frontend metadata — not used by the build
├── AGENTS.md
└── README.md
```

The portable release artifact is a single `Voxen.exe` (plus a zip containing the exe and README). Runtime and model files are **not** bundled; users configure paths after first launch. The legacy `src/`, `package.json`, `vite.config.ts`, `svelte.config.js`, `tsconfig.json`, `dist/`, and `src-tauri/tauri.conf.json` files remain only as reference and are not part of the build.

---

## Development

### Prerequisites

- Windows 10/11 x64
- Rust **1.99.1** with the MSVC toolchain
- Visual Studio C++ Build Tools (MSVC linker and Windows SDK)

Node.js, npm, WebView2, and the Tauri CLI are **not** required to build or run Voxen.

### Commands

| Command | Purpose |
|---------|---------|
| `cargo +1.99.1 build --manifest-path src-tauri/Cargo.toml` | Debug build |
| `cargo +1.99.1 run --manifest-path src-tauri/Cargo.toml` | Run the native app |
| `cargo +1.99.1 build --release --manifest-path src-tauri/Cargo.toml` | Release build |
| `cargo +1.99.1 check --manifest-path src-tauri/Cargo.toml` | Compile check |
| `cargo +1.99.1 test --manifest-path src-tauri/Cargo.toml` | Unit tests |
| `./packaging/build.ps1` | Stage `Voxen.exe`, zip, and SHA256SUMS in `packaging/out` |

There is no dev server or hot reload; rebuild and relaunch the native process.

### Dependencies

| Layer | Packages |
|-------|----------|
| Runtime | `rune` (pinned git rev; patches `parley_engine`, `wgpu-hal`, `winit`, `accesskit_windows` to the same rev), `serde`/`serde_json`, `cpal` 0.15, `libloading` 0.8, `png` 0.18, `ureq` 2 |
| Windows only | `windows` 0.58 (Win32 UI/input, clipboard, GDI, OLE, threading), `clipboard-win` 5 |
| Build | `winres` 0.1 for the exe icon and version resource |

External (not cargo): NeMo-Speech.cpp runtime DLLs and a Nemotron or Parakeet GGUF on disk; optionally llama.cpp `llama-server` and a cleanup GGUF.

### Native shell configuration

- App ID: `com.voxen.app`
- Main window: 900×680, minimum 720×520
- Tray icon decoded from `src-tauri/icons/icon.png`; exe icon from `icons/icon.ico` via `winres`
- Process keeps running when the window closes (`keep_running`)
- Unmodified global shortcuts allowed
- Indicator overlay: 220×76, bottom edge, centred, 40 pt margin, non-interactive

### Releases and updates

Releases are built by GitHub Actions from `.github/workflows/release.yml`, which runs on `v*` tags (or manual dispatch), installs Rust 1.99.1, runs `cargo build --release --manifest-path src-tauri/Cargo.toml`, and publishes:

- `Voxen.exe`
- `Voxen_<version>_x64.zip` (exe + README)
- `SHA256SUMS.txt`

1. Bump `version` in `src-tauri/Cargo.toml` (the updater reads it; tag builds use the tag name, manual dispatches read `Cargo.toml`).
2. Commit, then push a matching tag:
   ```powershell
   git tag v0.2.0
   git push origin master --tags
   ```
3. The workflow publishes the portable artifacts to the GitHub release.

**Update flow (in-app):**

1. Check `https://api.github.com/repos/Vortextbloons/Voxen/releases/latest`.
2. Compare the numeric tag (leading `v` stripped) with `CARGO_PKG_VERSION`.
3. Choose the `Voxen.exe` asset (or the first `.exe` asset) and download it to `%TEMP%\voxen-update`.
4. Reject an incomplete download or a file that is not a Windows executable (missing `MZ` header).
5. Write a hidden `voxen-update.bat` that waits for the current PID to exit, moves the new exe over the running one, relaunches it, and deletes itself.
6. Once the helper is running, Voxen quits (`UpdateStatus::Ready`); the helper swaps the exe and launches the new version.

**Limitation:** the migration removed Tauri's minisign updater and the `latest.json` feed. An existing Tauri/NSIS installation cannot auto-update into the Rune build; install the portable `Voxen.exe` once. After that, in-app updates replace the portable exe in place. Updates are **not cryptographically signed or verified**; Voxen relies on HTTPS and the GitHub release channel, plus the basic PE/size checks above. Treat the GitHub account and release permissions as the update trust boundary. There is also no installer, uninstall entry, or Start-menu shortcut.

### Contribution notes

See [AGENTS.md](../AGENTS.md) for editing principles and [RUNE_UI_CONTRACT.md](RUNE_UI_CONTRACT.md) for Rune file ownership and backend API contracts.

---

## Testing

### Always-on Rust tests

| Test | Module | What it checks |
|------|--------|----------------|
| `stereo_capture_is_averaged_to_mono` | `lib.rs` | Multi-channel mic samples average to mono |
| `empty_replacements_are_rejected_and_cannot_expand_text` | `lib.rs` | Save validation and runtime skip for empty `from` |
| `transcribing_and_inserting_block_new_recordings` | `lib.rs` | `is_available` allows only `idle` and `error`; `starting`, `recording`, `transcribing`, and `inserting` block |
| `cleanup_requires_a_mode_and_both_paths` | `lib.rs` | Cleanup only runs with a non-raw mode and both paths |
| `selecting_a_cleanup_mode_requires_cleanup_paths` | `lib.rs` | Saved non-raw mode without paths is rejected |
| `indicator_visibility_follows_the_phase` | `lib.rs` | Indicator shows for starting/recording/transcribing, hides when a start is cancelled |
| `live_edits_only_replace_the_changed_suffix` | `insertion.rs` | Common-prefix logic for live typing |
| `live_edits_keep_whole_surrogate_pairs` | `insertion.rs` | Unicode surrogate-safe backspace boundaries |
| `cancelled_sessions_never_touch_the_target_window` | `insertion.rs` | Cancelled insert/replace calls are no-ops |
| `observer_installs_and_stops_cleanly` | `insertion.rs` | Keyboard observer lifecycle |
| `injected_enter_does_not_submit_but_physical_enter_does` | `insertion.rs` | Injected keystrokes are ignored; physical Enter cancels |
| `enter_only_submits_in_the_target_window` | `insertion.rs` | Enter outside the captured target does not cancel |
| `shift_enter_keeps_a_newline_even_if_shift_is_released_first` | `insertion.rs` | Shift+Enter is not a submission |
| `repeated_enter_keydowns_submit_only_once` | `insertion.rs` | Duplicate keydowns cancel once |
| `enter_held_before_observation_does_not_submit_on_repeat` | `insertion.rs` | Pre-held Enter is ignored until released |
| `submission_state_is_scoped_to_one_session` | `insertion.rs` | One session's submit does not cancel another |
| `cleanup` guard tests | `cleanup.rs` | Clear-filler/correction acceptance; conjunction, symbol/number, ambiguous-filler, name, negation, day-swap, cross-sentence-correction, truncation, and hallucination rejection |
| `normalize` tests | `normalize.rs` | Spoken numbers, scale values, digit pairs, code letters, decimals/versions convert; times, separators, lone number words, and duplicates stay as words |
| `load_*` cleanup tests | `cleanup.rs` | Missing paths are rejected; present files load without starting a server |

### Opt-in integration tests

Require local runtime, model, GPU, or interactive clipboard:

```powershell
$env:VOXEN_RUNTIME = (Resolve-Path runtime/nemo-speech-0.2.0-windows-x86_64-vulkan).Path
$env:VOXEN_MODEL = 'C:\path\to\parakeet-tdt-0.6b-v3.q8_0.gguf'
$env:VOXEN_CLEANUP_RUNTIME = 'C:\path\to\llama.cpp-win-x86_64-vulkan-avx2-2.51.0'
$env:VOXEN_CLEANUP_MODEL = 'C:\path\to\Qwen3.5-2B-Q4_K_S.gguf'
cargo +1.99.1 test --manifest-path src-tauri/Cargo.toml real_runtime_transcribes_sample -- --ignored
cargo +1.99.1 test --manifest-path src-tauri/Cargo.toml real_runtime_cleans_spoken_text -- --ignored
cargo +1.99.1 test --manifest-path src-tauri/Cargo.toml real_runtime_streams_sample_incrementally -- --ignored
cargo +1.99.1 test --manifest-path src-tauri/Cargo.toml rich_and_empty_clipboards_are_restored -- --ignored
cargo +1.99.1 test --manifest-path src-tauri/Cargo.toml paste_reaches_a_windows_edit_control -- --ignored
cargo +1.99.1 test --manifest-path src-tauri/Cargo.toml live_typing_revises_unicode_without_duplicating -- --ignored
```

`real_runtime_streams_sample_incrementally` feeds the JFK sample in small chunks and asserts interim streaming results appear before the final transcript (NeMo C ABI only).

`live_typing_revises_unicode_without_duplicating` drives `replace_text` through progressive hypotheses into a native `EDIT` control, including Unicode text.

Documented verification (October 18, 2026) on the primary dev machine: Vulkan + RX 9060 XT, JFK sample transcription, C ABI reload, native shortcut/mic tests, clipboard restore tests. Cross-app spoken dictation in Discord, VS Code, and browsers still needs manual interactive validation.

---

## Privacy and security

- Audio samples exist in RAM during capture and inference; they are not written to disk by Voxen.
- Transcripts live in memory (`lastTranscript` in the snapshot) until replaced or the app exits.
- Settings file contains paths and preferences only, not audio.
- Dictation needs no network once runtime and model are local. Text cleanup talks only to a `llama-server` child process on localhost.
- Update checks and downloads contact GitHub (`api.github.com` and the release asset URL). Updates are not signature-verified; see [Releases and updates](#releases-and-updates).

---

## Troubleshooting

| Symptom | Likely cause | What to do |
|---------|----------------|------------|
| Shortcut does nothing | Shortcut not registered, or capture is active | Open Settings and pick another shortcut if startup reported a registration/conflict error |
| *Set and save the local runtime and model paths first* | Paths empty in saved config | Fill **Runtime path** and **Model path**, **Save changes** |
| *Failed to load … nemo_speech_asr_c.dll* | Wrong runtime path or DLLs split | Point at SDK root or DLL; keep NeMo `bin` DLLs together |
| *No microphone is available* | No input device or wrong name | Refresh mic list; use system default or reconnect device |
| Transcript in settings only | Recording started while Voxen was focused | Focus target app before using the global shortcut |
| Paste missing in target app | Focus changed, live disabled and batch skipped, or higher integrity target | Keep target focused; run Voxen and target at same privilege level |
| Paste or cleanup revision skipped | Enter was pressed in the target app | Expected submission protection; dictate again if needed |
| *Keyboard submission protection unavailable* | Low-level keyboard hook could not be installed | Restart Voxen; check security software blocking hooks |
| Garbled or duplicated live text | Target app handles keys differently | Try another editor; batch fallback applies only if streaming never started |
| Clipboard not restored | Target consumed clipboard (sequence number changed) | Expected for some apps; backup skipped intentionally |
| Model slow first time, fast after | Cold load vs resident recognizer | Lower **Unload model after** or set `0`; use **Unload model** to free RAM |
| *llama-server was not found…* | Wrong cleanup runtime path | Point at `llama-server.exe` or its folder; keep the backend DLLs together |
| *The cleanup model was not found* | Wrong cleanup model path | Point at the local GGUF file |
| *Cleanup kept the original transcript…* | Guard rejected the model reply (or the request timed out) | Original text was preserved; try Light mode or a shorter utterance |
| Settings invalid on startup | Hand-edited `config.json` | Fix JSON/values in UI and save again |
| Update check fails | GitHub unreachable, HTTP error, or a release without an `.exe` asset | Check connectivity; retry after the release finishes publishing |
| Update install fails | Incomplete download, non-PE asset, or the swap helper could not replace the exe | Download the release manually and replace `Voxen.exe`; restart Voxen |

---

## Known limitations

- **Windows only** for paste insertion; other platforms would need alternate insertion strategies.
- **No signed updates** — the Tauri minisign flow is gone. A downloaded update is checked only for a complete transfer and a Windows `MZ` header before it replaces the exe; trust is HTTPS plus the GitHub release channel.
- **Portable exe only** — no installer, uninstall entry, or Start-menu shortcut; updating is an in-place exe replacement.
- **Paste-based batch fallback** — apps without standard Ctrl+V behavior may need future app-specific formatters.
- **Live keystroke insertion** — revises text via backspace + Unicode typing; exotic inputs (vim, remote desktop, custom widgets) may not work reliably.
- **Foreground capture** — focus must remain on the target through insertion for paste to succeed.
- **NeMo DLL lifetime** — full native unload of DLLs is avoided; restart app after runtime updates, and remove a managed runtime only after restarting Voxen.
- **Parakeet is offline-only** — NeMo-Speech.cpp v0.2.0 rejects streaming for Parakeet TDT; Voxen skips live typing and inserts once after you stop. Parakeet also ignores language hints and word hints.
- **Context needs TextPattern** — technical context is available only when the focused control exposes UI Automation `TextPattern`; controls without it yield no context. Password detection and excluded-app matching are best-effort and bounded by 250 ms automation timeouts. Word hints work only with the tokenizer-equipped Nemotron 3.5 GGUF; the pinned Nemotron English GGUF has no embedded tokenizer, and Parakeet TDT has no word boosting, so context is cleanup-only there.
- **No download resume** — a managed runtime or model download restarts from zero if interrupted.
- **Overlay placement** - the indicator is placed at the bottom-centre of the primary display by the app.
- **Submission protection scope** — only physical Enter in the captured target window cancels; other send/submit shortcuts are not observed.

---

## Roadmap (from project intent)

Not implemented yet; listed for context:

- Launch at login
- Persisted dictation history
- Further streaming polish (non–text-field apps, latency tuning)
- Application-specific formatting
- Performance tuning beyond current idle model retention

Phrase replacements, the deterministic normalizer, offline local inference, and the local text cleanup pass are the main post-processing surface today.

---

## Glossary

| Term | Meaning |
|------|---------|
| ASR | Automatic speech recognition (Nemotron or Parakeet via NeMo-Speech.cpp) |
| Rune | Native Rust UI framework pinned to the git revision above; owns windows, GPU rendering, tray, shortcuts, and overlays |
| Engine | Cloneable backend facade the settings UI calls for snapshots and actions |
| NeMo runtime | Extracted `nemo-speech-*` SDK with `nemo_speech_asr_c.dll` and CLI |
| Utterance | One record cycle from shortcut start to stop |
| Foreground HWND | Windows window handle captured when recording starts; insertion target |
| Phase | Coarse dictation state: `idle`, `starting`, `recording`, `transcribing`, `inserting`, `error` |
| Live session | Open NeMo streaming recognizer for the current utterance |
| Submission protection | Per-session keyboard hook that cancels insertion when you press Enter in the target app |

---

## Quick links

- Root [README](../README.md) — short overview and link here
- [NeMo-Speech.cpp v0.2.0 release](https://github.com/NVIDIA/NeMo-Speech.cpp/releases/tag/v0.2.0)
- [Parakeet TDT 0.6B v3 model card](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3)
