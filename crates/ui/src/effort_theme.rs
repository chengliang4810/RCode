//! Device-local colours for the reasoning slider, independent of model options.

use gpui::Hsla;
use rcode_presentation::TextKey;
use serde::{Deserialize, Serialize};

use crate::{motion, theme::Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EffortTheme {
    Accent,
    #[default]
    Purple,
    Terracotta,
    DeepBlue,
    SkyBlue,
    Aurora,
    Silver,
    Gold,
}

impl EffortTheme {
    pub const ALL: [Self; 8] = [
        Self::Accent,
        Self::Purple,
        Self::Terracotta,
        Self::DeepBlue,
        Self::SkyBlue,
        Self::Aurora,
        Self::Silver,
        Self::Gold,
    ];

    pub fn label_key(self) -> TextKey {
        match self {
            Self::Accent => TextKey::SettingsAppearanceEffortAccent,
            Self::Purple => TextKey::SettingsAppearanceEffortPurple,
            Self::Terracotta => TextKey::SettingsAppearanceEffortTerracotta,
            Self::DeepBlue => TextKey::SettingsAppearanceEffortDeepBlue,
            Self::SkyBlue => TextKey::SettingsAppearanceEffortSkyBlue,
            Self::Aurora => TextKey::SettingsAppearanceEffortAurora,
            Self::Silver => TextKey::SettingsAppearanceEffortSilver,
            Self::Gold => TextKey::SettingsAppearanceEffortGold,
        }
    }

    pub fn palette(self, theme: &Theme) -> EffortPalette {
        let rgb = |hex| gpui::Hsla::from(gpui::rgb(hex));
        let colors = match self {
            Self::Accent => [theme.accent_strong, theme.accent, theme.accent_strong],
            Self::Purple => [rgb(0x8050ed), rgb(0x9b6df5), rgb(0x7143da)],
            Self::Terracotta => [rgb(0xb95a3e), rgb(0xd7805e), rgb(0xca6947)],
            Self::DeepBlue => [rgb(0x3152d9), rgb(0x637dfa), rgb(0x394bcb)],
            Self::SkyBlue => [rgb(0x006bd9), rgb(0x168df0), rgb(0x3aa4f2)],
            Self::Aurora => [rgb(0x4285f4), rgb(0x9b72cc), rgb(0xd96670)],
            Self::Silver => [rgb(0xffffff), rgb(0xe8e8ef), rgb(0xc9ccd8)],
            Self::Gold => [rgb(0xc78c18), rgb(0xf1bc45), rgb(0xe3a324)],
        };
        EffortPalette {
            colors,
            light: self == Self::Silver
                || (self == Self::Accent && crate::theme::relative_luminance(colors[1]) > 0.65),
        }
    }
}

#[derive(Clone, Copy)]
pub struct EffortPalette {
    pub colors: [Hsla; 3],
    pub light: bool,
}

impl EffortPalette {
    pub fn color_at(self, fraction: f32) -> Hsla {
        let position = fraction.clamp(0.0, 1.0) * 2.0;
        if position <= 1.0 {
            motion::mix(self.colors[0], self.colors[1], position)
        } else {
            motion::mix(self.colors[1], self.colors[2], position - 1.0)
        }
    }

    pub fn spark(self) -> Hsla {
        if self.light {
            gpui::black()
        } else {
            gpui::white()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::UiSettings;

    #[test]
    fn effort_theme_defaults_round_trips_and_survives_unrelated_settings_edits() {
        let legacy: UiSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.effort_theme, EffortTheme::Purple);
        let dir = tempfile::tempdir().unwrap();
        for preset in EffortTheme::ALL {
            let mut settings = legacy.clone();
            settings.effort_theme = preset;
            settings.save(dir.path()).unwrap();
            assert_eq!(UiSettings::load(dir.path()).effort_theme, preset);
        }
        let mut current = legacy.clone();
        current.effort_theme = EffortTheme::Aurora;
        let mut edited = legacy.clone();
        edited.sidebar_width = 310.0;
        let merged = UiSettings::merge_changes(&legacy, &edited, current);
        assert_eq!(merged.effort_theme, EffortTheme::Aurora);
        assert_eq!(merged.sidebar_width, 310.0);
        let mut edited = legacy.clone();
        edited.effort_theme = EffortTheme::Silver;
        assert_eq!(
            UiSettings::merge_changes(&legacy, &edited, merged).effort_theme,
            EffortTheme::Silver
        );
    }

    #[test]
    fn effort_palettes_have_readable_sparks_and_preserve_their_gradient() {
        let theme = Theme::dark();
        for preset in EffortTheme::ALL {
            let palette = preset.palette(&theme);
            assert_eq!(palette.color_at(-1.0), palette.colors[0]);
            assert_eq!(palette.color_at(0.5), palette.colors[1]);
            assert_eq!(palette.color_at(2.0), palette.colors[2]);
            assert_eq!(
                palette.spark(),
                if preset == EffortTheme::Silver {
                    gpui::black()
                } else {
                    gpui::white()
                }
            );
        }
        assert_ne!(
            EffortTheme::Aurora.palette(&theme).colors[0],
            EffortTheme::Aurora.palette(&theme).colors[2]
        );
        let mut other = theme.clone();
        other.accent = gpui::rgb(0xff0000).into();
        other.accent_strong = other.accent;
        assert_eq!(
            EffortTheme::Purple.palette(&theme).colors,
            EffortTheme::Purple.palette(&other).colors
        );
        assert_ne!(
            EffortTheme::Accent.palette(&theme).colors,
            EffortTheme::Accent.palette(&other).colors
        );
    }
}
