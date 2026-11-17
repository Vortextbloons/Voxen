use crate::theme::{self, Palette, HAIRLINE, RADIUS, SIDEBAR_WIDTH};
use crate::updater::{UpdateStatus, Updater};
use crate::widgets::{self, Tone};
use crate::{cleanup, AppSnapshot, Config, Engine, Replacement};
use rune::{col, el, row, text, Cursor, El, Key, KeyEvent, LoopSpec, ScrollSpec, TextEdit, Tx};
use std::time::Duration;

const SHORTCUT_KEY: &str = "shortcut-capture";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Microphone,
    Activation,
    Model,
    Cleanup,
    Replacements,
    Updates,
}

const TABS: [(Tab, &str); 6] = [
    (Tab::Microphone, "Microphone"),
    (Tab::Activation, "Activation"),
    (Tab::Model, "Model"),
    (Tab::Cleanup, "Cleanup"),
    (Tab::Replacements, "Replacements"),
    (Tab::Updates, "Updates"),
];

struct Fields {
    runtime: TextEdit,
    model: TextEdit,
    device: TextEdit,
    unload: TextEdit,
    cleanup_runtime: TextEdit,
    cleanup_model: TextEdit,
}

impl Fields {
    fn from_config(config: &Config) -> Self {
        Self {
            runtime: TextEdit::new(config.runtime_path.clone()),
            model: TextEdit::new(config.model_path.clone()),
            device: TextEdit::new(config.device.clone()),
            unload: TextEdit::new(config.unload_after_seconds.to_string()),
            cleanup_runtime: TextEdit::new(config.cleanup_runtime_path.clone()),
            cleanup_model: TextEdit::new(config.cleanup_model_path.clone()),
        }
    }
}

struct ReplacementRow {
    from: TextEdit,
    to: TextEdit,
}

pub struct VoxenApp {
    pub(crate) engine: Engine,
    snapshot: AppSnapshot,
    draft: Config,
    fields: Fields,
    replacements: Vec<ReplacementRow>,
    microphones: Vec<String>,
    microphone_error: Option<String>,
    tab: Tab,
    saved: bool,
    saving: bool,
    save_error: Option<String>,
    action_error: Option<String>,
    shortcut_capture: bool,
    shortcut_previous: String,
    shortcut_error: Option<String>,
    updater: Updater,
    loaded: bool,
}

