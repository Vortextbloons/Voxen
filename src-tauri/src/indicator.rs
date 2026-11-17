use rune::color::hex;
use rune::tok::TextStyle;
use rune::{col, el, text, Color, Cx, El, LoopSpec};

pub struct Indicator {
    pub phase: String,
    pub error: bool,
}

impl Indicator {
    pub fn new() -> Self {
        Indicator {
            phase: "idle".to_string(),
            error: false,
        }
    }

    pub fn set(&mut self, phase: &str, error: bool) {
        self.phase.clear();
        self.phase.push_str(phase);
        self.error = error;
    }
}

impl Default for Indicator {
    fn default() -> Self {
        Self::new()
    }
}

pub fn view(s: &Indicator, _cx: &mut Cx<Indicator>) -> El {
    let ink = hex("#f0f1f3").unwrap();
    let cyan = hex("#2fb4ff").unwrap();
    let violet = hex("#9b5cff").unwrap();
    let danger = hex("#f87171").unwrap();

    let accent = if s.error { danger } else { cyan };
    let rim = if s.error { danger } else { violet };

    let label = if s.error {
        "Error"
    } else {
        match s.phase.as_str() {
            "starting" => "Starting",
            "recording" => "Listening...",
            "transcribing" => "Transcribing",
            "inserting" => "Inserting",
            _ => "",
        }
    };

    let glow = el()
        .size(24.0, 24.0)
        .center()
        .radial_gradient(
            [0.5, 0.5],
            12.0,
            [
                (0.0, accent.alpha(0.38)),
                (0.45, accent.alpha(0.14)),
                (1.0, Color::TRANSPARENT),
            ],
        )
        .child(el().size(8.0, 8.0).radius(4.0).fill(accent));

    let glow = if s.phase == "recording" && !s.error {
        glow.looping(LoopSpec::opacity(1.5, 0.0, 0.6, 1.0))
    } else {
        glow
    };

    let pill = el()
        .h(52.0)
        .pl(16.0)
        .pr(20.0)
        .gap(10.0)
        .center_y()
        .radius(26.0)
        .continuous_corners()
        .fill(hex("#0b0d11e6").unwrap())
        .gradient(hex("#23283380").unwrap(), hex("#0b0d1100").unwrap())
        .border_gradient(1.0, rim.alpha(0.7), rim.alpha(0.22))
        .shadow(3.0, 8.0, hex("#00000059").unwrap())
        .child(glow)
        .child(
            text(label)
                .font(TextStyle::new(13.0, 500.0))
                .color(if s.error { danger } else { ink })
                .single_line(),
        );

    col().size(220.0, 76.0).center().child(pill)
}
