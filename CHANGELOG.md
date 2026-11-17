# Voxen v0.4.0

## Added

- Guided local setup with language selection, detected CPU/Vulkan devices, microphone testing, and sample dictation confirmation.
- Managed model library with Nemotron 3.5 multilingual, Nemotron English, and Parakeet TDT v3; download progress, pinned SHA-256 verification, staged installation, removal, and model license/capability information.
- Optional selected-text or caret-relative nearby Windows accessibility context, application exclusions, password-field protection, and an explicit off control.
- Technical word hints with the tokenizer-equipped Nemotron 3.5 model; bounded local context for cleanup while preserving the existing word-change guard.

## Changed

- Native Rune desktop application replaces the previous Tauri interface.
- Parakeet TDT v3 is explicitly batch-only with the pinned NeMo runtime; Nemotron supports streaming dictation.
- Nemotron English and Parakeet context is cleanup-only because their managed models do not support technical word hints.

## Upgrading

Download Voxen.exe or extract Voxen_0.4.0_x64.zip and run Voxen.exe on Windows x64.

Automatic updates support Rune-to-Rune builds only. Tauri-to-Rune upgrades and automatic migration of Tauri settings are unsupported. Quit the old Tauri app, download the portable Rune build manually, and complete setup. No Tauri latest.json feed or signed Tauri updater artifacts are published.

Existing Rune builds can use their in-app updater to replace the portable executable in place. Keep the executable in a writable location. Updates are unsigned; Windows may warn about the download.

## Known limitations

- Runtime removal after loading requires restarting Voxen; model removal unloads the recognizer first.
- Downloads restart after interruption rather than resuming.
- Accessibility context availability varies by application; it has not been interactively validated across supported editors.
- The complete in-app network installation flow still needs end-to-end verification. Artifact hashes, sizes, and real-runtime transcription/streaming were checked separately.
