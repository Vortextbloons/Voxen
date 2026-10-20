<div align="center">

<table>
<tr>
<td align="center" style="background: linear-gradient(160deg, #1a2d5c 0%, #142343 35%, #0a1228 70%, #080F25 100%); border: 1px solid #5789FF40; border-radius: 24px; padding: 52px 32px 44px; box-shadow: 0 24px 80px #080F2566;">

<table cellpadding="0" cellspacing="0" style="margin: 0 auto 20px;">
<tr>
<td align="center" style="padding: 6px; border-radius: 52px; background: linear-gradient(135deg, #16C9FF55, #5789FF44, #A42DFF55);">
<img src="src-tauri/icons/icon.svg" width="168" height="168" alt="Voxen" style="display: block; border-radius: 44px; box-shadow: 0 12px 40px #16C9FF33, 0 0 0 1px #FFFFFF12 inset;" />
</td>
</tr>
</table>

<h1 style="margin: 0 0 6px; font-size: 3em; letter-spacing: -0.03em; line-height: 1.1;">
  <span style="color: #FFFFFF;">Vox</span><span style="color: #16C9FF;">en</span>
</h1>

<p style="margin: 0 0 22px; font-size: 1.15em; font-weight: 500; color: #CBD9FF; letter-spacing: 0.02em;">
  Speak anywhere you type.
</p>

<p style="margin: 0 auto 28px; max-width: 580px; color: #9EB0E0; line-height: 1.6; font-size: 0.98em;">
  A lightweight <strong style="color: #E8EEFF;">Windows dictation</strong> app — global shortcut, local speech recognition, and your words land in whatever already has focus.
</p>

<p style="margin: 0;">
  <img src="https://img.shields.io/badge/Platform-Windows-142343?style=for-the-badge&logo=windows&logoColor=white" alt="Windows" />
  &nbsp;
  <img src="https://img.shields.io/badge/STT-Local_on_device-16C9FF?style=for-the-badge&logo=cpu&logoColor=142343" alt="Local STT" />
  &nbsp;
  <img src="https://img.shields.io/badge/Cloud-None-5789FF?style=for-the-badge&logo=shield&logoColor=white" alt="No cloud" />
  &nbsp;
  <img src="https://img.shields.io/badge/Stack-Tauri_·_Rust-A42DFF?style=for-the-badge&logo=tauri&logoColor=white" alt="Tauri + Rust" />
</p>

</td>
</tr>
</table>

<br />

<table width="100%">
<tr>
<td align="center" width="32%" style="padding: 18px 14px; border-radius: 16px; background: linear-gradient(180deg, #142343 0%, #101a35 100%); border: 1px solid #16C9FF35;">
  <div style="font-size: 1.5em; margin-bottom: 6px;">🎙️</div>
  <strong style="color: #16C9FF;">Hold or toggle</strong><br />
  <span style="color: #8FA4D8; font-size: 0.88em;">Your shortcut, your mode</span>
</td>
<td width="2%"></td>
<td align="center" width="32%" style="padding: 18px 14px; border-radius: 16px; background: linear-gradient(180deg, #142343 0%, #101a35 100%); border: 1px solid #5789FF35;">
  <div style="font-size: 1.5em; margin-bottom: 6px;">🔒</div>
  <strong style="color: #5789FF;">On-device STT</strong><br />
  <span style="color: #8FA4D8; font-size: 0.88em;">Audio stays on your machine</span>
</td>
<td width="2%"></td>
<td align="center" width="32%" style="padding: 18px 14px; border-radius: 16px; background: linear-gradient(180deg, #142343 0%, #101a35 100%); border: 1px solid #A42DFF35;">
  <div style="font-size: 1.5em; margin-bottom: 6px;">⚡</div>
  <strong style="color: #C77DFF;">Tray-first</strong><br />
  <span style="color: #8FA4D8; font-size: 0.88em;">Minimal UI until you need settings</span>
</td>
</tr>
</table>

</div>

<br />

---

## How it works

```mermaid
flowchart LR
  A["🖱️ Focus any app"] --> B["⌨️ Global shortcut"]
  B --> C["🎙️ Speak"]
  C --> D["📝 Text at cursor"]
  D --> E["✨ Optional local polish"]
  style A fill:#142343,stroke:#16C9FF,color:#fff
  style B fill:#142343,stroke:#5789FF,color:#fff
  style C fill:#142343,stroke:#16C9FF,color:#fff
  style D fill:#142343,stroke:#5789FF,color:#fff
  style E fill:#080F25,stroke:#A42DFF,color:#CBD9FF
```

<table width="100%">
<tr>
<td width="50%" valign="top" style="padding: 8px 20px 8px 0;">

### Getting started

1. Put the cursor where you want text.
2. **Hold** the shortcut to talk, or **toggle** it on and off in Settings.
3. When you stop, Voxen inserts the transcript there.

Optional cleanup can lightly polish wording with a small **local** model — **off by default**.

</td>
<td width="50%" valign="top" style="padding: 8px 0 8px 24px; border-left: 2px solid #5789FF28;">

### Scope

Voxen is intentionally small: shortcut dictation, local speech recognition, and a few optional tweaks — not accounts, sync, saved history, or a full voice assistant.

| | |
|---|---|
| ✅ | Global hotkey, tray, local models |
| ❌ | Cloud transcription, voice commands, history |

**Windows only** for now. Microphone, shortcut, and model setup live in **Settings**.

</td>
</tr>
</table>

---

<div align="center">

<table width="100%">
<tr>
<td align="center" width="33%" style="padding: 22px 12px; vertical-align: top; border-radius: 14px; background: #F8FAFF;">

<div style="font-size: 2em; margin-bottom: 8px;">⚡</div>

**Fast**

<p style="margin: 8px 0 0; color: #5a6b8a; font-size: 0.9em;">
Global shortcut, minimal chrome, tray when you need it
</p>

</td>
<td width="2%"></td>
<td align="center" width="33%" style="padding: 22px 12px; vertical-align: top; border-radius: 14px; background: #F8FAFF;">

<div style="font-size: 2em; margin-bottom: 8px;">🔐</div>

**Private**

<p style="margin: 8px 0 0; color: #5a6b8a; font-size: 0.9em;">
Recognition on your machine — no cloud transcription
</p>

</td>
<td width="2%"></td>
<td align="center" width="33%" style="padding: 22px 12px; vertical-align: top; border-radius: 14px; background: #F8FAFF;">

<div style="font-size: 2em; margin-bottom: 8px;">🎯</div>

**Focused**

<p style="margin: 8px 0 0; color: #5a6b8a; font-size: 0.9em;">
Dictation into the app that already has focus
</p>

</td>
</tr>
</table>

<br />

<table>
<tr>
<td align="center" style="padding: 16px 28px; border-radius: 14px; background: linear-gradient(135deg, #142343 0%, #1a2d5c 100%); border: 1px solid #5789FF33;">
<span style="color: #CBD9FF; font-size: 0.92em;"><strong style="color: #16C9FF;">Goal</strong> — fast, private voice input with almost no UI. Settings and a tray icon when you need them; otherwise it stays out of the way.</span>
</td>
</tr>
</table>

<br />

<a href="docs/APP.md">
  <img src="https://img.shields.io/badge/Documentation-Setup_&_dev_guide-16C9FF?style=for-the-badge&logo=readthedocs&logoColor=142343" alt="Documentation" />
</a>

<br /><br />

<sub style="color: #8899bb;">Svelte 5 · Tauri 2 · Rust · Parakeet TDT v3 · NeMo-Speech.cpp</sub>

</div>
