use rune::tok::TextStyle;
use rune::Color;

pub const RADIUS: f32 = 10.0;
pub const SIDEBAR_WIDTH: f32 = 244.0;
pub const HAIRLINE: f32 = 1.0;

pub const BRAND: TextStyle = TextStyle::new(15.0, 650.0).lh(18.0);
pub const PAGE_TITLE: TextStyle = TextStyle::new(21.0, 600.0).lh(25.0).tracking(-0.3);
pub const PAGE_SUBTITLE: TextStyle = TextStyle::new(12.5, 400.0).lh(19.0);
pub const BODY: TextStyle = TextStyle::new(12.5, 400.0).lh(19.0);
pub const LABEL: TextStyle = TextStyle::new(12.5, 500.0).lh(18.0);
pub const BUTTON: TextStyle = TextStyle::new(12.5, 500.0).lh(16.0);
pub const BUTTON_STRONG: TextStyle = TextStyle::new(12.5, 600.0).lh(16.0);
pub const CHIP: TextStyle = TextStyle::new(12.0, 500.0).lh(15.0);
pub const HINT: TextStyle = TextStyle::new(12.0, 400.0).lh(19.0);
pub const SMALL: TextStyle = TextStyle::new(11.5, 400.0).lh(17.0);
pub const EYEBROW: TextStyle = TextStyle::new(10.5, 600.0).lh(13.0).tracking(0.6);

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color,
    pub surface: Color,
    pub surface_soft: Color,
    pub surface_hover: Color,
    pub ink: Color,
    pub ink_soft: Color,
    pub ink_faint: Color,
    pub line: Color,
    pub line_strong: Color,
    pub accent: Color,
    pub accent_hover: Color,
    pub accent_soft: Color,
    pub accent_cyan: Color,
    pub accent_blue: Color,
    pub accent_violet: Color,
    pub danger: Color,
    pub danger_soft: Color,
    pub ok: Color,
}

impl Palette {
    pub fn dark() -> Self {
        Self {
            bg: hex("#17191d"),
            surface: hex("#202328"),
            surface_soft: hex("#131519"),
            surface_hover: hex("#292d34"),
            ink: hex("#f0f1f3"),
            ink_soft: hex("#b4bac4"),
            ink_faint: hex("#969eab"),
            line: hex("#30343c"),
            line_strong: hex("#454c58"),
            accent: hex("#648cf5"),
            accent_hover: hex("#7b9ef8"),
            accent_soft: hex("#26334e"),
            accent_cyan: hex("#2fb4ff"),
            accent_blue: hex("#3b82f6"),
            accent_violet: hex("#9b5cff"),
            danger: hex("#f87171"),
            danger_soft: hex("#2a181c"),
            ok: hex("#4ade80"),
        }
    }
}

fn hex(value: &str) -> Color {
    rune::color::hex(value).unwrap()
}
