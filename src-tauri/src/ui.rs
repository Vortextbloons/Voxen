use crate::library::{self, Artifact, Kind, Library, ModelId, MODELS};
use crate::nemo;
use crate::theme::{self, Palette, HAIRLINE, RADIUS, SIDEBAR_WIDTH};
use crate::updater::{UpdateStatus, Updater};
use crate::widgets::{self, Tone};
use crate::{cleanup, context, AppSnapshot, Config, Engine, Replacement};
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
    excluded_apps: TextEdit,
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
            excluded_apps: TextEdit::new(config.excluded_apps.join(", ")),
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
    library: Library,
    selected_model: ModelId,
    devices: Vec<nemo::Device>,
    device_error: Option<String>,
    device_rx: Option<std::sync::mpsc::Receiver<(Vec<nemo::Device>, Option<String>)>>,
    revision: u64,
    loaded: bool,
}

impl VoxenApp {
    pub fn new(engine: Engine) -> Self {
        let snapshot = engine.snapshot();
        let revision = engine.revision();
        let (microphones, microphone_error) = match Engine::list_microphones() {
            Ok(names) => (names, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        let draft = snapshot.config.clone();
        let (device_tx, device_rx) = std::sync::mpsc::channel();
        let runtime_path = draft.runtime_path.clone();
        std::thread::spawn(move || {
            let _ = device_tx.send(detect_devices(&runtime_path));
        });
        let fields = Fields::from_config(&draft);
        let replacements = replacement_rows(&draft);
        let library = Library::new();
        let selected_model =
            library::model_from_path(&draft.model_path).unwrap_or(ModelId::Multilingual);
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
            library,
            selected_model,
            devices: vec![nemo::Device {
                id: "cpu".into(),
                label: "CPU".into(),
            }],
            device_error: None,
            device_rx: Some(device_rx),
            revision,
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
        let (library_changed, completed) = self.library.poll();
        if library_changed {
            changed = true;
        }
        if let Some(kind) = completed {
            self.installed(kind);
            changed = true;
        }
        if self.library.runtime.busy()
            || self.library.models.iter().any(|artifact| artifact.busy())
        {
            tx.wake_after(Duration::from_millis(250));
        }
        if self.device_rx.is_some() {
            match self.device_rx.as_ref().map(|rx| rx.try_recv()) {
                Some(Ok((devices, error))) => {
                    self.set_devices(devices, error);
                    self.device_rx = None;
                    changed = true;
                }
                Some(Err(std::sync::mpsc::TryRecvError::Empty)) => {
                    tx.wake_after(Duration::from_millis(25));
                }
                _ => self.device_rx = None,
            }
        }
        let revision = self.engine.revision();
        if revision != self.revision {
            self.revision = revision;
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
        panel_shell("model", "Model library")
            .child(widgets::intro(
                p,
                "Speech recognition runs locally on this machine. Install the official NVIDIA runtime and a matching model here, or point Voxen at files you already have under Advanced manual setup.",
            ))
            .child(self.language_field(p))
            .child(self.device_field(p))
            .child(self.runtime_card(p))
            .child(self.model_choice(p))
            .child(self.model_card(p))
            .child(self.sample_card(p))
            .child(self.advanced_model(p))
    }

    fn language_field(&self, p: &Palette) -> El {
        let mut list = col()
            .key("language-list")
            .role("listbox")
            .label("Language")
            .gap(4.0)
            .w_full()
            .max_h(156.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .scroll_edge_fade(6.0)
            .focus_group()
            .focus_selected(true);
        for (code, label) in nemo::LANGUAGES {
            let selected = self.draft.language == *code;
            let value = *code;
            list = list.child(
                widgets::option(p, label, selected)
                    .on_press(move |s: &mut VoxenApp| s.choose_language(value)),
            );
        }
        field(
            p,
            "Language",
            list,
            Some("Nemotron 3.5 covers 32 production locales. Greek, Latvian, Lithuanian, Maltese, and Slovenian are Parakeet-only here; other adaptation-only locales are not offered. Parakeet detects its 25 languages offline."),
        )
    }

    fn device_field(&self, p: &Palette) -> El {
        let mut list = col()
            .key("device-list")
            .role("listbox")
            .label("Compute device")
            .gap(4.0)
            .w_full()
            .focus_group()
            .focus_selected(true);
        for device in &self.devices {
            let selected = self.fields.device.text == device.id;
            let id = device.id.clone();
            list = list.child(
                widgets::option(p, &device.label, selected)
                    .on_press(move |s: &mut VoxenApp| s.choose_device(id.clone())),
            );
        }
        let mut refresh =
            widgets::button(p, "Refresh devices", Tone::Secondary, true).self_start();
        refresh = refresh.on_press(|s: &mut VoxenApp| s.refresh_devices());
        let mut group = col()
            .gap(6.0)
            .child(list)
            .child(refresh)
            .child(widgets::hint(
                p,
                "Vulkan devices need a working Vulkan 1.1+ driver. The managed runtime is the Vulkan build; CPU mode works without a dedicated GPU but is slower.",
            ));
        if let Some(error) = &self.device_error {
            group = group.child(widgets::hint(
                p,
                &format!("Devices could not be detected: {error}"),
            ));
        }
        let selected = self.fields.device.text.trim();
        if selected.starts_with("vulkan:")
            && !self.devices.iter().any(|device| device.id == selected)
        {
            group = group.child(widgets::hint(
                p,
                &format!("{selected} was not detected. Choose a listed device or keep it as a manual override."),
            ));
        }
        field(p, "Compute device", group, None)
    }

    fn runtime_card(&self, p: &Palette) -> El {
        let artifact = &self.library.runtime;
        let mut card = widgets::card(p)
            .child(widgets::eyebrow(p, "Recognition runtime"))
            .child(
                text(format!(
                    "NeMo-Speech.cpp {} · Vulkan x86_64 · {} · {}",
                    library::RUNTIME_VERSION,
                    library::size_label(library::RUNTIME_BYTES),
                    library::RUNTIME_LICENSE
                ))
                .font(theme::SMALL)
                .color(p.ink_soft)
                .wrap(),
            )
            .child(widgets::small(
                p,
                &format!("License: {} · {}", library::RUNTIME_LICENSE, library::RUNTIME_LICENSE_URL),
            ));
        card = artifact_state(p, card, artifact);
        if !self.fields.runtime.text.trim().is_empty()
            && self.fields.runtime.text.trim() != artifact.path.to_string_lossy().as_ref()
        {
            card = card.child(widgets::hint(
                p,
                "A manual runtime is configured below. This card manages the downloaded copy.",
            ));
        }
        let mut actions = row().gap(8.0).center_y();
        if artifact.busy() {
        } else if artifact.ready {
            if self.idle() {
                actions = actions.child(
                    widgets::button(p, "Remove managed runtime", Tone::Secondary, true)
                        .on_press(|s: &mut VoxenApp| s.remove_runtime()),
                );
            }
        } else {
            actions = actions.child(
                widgets::button(p, "Download runtime", Tone::Primary, true)
                    .on_press(|s: &mut VoxenApp| s.install_runtime()),
            );
        }
        card.child(actions)
    }

    fn model_choice(&self, p: &Palette) -> El {
        let mut list = col()
            .key("model-choice")
            .role("radiogroup")
            .label("Speech model")
            .gap(8.0)
            .w_full()
            .focus_group()
            .focus_selected(true);
        for spec in MODELS {
            let selected = self.selected_model == spec.id;
            let id = spec.id;
            list = list.child(
                widgets::choice(p, spec.label, spec.detail, selected)
                    .on_press(move |s: &mut VoxenApp| s.choose_model(id)),
            );
        }
        field(
            p,
            "Speech model",
            list,
            Some("Nemotron models stream live; Parakeet is offline only. Use Nemotron English for English-only dictation and Nemotron 3.5 for the other supported locales."),
        )
    }

    fn model_card(&self, p: &Palette) -> El {
        let spec = library::model_spec(self.selected_model);
        let artifact = self.library.model(spec.id);
        let path = artifact.path.to_string_lossy().to_string();
        let current = self.fields.model.text.trim() == path.as_str();
        let mut card = widgets::card(p)
            .child(widgets::eyebrow(p, "Selected model"))
            .child(
                text(format!("{} · Q8_0 · {}", spec.label, library::size_label(spec.bytes)))
                    .font(theme::BODY)
                    .color(p.ink)
                    .wrap(),
            )
            .child(widgets::small(p, spec.detail))
            .child(widgets::small(
                p,
                &format!("License: {} · {}", spec.license, spec.license_url),
            ));
        card = artifact_state(p, card, artifact);
        if current {
            card = card.child(widgets::status(p, "In use."));
        } else if !self.fields.model.text.trim().is_empty() {
            card = card.child(widgets::hint(
                p,
                "A manual model is configured below. This card manages the downloaded copy.",
            ));
        }
        if spec.id == ModelId::Parakeet && !nemo::parakeet_supports(&self.draft.language) {
            card = card.child(widgets::hint(
                p,
                "Parakeet does not support this language. Choose the multilingual Nemotron model.",
            ));
        }
        if !spec.streaming {
            card = card.child(widgets::hint(
                p,
                "No live typing with this model. The transcript inserts after you stop recording.",
            ));
        }
        if !spec.boosting {
            let reason = if spec.id == ModelId::Parakeet {
                "Parakeet TDT does not support NVIDIA word boosting."
            } else {
                "This pinned GGUF has no embedded SentencePiece tokenizer, so NVIDIA word boosting cannot use technical context."
            };
            card = card.child(widgets::hint(
                p,
                &format!(
                    "{reason} With Light or Polished cleanup, context is only a limited spelling and casing reference; Raw mode reads no context."
                ),
            ));
        }
        if spec.id == ModelId::English && !nemo::english_supports(&self.draft.language) {
            card = card.child(widgets::hint(
                p,
                "This English model does not support the selected language. Choose the multilingual Nemotron model.",
            ));
        }
        if spec.id == ModelId::Multilingual && !nemo::nemotron_supports(&self.draft.language) {
            card = card.child(widgets::hint(
                p,
                "Nemotron 3.5 does not offer this locale out of the box. Choose Parakeet for this language.",
            ));
        }
        let mut actions = row().gap(8.0).center_y();
        if artifact.busy() {
        } else if artifact.ready {
            if !current {
                actions = actions.child(
                    widgets::button(p, "Use this model", Tone::Primary, true)
                        .on_press(move |s: &mut VoxenApp| s.use_model(spec.id)),
                );
            }
            if self.idle() {
                actions = actions.child(
                    widgets::button(p, "Remove managed model", Tone::Secondary, true)
                        .on_press(move |s: &mut VoxenApp| s.remove_model(spec.id)),
                );
            }
        } else {
            actions = actions.child(
                widgets::button(p, "Download model", Tone::Primary, true)
                    .on_press(move |s: &mut VoxenApp| s.install_model(spec.id)),
            );
        }
        card.child(actions)
    }

    fn sample_card(&self, p: &Palette) -> El {
        let complete = self
            .snapshot
            .last_transcript
            .as_ref()
            .is_some_and(|text| !text.trim().is_empty());
        let mut card = widgets::card(p)
            .child(widgets::eyebrow(p, "Sample dictation"))
            .child(widgets::hint(
                p,
                "Use Test microphone, say one sentence, and stop. The transcript stays in the sidebar and is not inserted into another app.",
            ));
        card = if complete {
            card.child(widgets::status(
                p,
                "Dictation completed. Setup is ready for the global shortcut.",
            ))
        } else {
            card.child(widgets::hint(p, "Waiting for a completed sample."))
        };
        let mut test = widgets::button(
            p,
            record_label(&self.snapshot.phase),
            if self.snapshot.phase == "recording" {
                Tone::Danger
            } else {
                Tone::Primary
            },
            self.record_enabled(),
        )
        .self_start();
        if self.record_enabled() {
            test = test.on_press(|s: &mut VoxenApp| s.toggle_recording());
        }
        card.child(test)
    }

    fn advanced_model(&self, p: &Palette) -> El {
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
        col()
            .key("advanced-model")
            .gap(18.0)
            .w_full()
            .child(widgets::eyebrow(p, "Advanced manual setup"))
            .child(widgets::hint(
                p,
                "Use these paths for an existing runtime or model, a custom device, or a runtime downloaded elsewhere. Managed installs fill them automatically.",
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
                Some("Device identifier used by the runtime, for example cpu or vulkan:0."),
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
        let mut context_modes = col().gap(8.0);
        for (mode, title, description) in [
            (
                context::Mode::Off,
                "Off",
                "Read nothing from the target application.",
            ),
            (
                context::Mode::Selection,
                "Selected text",
                "Use only explicitly selected text as context.",
            ),
            (
                context::Mode::Nearby,
                "Selected or nearby text",
                "Use selected text, or up to 400 characters either side of the caret when nothing is selected.",
            ),
        ] {
            let selected = self.draft.context_mode == mode;
            context_modes = context_modes.child(
                widgets::choice(p, title, description, selected)
                    .on_press(move |s: &mut VoxenApp| s.set_context_mode(mode)),
            );
        }
        let excluded = field_input(
            p,
            &self.fields.excluded_apps,
            "context-exclusions",
            "KeePassXC.exe, 1Password.exe",
            |s: &mut VoxenApp| &mut s.fields.excluded_apps,
        );
        let context = col()
            .key("technical-context")
            .gap(10.0)
            .w_full()
            .child(widgets::eyebrow(p, "Technical context"))
            .child(widgets::hint(
                p,
                "Read once with Windows accessibility when recording starts, before the indicator appears. Context is bounded to 800 characters, skipped in password fields, and never uses screenshots, clipboard access, or cloud requests. Word hints require NVIDIA's tokenizer-equipped Nemotron 3.5 GGUF; the pinned Nemotron English GGUF has no embedded tokenizer and Parakeet TDT has no word boosting, so Raw mode reads no context with either. With Light or Polished cleanup, context is only a limited spelling and casing reference: the cleanup guard preserves every word, so it can adjust capitalization and punctuation but cannot replace a misspelled word or introduce a technical term.",
            ))
            .child(context_modes)
            .child(field(
                p,
                "Excluded applications",
                excluded,
                Some("Comma-separated executable names. These apps are skipped before any context is read."),
            ));
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
            .child(context)
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

    fn set_context_mode(&mut self, mode: context::Mode) {
        self.draft.context_mode = mode;
        self.saved = false;
    }

    fn choose_language(&mut self, language: &'static str) {
        self.draft.language = language.to_string();
        let supported = match self.selected_model {
            ModelId::Multilingual => nemo::nemotron_supports(language),
            ModelId::English => nemo::english_supports(language),
            ModelId::Parakeet => nemo::parakeet_supports(language),
        };
        if !supported {
            self.selected_model =
                if nemo::parakeet_supports(language) && !nemo::nemotron_supports(language) {
                    ModelId::Parakeet
                } else {
                    ModelId::Multilingual
                };
        }
        self.saved = false;
    }

    fn choose_model(&mut self, model: ModelId) {
        self.selected_model = model;
    }

    fn choose_device(&mut self, device: String) {
        self.fields.device = TextEdit::new(device.clone());
        self.draft.device = device;
        self.saved = false;
    }

    fn idle(&self) -> bool {
        matches!(self.snapshot.phase.as_str(), "idle" | "error")
    }

    fn refresh_devices(&mut self) {
        let (devices, error) = detect_devices(self.fields.runtime.text.trim());
        self.set_devices(devices, error);
    }

    fn set_devices(&mut self, devices: Vec<nemo::Device>, error: Option<String>) {
        self.devices = devices;
        self.device_error = error;
        if !self
            .devices
            .iter()
            .any(|device| device.id == self.fields.device.text.trim())
        {
            let device = self
                .devices
                .iter()
                .find(|device| device.id.starts_with("vulkan:"))
                .map(|device| device.id.clone())
                .unwrap_or_else(|| "cpu".into());
            self.choose_device(device);
        }
    }

    fn install_runtime(&mut self) {
        self.action_error = None;
        let _ = self.engine.unload_model();
        self.library.install(Kind::Runtime);
    }

    fn install_model(&mut self, model: ModelId) {
        self.action_error = None;
        let _ = self.engine.unload_model();
        self.library.install(Kind::Model(model));
    }

    fn use_model(&mut self, model: ModelId) {
        self.selected_model = model;
        let path = self.library.model(model).path.to_string_lossy().to_string();
        self.fields.model = TextEdit::new(path.clone());
        self.draft.model_path = path;
        self.saved = false;
        self.action_error = None;
        if self.idle() {
            self.save();
        }
    }

    fn installed(&mut self, kind: Kind) {
        let path = match kind {
            Kind::Runtime => &self.library.runtime.path,
            Kind::Model(id) => &self.library.model(id).path,
        }
        .to_string_lossy()
        .to_string();
        match kind {
            Kind::Runtime => {
                self.fields.runtime = TextEdit::new(path.clone());
                self.draft.runtime_path = path;
                self.refresh_devices();
                if let Some(device) = self
                    .devices
                    .iter()
                    .find(|device| device.id.starts_with("vulkan:"))
                {
                    self.choose_device(device.id.clone());
                }
            }
            Kind::Model(id) => {
                self.selected_model = id;
                self.fields.model = TextEdit::new(path.clone());
                self.draft.model_path = path;
            }
        }
        self.saved = false;
        self.action_error = None;
        if self.idle() {
            self.save();
        } else {
            self.save_error =
                Some("Installed. Save changes after the current dictation finishes.".into());
        }
    }

    fn remove_runtime(&mut self) {
        self.action_error = None;
        let path = self.library.runtime.path.to_string_lossy().to_string();
        match self.engine.remove_managed_runtime(&path) {
            Ok(()) => {
                self.library.mark_missing(Kind::Runtime);
                if self.fields.runtime.text.trim() == path.as_str() {
                    self.fields.runtime = TextEdit::new("");
                    self.draft.runtime_path.clear();
                }
                self.refresh_devices();
                self.saved = false;
                if self.idle() {
                    self.save();
                }
            }
            Err(error) => {
                self.action_error = Some(format!(
                    "{error}. Restart Voxen before removing a runtime that is in use."
                ));
            }
        }
    }

    fn remove_model(&mut self, model: ModelId) {
        self.action_error = None;
        let path = self.library.model(model).path.to_string_lossy().to_string();
        match self.engine.remove_managed_model(&path) {
            Ok(()) => {
                self.library.mark_missing(Kind::Model(model));
                if self.fields.model.text.trim() == path.as_str() {
                    self.fields.model = TextEdit::new("");
                    self.draft.model_path.clear();
                }
                self.saved = false;
                if self.idle() {
                    self.save();
                }
            }
            Err(error) => self.action_error = Some(error),
        }
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
        config.excluded_apps = self
            .fields
            .excluded_apps
            .text
            .split([',', ';'])
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect();
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

fn detect_devices(runtime_path: &str) -> (Vec<nemo::Device>, Option<String>) {
    let cpu = nemo::Device {
        id: "cpu".into(),
        label: "CPU".into(),
    };
    if runtime_path.trim().is_empty() {
        return (vec![cpu], None);
    }
    match Engine::list_devices(runtime_path) {
        Ok(devices) if !devices.is_empty() => (devices, None),
        Ok(_) => (vec![cpu], None),
        Err(error) => (vec![cpu], Some(error)),
    }
}

fn artifact_state(p: &Palette, mut card: El, artifact: &Artifact) -> El {
    if let Some(phase) = &artifact.phase {
        card = card
            .child(widgets::label(p, phase))
            .child(widgets::progress(p, artifact.progress));
    } else if artifact.ready {
        card = card
            .child(widgets::status(p, "Managed copy installed and ready."))
            .child(widgets::small(p, &artifact.path.display().to_string()));
    } else {
        card = card.child(widgets::hint(p, "Managed copy not installed."));
    }
    if let Some(error) = &artifact.error {
        card = card.child(widgets::error(p, error));
    }
    card
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
