//! Semantic terminal palettes. System mode leaves surfaces to the terminal.
use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
    Mono,
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub background: Color,
    pub surface: Color,
    pub text: Color,
    pub muted: Color,
    pub border: Color,
    pub accent: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
}

impl Theme {
    pub const ALL: [Self; 4] = [Self::System, Self::Dark, Self::Light, Self::Mono];
    pub fn name(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::Mono => "Monochrome",
        }
    }
    pub fn palette(self) -> Palette {
        let rgb = |r, g, b| Color::Rgb(r, g, b);
        match self {
            Self::Dark => Palette {
                background: rgb(17, 19, 21),
                surface: rgb(27, 30, 34),
                text: rgb(231, 233, 236),
                muted: rgb(164, 171, 181),
                border: rgb(58, 66, 77),
                accent: rgb(121, 168, 242),
                success: rgb(124, 201, 154),
                warning: rgb(231, 189, 104),
                error: rgb(241, 139, 139),
            },
            Self::Light => Palette {
                background: rgb(250, 250, 249),
                surface: rgb(255, 255, 255),
                text: rgb(32, 36, 42),
                muted: rgb(89, 97, 110),
                border: rgb(205, 210, 217),
                accent: rgb(36, 94, 181),
                success: rgb(38, 119, 70),
                warning: rgb(136, 98, 13),
                error: rgb(181, 46, 61),
            },
            Self::System => Palette {
                background: Color::Reset,
                surface: Color::Reset,
                text: Color::Reset,
                muted: Color::Gray,
                border: Color::DarkGray,
                accent: Color::Cyan,
                success: Color::Green,
                warning: Color::Yellow,
                error: Color::Red,
            },
            Self::Mono => Palette {
                background: Color::Reset,
                surface: Color::Reset,
                text: Color::Reset,
                muted: Color::Reset,
                border: Color::Reset,
                accent: Color::Reset,
                success: Color::Reset,
                warning: Color::Reset,
                error: Color::Reset,
            },
        }
    }
}

/// Remove terminal commands while retaining ordinary Unicode and line breaks.
pub fn safe_text(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control() || *ch == '\n')
        .collect()
}
