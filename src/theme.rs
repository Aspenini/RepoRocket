use anyhow::Result;
use std::fs;

use crate::state::Paths;

#[derive(Clone, Copy)]
pub struct ThemeColors {
    pub background: slint::Color,
    pub panel: slint::Color,
    pub surface: slint::Color,
    pub text: slint::Color,
    pub muted: slint::Color,
    pub accent: slint::Color,
    pub border: slint::Color,
    pub hover_surface: slint::Color,
    pub selected_surface: slint::Color,
    pub placeholder: slint::Color,
    pub progress_track: slint::Color,
    pub overlay: slint::Color,
    pub modal: slint::Color,
    pub danger: slint::Color,
    pub danger_text: slint::Color,
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self {
            background: rgb(16, 20, 25),
            panel: rgb(11, 15, 20),
            surface: rgb(23, 32, 43),
            text: rgb(246, 248, 251),
            muted: rgb(174, 184, 197),
            accent: rgb(47, 95, 143),
            border: rgb(40, 50, 65),
            hover_surface: rgb(32, 42, 55),
            selected_surface: rgb(40, 75, 114),
            placeholder: rgb(34, 43, 54),
            progress_track: rgb(32, 40, 50),
            overlay: rgb(5, 6, 7),
            modal: rgb(27, 34, 44),
            danger: rgb(214, 64, 64),
            danger_text: rgb(255, 90, 90),
        }
    }
}

impl ThemeColors {
    pub fn light() -> Self {
        Self {
            background: rgb(244, 247, 251),
            panel: rgb(29, 39, 52),
            surface: rgb(255, 255, 255),
            text: rgb(28, 36, 48),
            muted: rgb(95, 107, 123),
            accent: rgb(37, 109, 179),
            border: rgb(209, 218, 229),
            hover_surface: rgb(232, 239, 248),
            selected_surface: rgb(212, 231, 250),
            placeholder: rgb(226, 233, 242),
            progress_track: rgb(219, 226, 236),
            overlay: slint::Color::from_argb_u8(220, 10, 14, 20),
            modal: rgb(255, 255, 255),
            danger: rgb(196, 43, 43),
            danger_text: rgb(196, 43, 43),
        }
    }

    pub fn named(paths: &Paths, name: &str) -> Self {
        match name {
            "Default Dark" => Self::default(),
            "Default Light" => Self::light(),
            _ => load_custom(paths, name).unwrap_or_default(),
        }
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> slint::Color {
    slint::Color::from_rgb_u8(r, g, b)
}

pub fn load_themes(paths: &Paths) -> Vec<String> {
    let mut themes = vec!["Default Dark".into(), "Default Light".into()];
    let Ok(entries) = fs::read_dir(&paths.themes) else {
        return themes;
    };
    let mut custom = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| entry.path().join("theme.yaml").is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    custom.sort_unstable();
    themes.extend(custom);
    themes
}

fn load_custom(paths: &Paths, theme_name: &str) -> Result<ThemeColors> {
    let path = paths.themes.join(theme_name).join("theme.yaml");
    Ok(parse_theme_yaml(&fs::read_to_string(path)?))
}

fn parse_theme_yaml(text: &str) -> ThemeColors {
    let mut colors = ThemeColors::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = unquote(value.trim());
        let token = value.split_whitespace().next().unwrap_or(value);
        let Some(color) = parse_color(token) else {
            continue;
        };
        match key.trim() {
            "main-background" => colors.background = color,
            "panel-background" => colors.panel = color,
            "button-color" => colors.surface = color,
            "button-hover-color" => colors.hover_surface = color,
            "text-color" => colors.text = color,
            "accent-color" => colors.accent = color,
            "muted-text-color" => colors.muted = color,
            "border-color" => colors.border = color,
            "selected-surface-color" => colors.selected_surface = color,
            "placeholder-color" => colors.placeholder = color,
            "progress-track-color" => colors.progress_track = color,
            "overlay-color" => colors.overlay = color,
            "modal-color" => colors.modal = color,
            "danger-color" => colors.danger = color,
            "danger-text-color" => colors.danger_text = color,
            _ => {}
        }
    }
    colors
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value)
}

fn parse_color(value: &str) -> Option<slint::Color> {
    let hex = value.trim().strip_prefix('#')?;
    let (r, g, b, a) = match hex.len() {
        3 => (
            nibble(hex.as_bytes()[0])?,
            nibble(hex.as_bytes()[1])?,
            nibble(hex.as_bytes()[2])?,
            255,
        ),
        6 => (
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
            255,
        ),
        8 => (
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
            u8::from_str_radix(&hex[6..8], 16).ok()?,
        ),
        _ => return None,
    };
    Some(if a == 255 {
        slint::Color::from_rgb_u8(r, g, b)
    } else {
        slint::Color::from_argb_u8(a, r, g, b)
    })
}

fn nibble(ch: u8) -> Option<u8> {
    let value = match ch {
        b'0'..=b'9' => ch - b'0',
        b'a'..=b'f' => ch - b'a' + 10,
        b'A'..=b'F' => ch - b'A' + 10,
        _ => return None,
    };
    Some(value * 17)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_colors() {
        let color = parse_color("#2f5f8f").unwrap();
        assert_eq!(color.red(), 0x2f);
        assert_eq!(color.green(), 0x5f);
        assert_eq!(color.blue(), 0x8f);
        let short = parse_color("#abc").unwrap();
        assert_eq!(
            (short.red(), short.green(), short.blue()),
            (0xaa, 0xbb, 0xcc)
        );
    }

    #[test]
    fn parses_flat_theme_yaml() {
        let colors = parse_theme_yaml("accent-color: \"#112233\"\ntext-color: #fff\n");
        assert_eq!(colors.accent.red(), 0x11);
        assert_eq!(colors.text.red(), 0xff);
    }
}
