use crate::theme::{self, Palette, HAIRLINE, RADIUS};
use rune::{col, el, row, text, text_input, Color, Cursor, El, TextEdit};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Primary,
    Secondary,
    Danger,
}

pub fn text_field(p: &Palette, edit: &TextEdit, key: &str, placeholder: &str) -> El {
    text_input(edit, theme::BODY)
        .key(key)
        .max_w(460.0)
        .h(36.0)
        .px(10.0)
        .py(8.0)
        .radius(RADIUS)
        .fill(p.surface)
        .border(HAIRLINE, p.line)
        .color(p.ink)
        .caret(p.accent, p.accent_soft)
        .placeholder(placeholder, p.ink_faint)
        .single_line()
}

pub fn label(p: &Palette, value: &str) -> El {
    text(value).font(theme::LABEL).color(p.ink)
}

pub fn hint(p: &Palette, value: &str) -> El {
    text(value).font(theme::HINT).color(p.ink_faint).wrap()
}

pub fn small(p: &Palette, value: &str) -> El {
    text(value).font(theme::SMALL).color(p.ink_faint).wrap()
}

pub fn intro(p: &Palette, value: &str) -> El {
    text(value)
        .font(theme::HINT)
        .color(p.ink_faint)
        .wrap()
        .max_w(560.0)
}

pub fn error(p: &Palette, value: &str) -> El {
    text(value)
        .font(theme::HINT)
        .color(p.danger)
        .wrap()
        .role("alert")
}

pub fn status(p: &Palette, value: &str) -> El {
    text(value)
        .font(theme::HINT)
        .color(p.ok)
        .wrap()
        .role("status")
}

pub fn eyebrow(p: &Palette, value: &str) -> El {
    text(value)
        .font(theme::EYEBROW)
        .color(p.ink_faint)
        .text_case(rune::TextCase::Uppercase)
}

pub fn button(p: &Palette, label: &str, tone: Tone, enabled: bool) -> El {
    let (fill, border, ink, hover) = match tone {
        Tone::Primary => (p.accent, p.accent, Color::WHITE, p.accent_hover),
        Tone::Danger => (
            p.danger_soft,
            p.danger.alpha(0.35),
            p.danger,
            p.danger_soft.alpha(0.7),
        ),
        Tone::Secondary => (p.surface, p.line, p.ink, p.surface_hover),
    };
    let mut control = row()
        .role("button")
        .label(label)
        .focusable(enabled)
        .disabled(!enabled)
        .h(34.0)
        .px(14.0)
        .center()
        .radius(RADIUS)
        .fill(fill)
        .border(HAIRLINE, border)
        .child(text(label).font(theme::BUTTON).color(ink).a11y_hidden());
    if enabled {
        control = control.cursor(Cursor::Pointer).hover(hover);
    } else {
        control = control.opacity(0.45);
    }
    control
}

pub fn tab(p: &Palette, label: &str, selected: bool) -> El {
    el().role("tab")
        .label(label)
        .checked(selected)
        .focusable(true)
        .h(38.0)
        .px(2.0)
        .justify_center()
        .center_y()
        .border_sides(
            [0.0, 0.0, 2.0, 0.0],
            if selected {
                p.accent
            } else {
                Color::TRANSPARENT
            },
        )
        .cursor(Cursor::Pointer)
        .child(
            text(label)
                .font(theme::LABEL)
                .color(if selected { p.ink } else { p.ink_faint })
                .a11y_hidden(),
        )
}

pub fn choice(p: &Palette, title: &str, description: &str, selected: bool) -> El {
    row()
        .role("radio")
        .label(title)
        .checked(selected)
        .focusable(true)
        .w_full()
        .max_w(460.0)
        .padding(12.0)
        .gap(10.0)
        .radius(RADIUS)
        .fill(if selected { p.accent_soft } else { p.surface })
        .border(HAIRLINE, if selected { p.accent } else { p.line })
        .hover(if selected {
            p.accent_soft
        } else {
            p.surface_hover
        })
        .cursor(Cursor::Pointer)
        .child(radio_mark(p, selected))
        .child(
            col()
                .grow(1.0)
                .gap(2.0)
                .child(text(title).font(theme::LABEL).color(p.ink).a11y_hidden())
                .child(
                    text(description)
                        .font(theme::SMALL)
                        .color(p.ink_faint)
                        .wrap()
                        .a11y_hidden(),
                ),
        )
}

fn radio_mark(p: &Palette, selected: bool) -> El {
    el().size(15.0, 15.0)
        .shrink()
        .center()
        .radius(7.5)
        .fill(if selected {
            p.accent
        } else {
            Color::TRANSPARENT
        })
        .border(1.5, if selected { p.accent } else { p.line_strong })
        .a11y_hidden()
        .child(el().size(5.0, 5.0).radius(2.5).fill(if selected {
            Color::WHITE
        } else {
            Color::TRANSPARENT
        }))
}

pub fn option(p: &Palette, label: &str, selected: bool) -> El {
    row()
        .role("option")
        .label(label)
        .checked(selected)
        .focusable(true)
        .w_full()
        .max_w(460.0)
        .h(36.0)
        .px(12.0)
        .center_y()
        .radius(RADIUS)
        .fill(if selected { p.accent_soft } else { p.surface })
        .border(HAIRLINE, if selected { p.accent } else { p.line })
        .hover(if selected {
            p.accent_soft
        } else {
            p.surface_hover
        })
        .cursor(Cursor::Pointer)
        .child(
            text(label)
                .font(theme::BODY)
                .color(if selected { p.ink } else { p.ink_soft })
                .truncate()
                .a11y_hidden(),
        )
}

pub fn card(p: &Palette) -> El {
    col()
        .w_full()
        .padding(12.0)
        .gap(6.0)
        .radius(RADIUS)
        .fill(p.surface)
        .border(HAIRLINE, p.line)
}

pub fn progress(p: &Palette, percent: u8) -> El {
    row()
        .key("progress")
        .role("progressbar")
        .range_value(f64::from(percent), 0.0, 100.0)
        .w_full()
        .max_w(460.0)
        .h(8.0)
        .radius(4.0)
        .fill(p.line)
        .child(
            el().w_frac(f32::from(percent) / 100.0)
                .h_full()
                .radius(4.0)
                .fill(p.accent),
        )
}