impl VoxenApp {
    pub fn new(engine: Engine) -> Self {
        let snapshot = engine.snapshot();
        let (microphones, microphone_error) = match Engine::list_microphones() {
            Ok(names) => (names, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        let draft = snapshot.config.clone();
        let fields = Fields::from_config(&draft);
        let replacements = replacement_rows(&draft);
        let mut updater = Updater::new();
        updater.check();
        Self {
            engine,
            snapshot,
            draft,
            fields,
            replacements,
            microphones,
            microphone_error,
            tab: Tab::Microphone,
            saved: false,
            saving: false,
            save_error: None,
            action_error: None,
            shortcut_capture: false,
            shortcut_previous: String::new(),
            shortcut_error: None,
            updater,
            loaded: true,
        }
    }

    pub fn view(&self, cx: &mut rune::Cx<Self>) -> El {
        let p = Palette::dark();
        cx.foreground(p.ink);
        cx.focus_ring(2.0, 1.0, p.accent);
        if !self.loaded {
            return row()
                .key("window")
                .role("window")
                .w_full()
                .h_full()
                .fill(p.bg)
                .center()
                .child(
                    text("Loading…")
                        .key("loading")
                        .font(theme::HINT)
                        .color(p.ink_faint),
                );
        }
        row()
            .key("window")
            .role("window")
            .w_full()
            .h_full()
            .fill(p.bg)
            .child(self.sidebar(&p))
            .child(self.content(&p))
    }

    pub fn poll(&mut self, _clock: f64, tx: &mut Tx) -> bool {
        let mut changed = self.updater.poll();
        if matches!(
            self.updater.status(),
            UpdateStatus::Checking | UpdateStatus::Installing
        ) {
            tx.wake_after(Duration::from_millis(250));
        }
        if self.updater.status() == &UpdateStatus::Ready {
            tx.quit();
            return true;
        }
        let snapshot = self.engine.snapshot();
        if snapshot.config != self.snapshot.config {
            self.apply(snapshot);
            changed = true;
        } else {
            if snapshot.phase != self.snapshot.phase
                || snapshot.error != self.snapshot.error
                || snapshot.last_transcript != self.snapshot.last_transcript
                || snapshot.live_transcript != self.snapshot.live_transcript
                || snapshot.model_loaded != self.snapshot.model_loaded
            {
                changed = true;
            }
            self.snapshot = snapshot;
        }
        if matches!(self.snapshot.phase.as_str(), "starting" | "recording")
            && self.snapshot.config.activation_mode == "hold"
        {
            if !self.engine.shortcut_down() {
                let _ = self.engine.stop_recording();
            }
            tx.wake_after(Duration::from_millis(40));
        }
        changed
    }

    pub fn on_key(&mut self, ev: &KeyEvent, tx: &mut Tx) -> bool {
        if !self.shortcut_capture {
            if tx.focused() != Some(SHORTCUT_KEY) || matches!(ev.key, Key::Tab) {
                return false;
            }
            self.begin_shortcut_capture();
            if !self.shortcut_capture {
                return false;
            }
        } else if tx.focused().is_some_and(|key| key != SHORTCUT_KEY) {
            self.end_shortcut_capture();
            return false;
        }
        match &ev.key {
            Key::Tab => false,
            Key::Escape => {
                self.draft.shortcut = self.shortcut_previous.clone();
                self.end_shortcut_capture();
                self.saved = false;
                tx.blur();
                true
            }
            Key::Modifier { pressed: false, .. } if mods_empty(ev.mods) => {
                self.end_shortcut_capture();
                tx.blur();
                true
            }
            Key::Modifier { .. } => true,
            _ => {
                let Some(key) = tx.physical_key() else {
                    return true;
                };
                match rune::Shortcut::from_physical_key(key, ev.mods) {
                    Ok(shortcut) => {
                        self.draft.shortcut = shortcut.to_string();
                        self.shortcut_error = None;
                        self.saved = false;
                        self.end_shortcut_capture();
                        tx.blur();
                    }
                    Err(error) => self.shortcut_error = Some(error.to_string()),
                }
                true
            }
        }
    }

    pub fn on_close(&mut self, _tx: &mut Tx) {
        self.end_shortcut_capture();
    }

    pub fn on_shortcut(&mut self, _tx: &mut Tx) {
        self.engine.handle_shortcut();
    }

    pub fn on_focus(&mut self, focused: bool, _tx: &mut Tx) {
        if !focused {
            self.end_shortcut_capture();
        }
    }

    fn apply(&mut self, snapshot: AppSnapshot) {
        self.snapshot = snapshot;
        self.draft = self.snapshot.config.clone();
        self.fields = Fields::from_config(&self.draft);
        self.replacements = replacement_rows(&self.draft);
        self.saved = false;
    }

    fn sidebar(&self, p: &Palette) -> El {
        let mut list = col()
            .key("sidebar-scroll")
            .grow(1.0)
            .min_h(0.0)
            .w_full()
            .gap(12.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .scroll_edge_fade(6.0);
        if let Some(error) = &self.snapshot.error {
            list = list.child(widgets::error(p, error));
        }
        if let Some(error) = &self.action_error {
            list = list.child(widgets::error(p, error));
        }
        if let Some(live) = &self.snapshot.live_transcript {
            list = list.child(transcript(p, "Live", "live", live));
        }
        if let Some(last) = &self.snapshot.last_transcript {
            list = list.child(transcript(p, "Last transcription", "last", last));
        }
        let mut status = col()
            .gap(7.0)
            .pb(16.0)
            .border_sides([0.0, 0.0, HAIRLINE, 0.0], p.line)
            .child(self.phase_chip(p))
            .child(self.model_line(p));
        if let Some(info) = self.update_available() {
            let version = info.version.clone();
            status = status.child(update_chip(p, &version));
        }
        col()
            .key("sidebar")
            .w(SIDEBAR_WIDTH)
            .h_full()
            .shrink()
            .px(18.0)
            .py(24.0)
            .gap(16.0)
            .fill(p.surface_soft)
            .border_sides([0.0, HAIRLINE, 0.0, 0.0], p.line)
            .child(text("Voxen").font(theme::BRAND).color(p.ink))
            .child(status)
            .child(self.record_button(p))
            .child(widgets::hint(
                p,
                "Focus Voxen to test the microphone — the transcript stays here and inserts nothing. To dictate, focus the target app and use the shortcut.",
            ))
            .child(list)
    }

    fn phase_chip(&self, p: &Palette) -> El {
        let phase = self.snapshot.phase.as_str();
        let mut dot = el().size(7.0, 7.0).radius(3.5).a11y_hidden();
        dot = match phase {
            "recording" => dot
                .linear_gradient(2.356, [(0.0, p.accent_cyan), (1.0, p.accent_violet)])
                .shadow(0.0, 6.0, p.accent_cyan.alpha(0.55))
                .looping(LoopSpec::opacity(1.6, 0.0, 0.35, 1.0)),
            "transcribing" => dot
                .fill(p.accent_blue)
                .shadow(0.0, 6.0, p.accent_blue.alpha(0.5)),
            "inserting" => dot
                .fill(p.accent_violet)
                .shadow(0.0, 6.0, p.accent_violet.alpha(0.5)),
            "error" => dot.fill(p.danger),
            _ => dot.fill(p.ink_faint),
        };
        row()
            .key("phase")
            .gap(8.0)
            .center_y()
            .child(dot)
            .child(text(phase_label(phase)).font(theme::CHIP).color(p.ink_soft))
    }

    fn model_line(&self, p: &Palette) -> El {
        row()
            .key("model-state")
            .gap(7.0)
            .center_y()
            .child(
                el().size(6.0, 6.0)
                    .radius(3.0)
                    .fill(if self.snapshot.model_loaded {
                        p.ok
                    } else {
                        p.ink_faint
                    })
                    .a11y_hidden(),
            )
            .child(
                text(if self.snapshot.model_loaded {
                    "Model loaded"
                } else {
                    "Model not loaded"
                })
                .font(theme::SMALL)
                .color(p.ink_faint),
            )
    }

    fn record_button(&self, p: &Palette) -> El {
        let phase = self.snapshot.phase.as_str();
        let tone = if phase == "recording" {
            Tone::Danger
        } else {
            Tone::Primary
        };
        let mut button =
            widgets::button(p, record_label(phase), tone, self.record_enabled()).w_full();
        if self.record_enabled() {
            button = button.on_press(|s: &mut VoxenApp| s.toggle_recording());
        }
        button
    }

    fn record_enabled(&self) -> bool {
        let paths = !self.snapshot.config.runtime_path.trim().is_empty()
            && !self.snapshot.config.model_path.trim().is_empty();
        paths
            && !matches!(
                self.snapshot.phase.as_str(),
                "starting" | "transcribing" | "inserting"
            )
    }

    fn toggle_recording(&mut self) {
        self.action_error = None;
        let result = if self.snapshot.phase == "recording" {
            self.engine.stop_recording()
        } else {
            self.engine.start_recording()
        };
        if let Err(error) = result {
            self.action_error = Some(error);
        }
    }

    fn unload_enabled(&self) -> bool {
        self.snapshot.model_loaded && matches!(self.snapshot.phase.as_str(), "idle" | "error")
    }

    fn update_available(&self) -> Option<&crate::updater::UpdateInfo> {
        if *self.updater.status() == UpdateStatus::Available {
            self.updater.info()
        } else {
            None
        }
    }

    fn unload_model(&mut self) {
        self.action_error = None;
        if let Err(error) = self.engine.unload_model() {
            self.action_error = Some(error);
        }
    }

    fn content(&self, p: &Palette) -> El {
        col()
            .key("content")
            .grow(1.0)
            .min_h(0.0)
            .h_full()
            .px(36.0)
            .pt(28.0)
            .child(
                col()
                    .key("page-header")
                    .gap(3.0)
                    .child(
                        text("Settings")
                            .key("page-title")
                            .font(theme::PAGE_TITLE)
                            .color(p.ink),
                    )
                    .child(
                        text("Configure dictation, recognition, and text cleanup.")
                            .font(theme::PAGE_SUBTITLE)
                            .color(p.ink_faint),
                    ),
            )
            .child(self.tab_bar(p))
            .child(
                col()
                    .key("body")
                    .grow(1.0)
                    .min_h(0.0)
                    .w_full()
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .scroll_edge_fade(8.0)
                    .child(
                        col()
                            .key("panel-slot")
                            .w_full()
                            .max_w(660.0)
                            .py(26.0)
                            .child(self.panel(p)),
                    ),
            )
            .child(self.save_bar(p))
    }

    fn tab_bar(&self, p: &Palette) -> El {
        let mut bar = row()
            .key("tabs")
            .role("tablist")
            .label("Settings sections")
            .gap(22.0)
            .mt(18.0)
            .focus_group()
            .focus_follows()
            .focus_selected(true)
            .border_sides([0.0, 0.0, HAIRLINE, 0.0], p.line);
        for (tab, label) in TABS {
            bar = bar.child(
                widgets::tab(p, label, self.tab == tab)
                    .key(format!("tab-{}", label.to_lowercase()))
                    .on_press(move |s: &mut VoxenApp| s.tab = tab),
            );
        }
        bar
    }

    fn panel(&self, p: &Palette) -> El {
        match self.tab {
            Tab::Microphone => self.microphone_panel(p),
            Tab::Activation => self.activation_panel(p),
            Tab::Model => self.model_panel(p),
            Tab::Cleanup => self.cleanup_panel(p),
            Tab::Replacements => self.replacements_panel(p),
            Tab::Updates => self.updates_panel(p),
        }
    }

    fn microphone_panel(&self, p: &Palette) -> El {
        let mut panel = panel_shell("microphone", "Microphone");
        if let Some(error) = &self.microphone_error {
            panel = panel.child(widgets::error(
                p,
                &format!("Could not load microphones: {error}"),
            ));
        }
        let mut list = col()
            .key("microphone-list")
            .role("listbox")
            .label("Input device")
            .gap(6.0)
            .focus_group()
            .focus_selected(true);
        list = list.child(
            widgets::option(p, "System default", self.draft.microphone.is_none())
                .on_press(|s: &mut VoxenApp| s.choose_microphone(None)),
        );
        for name in &self.microphones {
            let selected = self.draft.microphone.as_deref() == Some(name.as_str());
            let value = name.clone();
            list = list.child(
                widgets::option(p, name, selected)
                    .on_press(move |s: &mut VoxenApp| s.choose_microphone(Some(value.clone()))),
            );
        }
        panel.child(field(
            p,
            "Input device",
            list,
            Some("Voxen records from the system default input unless you choose another device."),
        ))
    }

    fn activation_panel(&self, p: &Palette) -> El {
        let display = self.shortcut_display();
        let mut capture = el()
            .key(SHORTCUT_KEY)
            .role("textbox")
            .label("Shortcut")
            .a11y_value(display.clone())
            .focusable(true)
            .h(36.0)
            .px(10.0)
            .center_y()
            .radius(RADIUS)
            .fill(p.surface)
            .border(
                HAIRLINE,
                if self.shortcut_capture {
                    p.accent
                } else {
                    p.line
                },
            )
            .cursor(Cursor::Pointer)
            .child(
                text(display)
                    .font(theme::BODY)
                    .color(if self.shortcut_capture {
                        p.ink_faint
                    } else {
                        p.ink
                    })
                    .a11y_hidden(),
            )
            .on_press(|s: &mut VoxenApp| s.begin_shortcut_capture());
        capture = capture.max_w(460.0);
        let mut shortcut = col()
            .gap(6.0)
            .child(widgets::label(p, "Shortcut"))
            .child(capture)
            .child(widgets::hint(
                p,
                "Click the field and press your shortcut. Release the keys, then save. Esc cancels.",
            ));
        if let Some(error) = &self.shortcut_error {
            shortcut = shortcut.child(widgets::error(p, error));
        }
        let mut modes = col().gap(8.0).child(widgets::eyebrow(p, "Mode"));
        for (mode, title, description) in [
            ("hold", "Hold", "Record while the shortcut is held down."),
            (
                "toggle",
                "Toggle",
                "Press once to start, press again to stop.",
            ),
        ] {
            let selected = self.draft.activation_mode == mode;
            modes = modes.child(widgets::choice(p, title, description, selected).on_press(
                move |s: &mut VoxenApp| {
                    s.draft.activation_mode = mode.to_string();
                    s.saved = false;
                },
            ));
        }
        panel_shell("activation", "Activation")
            .child(shortcut)
            .child(modes)
    }

    fn model_panel(&self, p: &Palette) -> El {
        let runtime = field_input(
            p,
            &self.fields.runtime,
            "runtime-path",
            "Runtime path",
            |s: &mut VoxenApp| &mut s.fields.runtime,
        );
        let model = field_input(
            p,
            &self.fields.model,
            "model-path",
            "Model path",
            |s: &mut VoxenApp| &mut s.fields.model,
        );
        let device = field_input(
            p,
            &self.fields.device,
            "device",
            "vulkan:0",
            |s: &mut VoxenApp| &mut s.fields.device,
        );
        let unload_field = field_input(
            p,
            &self.fields.unload,
            "unload-after",
            "300",
            |s: &mut VoxenApp| &mut s.fields.unload,
        )
        .w(120.0)
        .label("Unload model after");
        let mut unload = col()
            .gap(6.0)
            .child(widgets::label(p, "Unload model after"))
            .child(
                row()
                    .gap(8.0)
                    .center_y()
                    .child(unload_field)
                    .child(text("seconds idle").font(theme::HINT).color(p.ink_faint)),
            )
            .child(widgets::hint(p, "0 keeps the model resident."));
        let mut unload_button =
            widgets::button(p, "Unload model", Tone::Secondary, self.unload_enabled()).self_start();
        if self.unload_enabled() {
            unload_button = unload_button.on_press(|s: &mut VoxenApp| s.unload_model());
        }
        unload = unload.child(unload_button);
        panel_shell("model", "Model")
            .child(widgets::intro(
                p,
                "Speech recognition runs locally on this machine. Point Voxen at your speech runtime and a model file on disk.",
            ))
            .child(field(
                p,
                "Runtime path",
                runtime,
                Some("Path to the NeMo SDK root directory or nemo_speech_asr_c.dll."),
            ))
            .child(field(p, "Model path", model, Some("Path to a local model file.")))
            .child(field(
                p,
                "Device",
                device,
                Some("Device identifier used by the runtime, for example cpu or vulkan:0 for an RX9060XT."),
            ))
            .child(unload)
    }

    fn cleanup_panel(&self, p: &Palette) -> El {
        let runtime = field_input(
            p,
            &self.fields.cleanup_runtime,
            "cleanup-runtime-path",
            "Cleanup runtime path",
            |s: &mut VoxenApp| &mut s.fields.cleanup_runtime,
        );
        let model = field_input(
            p,
            &self.fields.cleanup_model,
            "cleanup-model-path",
            "Cleanup model path",
            |s: &mut VoxenApp| &mut s.fields.cleanup_model,
        );
        let mut modes = col().gap(8.0).child(widgets::eyebrow(p, "Mode"));
        for (mode, title, description) in [
            (cleanup::Mode::Raw, "Raw", "Keep the transcript unchanged."),
            (
                cleanup::Mode::Light,
                "Light",
                "Remove fillers and resolve self-corrections, keep every other word.",
            ),
            (
                cleanup::Mode::Polished,
                "Polished",
                "Light cleanup plus punctuation, capitalization, and lists.",
            ),
        ] {
            let selected = self.draft.cleanup_mode == mode;
            modes = modes.child(
                widgets::choice(p, title, description, selected)
                    .on_press(move |s: &mut VoxenApp| s.set_cleanup_mode(mode)),
            );
        }
        panel_shell("cleanup", "Cleanup")
            .child(widgets::intro(
                p,
                "Optional second pass over every transcription with a small local model. If cleanup fails or rewrites too much, the original transcript is kept and a warning appears in the sidebar.",
            ))
            .child(modes)
            .child(field(
                p,
                "Cleanup runtime path",
                runtime,
                Some("Path to llama-server.exe or the folder containing it."),
            ))
            .child(field(
                p,
                "Cleanup model path",
                model,
                Some("Path to a small local instruction model file, for example Qwen3.5-2B-Q4_K_S.gguf."),
            ))
    }

    fn replacements_panel(&self, p: &Palette) -> El {
        let mut panel = panel_shell("replacements", "Replacements").child(widgets::intro(
            p,
            "Words or phrases replaced automatically in every transcription.",
        ));
        if self.replacements.is_empty() {
            panel = panel.child(widgets::card(p).child(widgets::small(
                p,
                "No replacements yet. Add one to replace a word or phrase in every transcription.",
            )));
        } else {
            let mut rows = col().key("replacement-rows").gap(10.0).w_full();
            for (index, row_edit) in self.replacements.iter().enumerate() {
                let from = widgets::text_field(
                    p,
                    &row_edit.from,
                    &format!("replacement-{index}-from"),
                    "From",
                )
                .grow(1.0)
                .min_w(0.0)
                .on_edit(move |s: &mut VoxenApp| &mut s.replacements[index].from)
                .on_input_change(|s: &mut VoxenApp, _: &mut Tx| s.saved = false);
                let to =
                    widgets::text_field(p, &row_edit.to, &format!("replacement-{index}-to"), "To")
                        .grow(1.0)
                        .min_w(0.0)
                        .on_edit(move |s: &mut VoxenApp| &mut s.replacements[index].to)
                        .on_input_change(|s: &mut VoxenApp, _: &mut Tx| s.saved = false);
                rows = rows.child(
                    row()
                        .key(format!("replacement-{index}"))
                        .gap(8.0)
                        .center_y()
                        .w_full()
                        .child(from)
                        .child(text("→").font(theme::HINT).color(p.ink_faint))
                        .child(to)
                        .child(
                            widgets::button(p, "Remove", Tone::Secondary, true)
                                .on_press(move |s: &mut VoxenApp| s.remove_replacement(index)),
                        ),
                );
            }
            panel = panel.child(rows);
        }
        panel.child(
            widgets::button(p, "Add replacement", Tone::Secondary, true)
                .self_start()
                .on_press(|s: &mut VoxenApp| s.add_replacement()),
        )
    }

    fn updates_panel(&self, p: &Palette) -> El {
        let mut panel = panel_shell("updates", "Updates")
            .child(widgets::intro(
                p,
                "Voxen updates itself from GitHub releases. Downloading and installing restarts the app.",
            ))
            .child(field(
                p,
                "Installed version",
                text(Updater::current_version())
                    .font(theme::BODY)
                    .color(p.ink_soft),
                None,
            ));
        match self.updater.status() {
            UpdateStatus::Available => {
                if let Some(info) = self.updater.info() {
                    let version = info.version.clone();
                    let notes = info.notes.trim().to_string();
                    let mut block = col()
                        .gap(6.0)
                        .child(widgets::label(p, &format!("Version {version} available")));
                    if !notes.is_empty() {
                        block = block.child(
                            text(notes)
                                .font(theme::SMALL)
                                .color(p.ink_faint)
                                .wrap()
                                .max_lines(6)
                                .max_w(560.0),
                        );
                    }
                    block = block.child(
                        widgets::button(p, "Download and install", Tone::Primary, true)
                            .self_start()
                            .on_press(|s: &mut VoxenApp| s.install_update()),
                    );
                    panel = panel.child(block);
                }
            }
            UpdateStatus::Installing => {
                let progress = self.updater.progress();
                panel = panel.child(
                    col()
                        .gap(8.0)
                        .child(widgets::label(
                            p,
                            &if progress > 0 {
                                format!("Installing update — {progress}%")
                            } else {
                                "Installing update…".to_string()
                            },
                        ))
                        .child(widgets::progress(p, progress)),
                );
            }
            UpdateStatus::Checking => {
                panel = panel.child(widgets::hint(p, "Checking for updates…"));
            }
            UpdateStatus::Current => {
                panel = panel.child(widgets::status(p, "Voxen is up to date."));
            }
            UpdateStatus::Ready => {
                panel = panel.child(widgets::hint(
                    p,
                    "The update is ready. Voxen restarts to finish installing.",
                ));
            }
            UpdateStatus::Error => {
                let message = self
                    .updater
                    .error()
                    .unwrap_or("The update check could not finish");
                panel = panel.child(widgets::error(
                    p,
                    &format!("Update check failed: {message}"),
                ));
            }
            UpdateStatus::Idle => {}
        }
        let busy = matches!(
            self.updater.status(),
            UpdateStatus::Checking | UpdateStatus::Installing
        );
        let mut check =
            widgets::button(p, "Check for updates", Tone::Secondary, !busy).self_start();
        if !busy {
            check = check.on_press(|s: &mut VoxenApp| {
                s.action_error = None;
                s.updater.check();
            });
        }
        panel.child(check)
    }

    fn save_bar(&self, p: &Palette) -> El {
        let mut bar = row()
            .key("savebar")
            .w_full()
            .py(14.0)
            .gap(10.0)
            .center_y()
            .fill(p.bg)
            .border_sides([HAIRLINE, 0.0, 0.0, 0.0], p.line);
        if let Some(error) = &self.save_error {
            bar = bar.child(widgets::error(p, error));
        }
        if self.saved {
            bar = bar.child(widgets::status(p, "Saved."));
        }
        let mut save = widgets::button(
            p,
            if self.saving {
                "Saving…"
            } else {
                "Save changes"
            },
            Tone::Primary,
            !self.saving,
        );
        if !self.saving {
            save = save.on_press(|s: &mut VoxenApp| s.save());
        }
        bar.child(rune::spacer()).child(save)
    }

    fn shortcut_display(&self) -> String {
        if self.shortcut_capture {
            return "Press your shortcut…".to_string();
        }
        match self.draft.shortcut.parse::<rune::Shortcut>() {
            Ok(shortcut) => {
                shortcut
                    .keycaps()
                    .join(if cfg!(target_os = "macos") { "" } else { "+" })
            }
            Err(_) => self.draft.shortcut.clone(),
        }
    }

    fn begin_shortcut_capture(&mut self) {
        if self.shortcut_capture {
            return;
        }
        self.shortcut_previous = self.draft.shortcut.clone();
        self.shortcut_error = None;
        match self.engine.set_shortcut_capture(true) {
            Ok(()) => self.shortcut_capture = true,
            Err(error) => self.shortcut_error = Some(error),
        }
    }

    fn end_shortcut_capture(&mut self) {
        if !self.shortcut_capture {
            return;
        }
        self.shortcut_capture = false;
        if let Err(error) = self.engine.set_shortcut_capture(false) {
            self.shortcut_error = Some(error);
        }
    }

    fn choose_microphone(&mut self, name: Option<String>) {
        self.draft.microphone = name;
        self.saved = false;
    }

    fn set_cleanup_mode(&mut self, mode: cleanup::Mode) {
        self.draft.cleanup_mode = mode;
        self.saved = false;
    }

    fn add_replacement(&mut self) {
        self.replacements.push(ReplacementRow {
            from: TextEdit::new(""),
            to: TextEdit::new(""),
        });
        self.saved = false;
    }

    fn remove_replacement(&mut self, index: usize) {
        if index < self.replacements.len() {
            self.replacements.remove(index);
            self.saved = false;
        }
    }

    fn install_update(&mut self) {
        self.action_error = None;
        self.updater.install();
    }

    fn collect_config(&self) -> Result<Config, String> {
        let unload = self.fields.unload.text.trim();
        let unload_after_seconds = if unload.is_empty() {
            0
        } else {
            unload
                .parse::<u64>()
                .map_err(|_| "Unload delay must be a whole number of seconds".to_string())?
        };
        let mut config = self.draft.clone();
        config.runtime_path = self.fields.runtime.text.clone();
        config.model_path = self.fields.model.text.clone();
        config.device = self.fields.device.text.clone();
        config.unload_after_seconds = unload_after_seconds;
        config.cleanup_runtime_path = self.fields.cleanup_runtime.text.clone();
        config.cleanup_model_path = self.fields.cleanup_model.text.clone();
        config.replacements = self
            .replacements
            .iter()
            .map(|row| Replacement {
                from: row.from.text.clone(),
                to: row.to.text.clone(),
            })
            .collect();
        Ok(config)
    }

    fn save(&mut self) {
        self.end_shortcut_capture();
        self.saving = true;
        self.save_error = None;
        self.saved = false;
        let config = match self.collect_config() {
            Ok(config) => config,
            Err(error) => {
                self.save_error = Some(error);
                self.saving = false;
                return;
            }
        };
        match self.engine.save_config(config) {
            Ok(snapshot) => {
                self.apply(snapshot);
                self.saved = true;
            }
            Err(error) => self.save_error = Some(error),
        }
        self.saving = false;
    }
}

fn mods_empty(mods: rune::Mods) -> bool {
    !mods.ctrl && !mods.alt && !mods.shift && !mods.logo
}

fn replacement_rows(config: &Config) -> Vec<ReplacementRow> {
    config
        .replacements
        .iter()
        .map(|row| ReplacementRow {
            from: TextEdit::new(row.from.clone()),
            to: TextEdit::new(row.to.clone()),
        })
        .collect()
}

fn phase_label(phase: &str) -> &str {
    match phase {
        "starting" => "Starting…",
        "recording" => "Recording",
        "transcribing" => "Transcribing…",
        "inserting" => "Inserting…",
        "error" => "Error",
        _ => "Idle",
    }
}

fn record_label(phase: &str) -> &str {
    match phase {
        "recording" => "Stop test",
        "starting" => "Starting…",
        "transcribing" => "Transcribing…",
        "inserting" => "Inserting…",
        _ => "Test microphone",
    }
}

fn panel_shell(key: &str, title: &str) -> El {
    col()
        .key(format!("panel-{key}"))
        .w_full()
        .gap(24.0)
        .role("tabpanel")
        .label(title)
}

fn field(p: &Palette, title: &str, control: El, note: Option<&str>) -> El {
    let mut group = col()
        .gap(6.0)
        .child(widgets::label(p, title))
        .child(control.label(title));
    if let Some(note) = note {
        group = group.child(widgets::hint(p, note));
    }
    group
}

fn field_input(
    p: &Palette,
    edit: &TextEdit,
    key: &str,
    placeholder: &str,
    access: impl for<'a> Fn(&'a mut VoxenApp) -> &'a mut TextEdit + 'static,
) -> El {
    widgets::text_field(p, edit, key, placeholder)
        .w_full()
        .on_edit(access)
        .on_input_change(|s: &mut VoxenApp, _: &mut Tx| s.saved = false)
}

fn transcript(p: &Palette, title: &str, key: &str, body: &str) -> El {
    widgets::card(p)
        .key(format!("transcript-{key}"))
        .child(widgets::eyebrow(p, title))
        .child(
            col()
                .key(format!("transcript-scroll-{key}"))
                .w_full()
                .max_h(128.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    text(body)
                        .font(theme::BODY)
                        .color(p.ink_soft)
                        .wrap()
                        .selectable(true),
                ),
        )
}

fn update_chip(p: &Palette, version: &str) -> El {
    let label = format!("Update {version} available");
    row()
        .role("button")
        .label(label.clone())
        .focusable(true)
        .h(24.0)
        .px(9.0)
        .center()
        .radius(7.0)
        .fill(p.accent_cyan.alpha(0.08))
        .border(HAIRLINE, p.accent_cyan.alpha(0.35))
        .hover(p.accent_cyan.alpha(0.14))
        .cursor(Cursor::Pointer)
        .child(
            text(label)
                .font(theme::SMALL)
                .color(p.accent_cyan)
                .a11y_hidden(),
        )
        .on_press(|s: &mut VoxenApp| s.tab = Tab::Updates)
}
