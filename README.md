# Voxen

Local Windows dictation with Svelte 5, Tauri 2, Rust, and Parakeet TDT v3 through the NeMo-Speech.cpp C ABI. Audio stays in RAM and recognition runs inside the app. There is no transcription server or Python runtime.

Full application reference: [docs/APP.md](docs/APP.md).

## Development

Install Node.js, Rust with the MSVC toolchain, Visual Studio C++ Build Tools, and WebView2. Then run:

```powershell
npm install
npm run tauri dev
```

`npm run dev` opens only the browser UI; microphone capture and global shortcuts require the native Tauri application.

## Local model setup

Download the Windows Vulkan archive from the [NeMo-Speech.cpp v0.2.0 release](https://github.com/NVIDIA/NeMo-Speech.cpp/releases/tag/v0.2.0), verify its published SHA-256, and extract the whole archive. Keep its DLLs together. The development copy is under `runtime/nemo-speech-0.2.0-windows-x86_64-vulkan`.

Download the pinned model once with the included CLI:

```powershell
& './runtime/nemo-speech-0.2.0-windows-x86_64-vulkan/bin/nemo-speech.exe' pull parakeet-tdt
```

The CLI prints the resulting local GGUF path. Set that path and the absolute path to `bin/nemo_speech_asr_c.dll` in Voxen's settings. Use `vulkan:0` for the RX 9060 XT on this machine. Runtime and model downloads are setup steps; dictation itself works offline. The model has a [CC-BY-4.0 license](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3).

## Text cleanup setup

Cleanup is optional and off by default (`raw` mode). It needs a `llama-server` binary and a small instruction model on disk — both stay local. Verified with the llama.cpp 2.51.0 Vulkan backend and `Qwen3.5-2B-Q4_K_S.gguf` (the smaller Qwen3.5-0.8B proved too weak: it echoed fillers or dropped hedging). In Voxen's settings choose **Light** or **Polished**, set the cleanup runtime path to `llama-server.exe` (or its folder) and the cleanup model path to the GGUF file, then save. If a cleanup reply rewrites too much, the original transcript is kept and a warning appears in the sidebar. Spoken numbers and model codes are normalized by a built-in deterministic pass that needs no setup (`M Five` → `M5`, `ninety nine fifty X` → `9950X`, `fifty eighty` → `5080`) and runs in every mode.

## First-use check

1. Save the local runtime/model paths and choose a microphone.
2. Open Notepad and place the cursor in a document.
3. Press the configured global shortcut, speak, and release it in hold mode; press again to stop in toggle mode.
4. Check that the transcript appears in the original application and that the clipboard is restored.
5. Repeat in a browser, Discord, and VS Code before relying on it for everyday use.

Windows input injection cannot reliably target applications running at a higher privilege level. Run the app and target editor at the same privilege level. Clipboard restoration and application paste timing need interactive testing across those applications.

## Checks

```powershell
npm run check
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
```

The release milestone is shortcut → capture → local transcription → insertion. Launch at login, ASR vocabulary boosting, persisted history, application-specific formatting, and performance tuning are later work. Phrase replacements, automatic spoken-number normalization, and the optional local text cleanup pass are available now; transcripts are retained only in RAM.

The settings window closes completely while dictation stays available in the system tray. Choose **Settings** from the tray menu to reopen it, or **Quit** to stop Voxen. The **Test microphone** button keeps its result in the settings window; global dictation inserts into the original foreground application. If you switch applications before insertion, Voxen keeps the transcript and reports the focus change.

The recognizer stays resident between utterances. Set the idle timeout to `0` to retain it, or use **Unload model** to free its memory. NeMo 0.2.0's DLLs stay loaded until Voxen exits because unloading and reloading them caused a native runtime failure during verification; model memory is still freed. Restart Voxen before replacing runtime DLLs.

## Local inference verification

On October 18, 2026, NeMo-Speech.cpp 0.2.0 reported a compatible Vulkan runtime and the RX 9060 XT as device 0. Its pinned Parakeet Q8 model successfully transcribed the upstream 11-second JFK sample. A second CLI invocation took 1.40 seconds including process startup and model loading. This is a single sample, not an app latency benchmark or a microphone-to-paste test.

The app's C ABI test also passed punctuation/capitalization, repeated inference, and model destruction/reload on Vulkan0. Native app checks passed microphone enumeration/capture, hold shortcut release, changing the shortcut, toggle mode, and timed model unloading. Windows tests passed rich/Unicode/empty clipboard restoration and actual `SendInput` paste into a Windows edit control. Cross-application checks in Discord, VS Code, and browsers still need interactive testing with spoken dictation.

Hardware and clipboard tests are opt-in because they require the local runtime, GPU, or clipboard:

```powershell
$env:VOXEN_RUNTIME = (Resolve-Path runtime/nemo-speech-0.2.0-windows-x86_64-vulkan).Path
$env:VOXEN_MODEL = 'C:\path\to\parakeet-tdt-0.6b-v3.q8_0.gguf'
cargo test --manifest-path src-tauri/Cargo.toml real_runtime_transcribes_sample -- --ignored
cargo test --manifest-path src-tauri/Cargo.toml rich_and_empty_clipboards_are_restored -- --ignored
cargo test --manifest-path src-tauri/Cargo.toml paste_reaches_a_windows_edit_control -- --ignored
```

Build the Windows installer with `npm run tauri build -- --bundles nsis`. Model and runtime assets are external and are not embedded in the installer. This development machine already has the verified runtime under `runtime/` and the pinned model in `%LOCALAPPDATA%\NeMoSpeech\models`.
