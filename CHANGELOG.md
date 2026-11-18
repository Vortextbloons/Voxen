# Voxen v0.5.0

## Changed

- Voxen releases idle dictation resources sooner while it sits in the tray. The cleanup server may unload after about 20 seconds of tray idle, and the speech recognizer may unload after about 45 seconds. A shorter configured **Unload model after** delay still applies, and `0` keeps the recognizer resident.
- Reopening Settings cancels a pending tray release. Closing the window during a recording or transcription waits for that session to finish before the tray timers start.
- The recording indicator releases its GPU surface after 30 seconds hidden, while repeated dictation can reuse it.
- Microphone startup and capture failures now end the session cleanly, surface the error, and resume the normal idle schedule.

## Known issue

- The renderer is intentionally unchanged in this release. The same build has intermittently used about one CPU core while idle on AMD systems; this is still under investigation.
- No specific memory savings are claimed because no post-dictation RAM benchmark was run.

## Upgrading

Download Voxen.exe or extract Voxen_0.5.0_x64.zip and run Voxen.exe on Windows x64.

Existing Rune builds can use their in-app updater to replace the portable executable in place. Keep the executable in a writable location. Updates are unsigned; Windows may warn about the download. Tauri-to-Rune upgrades and automatic migration of Tauri settings remain unsupported.

## Known limitations

- Runtime removal after loading requires restarting Voxen; model removal unloads the recognizer first.
- Downloads restart after interruption rather than resuming.
- Accessibility context availability varies by application; it has not been interactively validated across supported editors.
- The complete in-app network installation flow still needs end-to-end verification.
