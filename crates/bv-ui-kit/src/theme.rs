//! Theme resolution (FR-004): from the Windows appearance preferences to the
//! colours every control uses.
//!
//! Colour values are the Windows 11 design resources (WinUI 3 "Fluent" theme dictionary),
//! written in their `#AARRGGBB` notation so they can be compared with the source.

use crate::Color;

/// Light or dark appearance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeMode {
    /// Light.
    #[default]
    Light,
    /// Dark.
    Dark,
}

/// The accent colour and its shades, as Windows computes them from the user's choice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AccentPalette {
    /// Lightest shade.
    pub light3: Color,
    /// Second lightest shade (accent fills in dark mode).
    pub light2: Color,
    /// Light shade.
    pub light1: Color,
    /// The accent colour itself.
    pub base: Color,
    /// Dark shade (accent fills in light mode).
    pub dark1: Color,
    /// Darker shade.
    pub dark2: Color,
    /// Darkest shade.
    pub dark3: Color,
}

impl AccentPalette {
    /// The Windows 11 default accent (blue), used when Windows gives none.
    pub const DEFAULT: AccentPalette = AccentPalette {
        light3: Color::rgb(0x99EBFF),
        light2: Color::rgb(0x60CDFF),
        light1: Color::rgb(0x0093F9),
        base: Color::rgb(0x0078D4),
        dark1: Color::rgb(0x005FB8),
        dark2: Color::rgb(0x004275),
        dark3: Color::rgb(0x002642),
    };
}

/// The colours of the active high-contrast theme (the Windows "system colours").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HighContrastColors {
    /// Background of windows.
    pub window: Color,
    /// Text on that background.
    pub window_text: Color,
    /// Background of selected or hovered items.
    pub highlight: Color,
    /// Text of selected or hovered items.
    pub highlight_text: Color,
    /// Disabled text.
    pub gray_text: Color,
    /// Background of buttons.
    pub button_face: Color,
    /// Text of buttons.
    pub button_text: Color,
    /// Hyperlinks.
    pub hotlight: Color,
}

/// The Windows appearance preferences from which a theme is resolved. The shell reads
/// them from the system and passes them here whenever they change; the design system
/// never queries Windows itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeInputs {
    /// Mode of the Windows shell (taskbar, system flyouts): the tray icon, the flyout,
    /// the menu and the bubble follow it, like the system surfaces they sit next to.
    pub system_mode: ThemeMode,
    /// Mode chosen for applications: the settings window follows it.
    pub apps_mode: ThemeMode,
    /// The user's accent colour, when Windows provides one.
    pub accent: Option<AccentPalette>,
    /// The high-contrast colours, when a high-contrast theme is active.
    pub high_contrast: Option<HighContrastColors>,
    /// Windows "Transparency effects" are on (translucent materials allowed).
    pub transparency: bool,
    /// Windows "Animation effects" are on (off = the user asked to reduce motion).
    pub animations: bool,
    /// Windows text scaling factor (1.0 = 100 %, up to 2.25).
    pub text_scale: f32,
}

impl Default for ThemeInputs {
    fn default() -> Self {
        Self {
            system_mode: ThemeMode::Dark,
            apps_mode: ThemeMode::Light,
            accent: None,
            high_contrast: None,
            transparency: true,
            animations: true,
            text_scale: 1.0,
        }
    }
}

/// Background of a popup surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Material {
    /// The Windows translucent material (blurred backdrop drawn by the system), with a
    /// tint layer painted on top to match the system flyouts.
    Acrylic {
        /// Colour painted over the system backdrop.
        tint: Color,
        /// Opaque colour used instead when Windows cannot draw the backdrop.
        fallback: Color,
    },
    /// An opaque background (transparency effects off, or high contrast).
    Solid(Color),
}

/// Every colour a control may use, resolved for one surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    /// Light or dark.
    pub mode: ThemeMode,
    /// A high-contrast theme is active (controls then draw visible outlines).
    pub high_contrast: bool,
    /// Main text and icons.
    pub text_primary: Color,
    /// Secondary text.
    pub text_secondary: Color,
    /// Disabled text and icons.
    pub text_disabled: Color,
    /// Accent fill (selected tile, slider track, primary button).
    pub accent_fill: Color,
    /// Accent fill under the pointer.
    pub accent_fill_hover: Color,
    /// Text and icons drawn on the accent fill.
    pub text_on_accent: Color,
    /// Background of a subtle control (menu entry, list row) under the pointer.
    pub subtle_hover: Color,
    /// Background of a subtle control while pressed.
    pub subtle_pressed: Color,
    /// Text of a subtle control under the pointer (differs only in high contrast).
    pub subtle_hover_text: Color,
    /// Separator lines.
    pub divider: Color,
    /// Outline of popup surfaces.
    pub surface_stroke: Color,
    /// Outer ring of the keyboard focus indicator.
    pub focus_outer: Color,
    /// Inner ring of the keyboard focus indicator.
    pub focus_inner: Color,
    /// Background of popup surfaces (flyout, menu).
    pub material: Material,
    /// Colours of the tray icon: they follow the taskbar (the Windows system mode) on
    /// every surface, so the settings window can show the icon as it looks there.
    pub tray: TrayColors,
    /// Colours of the flyout controls (tiles, sliders, tabs, footer, tooltip).
    pub controls: ControlColors,
    /// Colours of application windows (settings window).
    pub window: WindowColors,
    /// Windows text scaling factor (1.0 = 100 %).
    pub text_scale: f32,
    /// Whether transitions are animated (Windows "Animation effects").
    pub animations: bool,
}

/// Colours of the tray icon (FR-010, FR-011, FR-084), for the taskbar it sits on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrayColors {
    /// The Windows look: the glyph (white on a dark taskbar, black on a light one).
    pub glyph: Color,
    /// The Windows look: waves not reached (the Windows 11 "disabled text" colour).
    pub unreached: Color,
    /// The colourful look: its gradient, top to bottom, made of accent shades (lighter
    /// on a dark taskbar).
    pub accent: [Color; 2],
    /// The colourful look: waves not reached, a pale accent.
    pub accent_unreached: Color,
    /// The taskbar behind the icon (pictures of the icon in the settings window).
    pub taskbar: Color,
    /// A high-contrast theme is active: only system colours, so the colourful look
    /// is drawn in the Windows look.
    pub high_contrast: bool,
}

impl TrayColors {
    /// The colours for the taskbar of `inputs` (its system mode, accent and contrast).
    pub fn for_taskbar(inputs: &ThemeInputs) -> TrayColors {
        if let Some(hc) = inputs.high_contrast {
            return TrayColors {
                glyph: hc.window_text,
                unreached: hc.gray_text,
                accent: [hc.window_text; 2],
                accent_unreached: hc.gray_text,
                taskbar: hc.window,
                high_contrast: true,
            };
        }
        let a = inputs.accent.unwrap_or(AccentPalette::DEFAULT);
        match inputs.system_mode {
            ThemeMode::Dark => TrayColors {
                glyph: Color::WHITE,
                unreached: Color::argb(0x5DFF_FFFF),
                accent: [a.light3, a.light1],
                accent_unreached: a.light2.with_alpha(0.32),
                taskbar: Color::rgb(0x1C1C1C),
                high_contrast: false,
            },
            ThemeMode::Light => TrayColors {
                glyph: Color::argb(0xE400_0000),
                unreached: Color::argb(0x5C00_0000),
                accent: [a.dark1, a.dark3],
                accent_unreached: a.dark1.with_alpha(0.32),
                taskbar: Color::rgb(0xEEEEEE),
                high_contrast: false,
            },
        }
    }
}

/// Colours of the flyout controls: WinUI 3 control fills, strokes and slider resources.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlColors {
    /// Background of a control at rest (device tile, selected tab).
    pub fill: Color,
    /// Background of a control under the pointer.
    pub fill_hover: Color,
    /// Background of a control while pressed.
    pub fill_pressed: Color,
    /// Background of a disabled control.
    pub fill_disabled: Color,
    /// Text of a control under the pointer or pressed (differs only in high contrast).
    pub hover_text: Color,
    /// Outline of a control.
    pub stroke: Color,
    /// Accent fill while pressed.
    pub accent_pressed: Color,
    /// Accent fill of a disabled control.
    pub accent_disabled: Color,
    /// Background of a container of choices (the tab bar).
    pub container: Color,
    /// Slider track (the part not filled by the value).
    pub slider_track: Color,
    /// Outer disc of the slider thumb.
    pub slider_thumb: Color,
    /// Outline of the slider thumb.
    pub slider_thumb_stroke: Color,
    /// Band at the bottom of the flyout.
    pub footer: Color,
    /// Opaque background of tooltips and small badges.
    pub solid: Color,
    /// Outline of tooltips.
    pub solid_stroke: Color,
    /// Thin scroll indicator.
    pub scroll_indicator: Color,
}

/// Colours of application windows: WinUI 3 resources of the window background, setting
/// cards, toggle switches and hyperlinks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowColors {
    /// The window shows the Windows "Mica" material (transparency effects on, no high
    /// contrast); otherwise [`base`](Self::base) is painted.
    pub mica: bool,
    /// Opaque background of the window (also behind Mica when Windows refuses it).
    pub base: Color,
    /// Background of a setting card.
    pub card: Color,
    /// Outline of a setting card.
    pub card_stroke: Color,
    /// Track of a toggle switch that is off.
    pub toggle_off_fill: Color,
    /// Track of a toggle switch that is off, under the pointer.
    pub toggle_off_fill_hover: Color,
    /// Outline of a toggle switch that is off (and its knob).
    pub toggle_off_stroke: Color,
    /// Hyperlink text.
    pub link: Color,
    /// Hyperlink text under the pointer.
    pub link_hover: Color,
    /// Shadow-like outline around drop-down lists.
    pub popup_stroke: Color,
    /// Error text (a refused name).
    pub critical: Color,
    /// Background of a text box being edited.
    pub input_focused: Color,
    /// Bottom line of a text box at rest.
    pub input_line: Color,
    /// Background of a notice (WinUI InfoBar), by severity: information, success,
    /// caution, critical.
    pub notice_background: [Color; 4],
    /// Circle behind the severity glyph of a notice, by severity.
    pub notice_icon: [Color; 4],
    /// The severity glyph drawn on that circle.
    pub notice_glyph: Color,
    /// Veil over the window behind a dialog.
    pub smoke: Color,
}

impl Theme {
    /// The same theme with an opaque material (Windows refused the translucent one).
    pub fn opaque(self) -> Theme {
        match self.material {
            Material::Acrylic { fallback, .. } => {
                Theme { material: Material::Solid(fallback), ..self }
            }
            Material::Solid(_) => self,
        }
    }

    /// The theme of the shell surfaces: tray icon, flyout, context menu, bubble.
    pub fn shell(inputs: &ThemeInputs) -> Theme {
        Theme::resolve(inputs, inputs.system_mode)
    }

    /// The theme of application windows (settings window).
    pub fn app(inputs: &ThemeInputs) -> Theme {
        Theme::resolve(inputs, inputs.apps_mode)
    }

    fn resolve(inputs: &ThemeInputs, mode: ThemeMode) -> Theme {
        let text_scale = inputs.text_scale.clamp(1.0, 2.25);
        let animations = inputs.animations;
        if let Some(hc) = inputs.high_contrast {
            return Theme::high_contrast(&hc, inputs, text_scale, animations);
        }
        let accent = inputs.accent.unwrap_or(AccentPalette::DEFAULT);
        let transparency = inputs.transparency;
        match mode {
            ThemeMode::Dark => Theme {
                mode,
                high_contrast: false,
                text_primary: Color::argb(0xFFFF_FFFF),
                text_secondary: Color::argb(0xC5FF_FFFF),
                text_disabled: Color::argb(0x5DFF_FFFF),
                accent_fill: accent.light2,
                accent_fill_hover: accent.light2.with_alpha(0.9),
                text_on_accent: Color::argb(0xFF00_0000),
                subtle_hover: Color::argb(0x0FFF_FFFF),
                subtle_pressed: Color::argb(0x0AFF_FFFF),
                subtle_hover_text: Color::argb(0xFFFF_FFFF),
                divider: Color::argb(0x15FF_FFFF),
                surface_stroke: Color::argb(0x3300_0000),
                focus_outer: Color::argb(0xFFFF_FFFF),
                focus_inner: Color::argb(0xB300_0000),
                // System flyouts use the darker "base" acrylic (tint #202020); the stock
                // backdrop is the lighter "default" one, so a share of the base tint is
                // painted over it.
                material: if transparency {
                    Material::Acrylic {
                        tint: Color::rgb(0x202020).with_alpha(0.45),
                        fallback: Color::rgb(0x202020),
                    }
                } else {
                    Material::Solid(Color::rgb(0x202020))
                },
                tray: TrayColors::for_taskbar(inputs),
                controls: ControlColors {
                    fill: Color::argb(0x0FFF_FFFF),
                    fill_hover: Color::argb(0x15FF_FFFF),
                    fill_pressed: Color::argb(0x08FF_FFFF),
                    fill_disabled: Color::argb(0x0BFF_FFFF),
                    hover_text: Color::argb(0xFFFF_FFFF),
                    stroke: Color::argb(0x12FF_FFFF),
                    accent_pressed: accent.light2.with_alpha(0.8),
                    accent_disabled: Color::argb(0x28FF_FFFF),
                    container: Color::argb(0x0AFF_FFFF),
                    slider_track: Color::argb(0x8BFF_FFFF),
                    slider_thumb: Color::rgb(0x454545),
                    slider_thumb_stroke: Color::argb(0x23FF_FFFF),
                    footer: Color::argb(0x2600_0000),
                    solid: Color::rgb(0x2C2C2C),
                    solid_stroke: Color::argb(0x3300_0000),
                    scroll_indicator: Color::argb(0x8BFF_FFFF),
                },
                window: WindowColors {
                    mica: transparency,
                    base: Color::rgb(0x202020),
                    card: Color::argb(0x0DFF_FFFF),
                    card_stroke: Color::argb(0x1900_0000),
                    toggle_off_fill: Color::argb(0x1900_0000),
                    toggle_off_fill_hover: Color::argb(0x0FFF_FFFF),
                    toggle_off_stroke: Color::argb(0xC5FF_FFFF),
                    link: accent.light3,
                    link_hover: accent.light3.with_alpha(0.8),
                    popup_stroke: Color::argb(0x3300_0000),
                    critical: Color::rgb(0xFF99A4),
                    input_focused: Color::argb(0xB31E_1E1E),
                    input_line: Color::argb(0x8BFF_FFFF),
                    notice_background: [
                        Color::argb(0x08FF_FFFF),
                        Color::rgb(0x393D1B),
                        Color::rgb(0x433519),
                        Color::rgb(0x442726),
                    ],
                    notice_icon: [
                        accent.light2,
                        Color::rgb(0x6CCB5F),
                        Color::rgb(0xFCE100),
                        Color::rgb(0xFF99A4),
                    ],
                    notice_glyph: Color::BLACK,
                    smoke: Color::argb(0x4D00_0000),
                },
                text_scale,
                animations,
            },
            ThemeMode::Light => Theme {
                mode,
                high_contrast: false,
                text_primary: Color::argb(0xE400_0000),
                text_secondary: Color::argb(0x9E00_0000),
                text_disabled: Color::argb(0x5C00_0000),
                accent_fill: accent.dark1,
                accent_fill_hover: accent.dark1.with_alpha(0.9),
                text_on_accent: Color::argb(0xFFFF_FFFF),
                subtle_hover: Color::argb(0x0900_0000),
                subtle_pressed: Color::argb(0x0600_0000),
                subtle_hover_text: Color::argb(0xE400_0000),
                divider: Color::argb(0x0F00_0000),
                surface_stroke: Color::argb(0x0F00_0000),
                focus_outer: Color::argb(0xE400_0000),
                focus_inner: Color::argb(0xB3FF_FFFF),
                material: if transparency {
                    Material::Acrylic {
                        tint: Color::rgb(0xF3F3F3).with_alpha(0.35),
                        fallback: Color::rgb(0xEEEEEE),
                    }
                } else {
                    Material::Solid(Color::rgb(0xEEEEEE))
                },
                tray: TrayColors::for_taskbar(inputs),
                controls: ControlColors {
                    fill: Color::argb(0xB3FF_FFFF),
                    fill_hover: Color::argb(0x80F9_F9F9),
                    fill_pressed: Color::argb(0x4DF9_F9F9),
                    fill_disabled: Color::argb(0x4DF9_F9F9),
                    hover_text: Color::argb(0xE400_0000),
                    stroke: Color::argb(0x0F00_0000),
                    accent_pressed: accent.dark1.with_alpha(0.8),
                    accent_disabled: Color::argb(0x3700_0000),
                    container: Color::argb(0x0600_0000),
                    slider_track: Color::argb(0x7200_0000),
                    slider_thumb: Color::rgb(0xFFFFFF),
                    slider_thumb_stroke: Color::argb(0x2900_0000),
                    footer: Color::argb(0x0900_0000),
                    solid: Color::rgb(0xF9F9F9),
                    solid_stroke: Color::argb(0x0F00_0000),
                    scroll_indicator: Color::argb(0x7200_0000),
                },
                window: WindowColors {
                    mica: transparency,
                    base: Color::rgb(0xF3F3F3),
                    card: Color::argb(0xB3FF_FFFF),
                    card_stroke: Color::argb(0x0F00_0000),
                    toggle_off_fill: Color::argb(0x0600_0000),
                    toggle_off_fill_hover: Color::argb(0x0900_0000),
                    toggle_off_stroke: Color::argb(0x9E00_0000),
                    link: accent.dark2,
                    link_hover: accent.dark2.with_alpha(0.8),
                    popup_stroke: Color::argb(0x0F00_0000),
                    critical: Color::rgb(0xC42B1C),
                    input_focused: Color::WHITE,
                    input_line: Color::argb(0x7200_0000),
                    notice_background: [
                        Color::argb(0x80F6_F6F6),
                        Color::rgb(0xDFF6DD),
                        Color::rgb(0xFFF4CE),
                        Color::rgb(0xFDE7E9),
                    ],
                    notice_icon: [
                        accent.dark1,
                        Color::rgb(0x0F7B0F),
                        Color::rgb(0x9D5D00),
                        Color::rgb(0xC42B1C),
                    ],
                    notice_glyph: Color::WHITE,
                    smoke: Color::argb(0x4D00_0000),
                },
                text_scale,
                animations,
            },
        }
    }

    fn high_contrast(
        hc: &HighContrastColors,
        inputs: &ThemeInputs,
        text_scale: f32,
        animations: bool,
    ) -> Theme {
        // High contrast: only the user's system colours, opaque surfaces, solid outlines.
        // Light or dark follows the high-contrast background, whatever the Windows mode.
        let mode = if hc.window.luminance() < 0.5 { ThemeMode::Dark } else { ThemeMode::Light };
        Theme {
            mode,
            high_contrast: true,
            text_primary: hc.window_text,
            text_secondary: hc.window_text,
            text_disabled: hc.gray_text,
            accent_fill: hc.highlight,
            accent_fill_hover: hc.highlight,
            text_on_accent: hc.highlight_text,
            subtle_hover: hc.highlight,
            subtle_pressed: hc.highlight,
            subtle_hover_text: hc.highlight_text,
            divider: hc.window_text,
            surface_stroke: hc.window_text,
            focus_outer: hc.window_text,
            focus_inner: hc.window,
            material: Material::Solid(hc.window),
            tray: TrayColors::for_taskbar(inputs),
            controls: ControlColors {
                fill: hc.button_face,
                fill_hover: hc.highlight,
                fill_pressed: hc.highlight,
                fill_disabled: hc.button_face,
                hover_text: hc.highlight_text,
                stroke: hc.button_text,
                accent_pressed: hc.highlight,
                accent_disabled: hc.gray_text,
                container: hc.window,
                slider_track: hc.button_text,
                slider_thumb: hc.button_face,
                slider_thumb_stroke: hc.button_text,
                footer: hc.window,
                solid: hc.window,
                solid_stroke: hc.window_text,
                scroll_indicator: hc.window_text,
            },
            window: WindowColors {
                mica: false,
                base: hc.window,
                card: hc.window,
                card_stroke: hc.window_text,
                toggle_off_fill: hc.window,
                toggle_off_fill_hover: hc.highlight,
                toggle_off_stroke: hc.window_text,
                link: hc.hotlight,
                link_hover: hc.hotlight,
                popup_stroke: hc.window_text,
                critical: hc.window_text,
                input_focused: hc.window,
                input_line: hc.window_text,
                notice_background: [hc.window; 4],
                notice_icon: [hc.window_text; 4],
                notice_glyph: hc.window,
                smoke: Color::TRANSPARENT,
            },
            text_scale,
            animations,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hc() -> HighContrastColors {
        HighContrastColors {
            window: Color::BLACK,
            window_text: Color::WHITE,
            highlight: Color::rgb(0x1AEBFF),
            highlight_text: Color::BLACK,
            gray_text: Color::rgb(0x3FF23F),
            button_face: Color::BLACK,
            button_text: Color::WHITE,
            hotlight: Color::rgb(0x75E9FC),
        }
    }

    #[test]
    fn shell_surfaces_follow_the_system_mode_and_windows_the_apps_mode() {
        let inputs = ThemeInputs {
            system_mode: ThemeMode::Dark,
            apps_mode: ThemeMode::Light,
            ..ThemeInputs::default()
        };
        assert_eq!(Theme::shell(&inputs).mode, ThemeMode::Dark);
        assert_eq!(Theme::shell(&inputs).tray.glyph, Color::WHITE);
        assert_eq!(Theme::app(&inputs).mode, ThemeMode::Light);
        // The settings window shows the tray icon as it looks on the taskbar.
        assert_eq!(Theme::app(&inputs).tray, Theme::shell(&inputs).tray);
        assert_eq!(Theme::app(&inputs).tray.taskbar, Color::rgb(0x1C1C1C));
    }

    #[test]
    fn accent_uses_the_windows_shades() {
        let mut accent = AccentPalette::DEFAULT;
        accent.light2 = Color::rgb(0xFF0000);
        accent.dark1 = Color::rgb(0x00FF00);
        let mut inputs = ThemeInputs { accent: Some(accent), ..ThemeInputs::default() };
        inputs.system_mode = ThemeMode::Dark;
        assert_eq!(Theme::shell(&inputs).accent_fill, Color::rgb(0xFF0000));
        inputs.system_mode = ThemeMode::Light;
        assert_eq!(Theme::shell(&inputs).accent_fill, Color::rgb(0x00FF00));
        inputs.accent = None;
        assert_eq!(Theme::shell(&inputs).accent_fill, AccentPalette::DEFAULT.dark1);
        // The BetterVolume tray look (FR-084): lighter shades on a dark taskbar.
        let a = AccentPalette::DEFAULT;
        inputs.system_mode = ThemeMode::Dark;
        assert_eq!(Theme::shell(&inputs).tray.accent, [a.light3, a.light1]);
        assert_eq!(Theme::shell(&inputs).tray.accent_unreached, a.light2.with_alpha(0.32));
        inputs.system_mode = ThemeMode::Light;
        assert_eq!(Theme::shell(&inputs).tray.accent, [a.dark1, a.dark3]);
        assert_eq!(Theme::shell(&inputs).tray.accent_unreached, a.dark1.with_alpha(0.32));
    }

    #[test]
    fn transparency_off_gives_opaque_surfaces() {
        let inputs = ThemeInputs { transparency: false, ..ThemeInputs::default() };
        assert!(matches!(Theme::shell(&inputs).material, Material::Solid(_)));
        let inputs = ThemeInputs { transparency: true, ..ThemeInputs::default() };
        assert!(matches!(Theme::shell(&inputs).material, Material::Acrylic { .. }));
    }

    #[test]
    fn opaque_replaces_the_translucent_material() {
        let inputs = ThemeInputs { system_mode: ThemeMode::Dark, ..ThemeInputs::default() };
        let theme = Theme::shell(&inputs);
        assert_eq!(theme.opaque().material, Material::Solid(Color::rgb(0x202020)));
        let solid = Theme::shell(&ThemeInputs { transparency: false, ..inputs });
        assert_eq!(solid.opaque(), solid);
    }

    #[test]
    fn high_contrast_uses_only_system_colours() {
        let inputs = ThemeInputs { high_contrast: Some(hc()), ..ThemeInputs::default() };
        let theme = Theme::shell(&inputs);
        assert!(theme.high_contrast);
        assert_eq!(theme.material, Material::Solid(Color::BLACK));
        assert_eq!(theme.text_primary, Color::WHITE);
        assert_eq!(theme.subtle_hover, hc().highlight);
        assert_eq!(theme.tray.glyph, Color::WHITE);
        assert!(theme.tray.high_contrast);
        assert_eq!(theme.tray.accent, [Color::WHITE; 2]);
        assert_eq!(theme.mode, ThemeMode::Dark, "a black high-contrast theme counts as dark");
        // A white high-contrast theme counts as light even when Windows is in dark mode.
        let white = HighContrastColors { window: Color::WHITE, window_text: Color::BLACK, ..hc() };
        let inputs = ThemeInputs {
            system_mode: ThemeMode::Dark,
            high_contrast: Some(white),
            ..ThemeInputs::default()
        };
        assert_eq!(Theme::shell(&inputs).mode, ThemeMode::Light);
    }

    #[test]
    fn text_scale_is_kept_in_the_windows_range() {
        let inputs = ThemeInputs { text_scale: 5.0, ..ThemeInputs::default() };
        assert_eq!(Theme::shell(&inputs).text_scale, 2.25);
        let inputs = ThemeInputs { text_scale: 0.2, ..ThemeInputs::default() };
        assert_eq!(Theme::shell(&inputs).text_scale, 1.0);
    }

    #[test]
    fn text_is_readable_on_the_accent_in_both_modes() {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let inputs = ThemeInputs { system_mode: mode, ..ThemeInputs::default() };
            let t = Theme::shell(&inputs);
            let (a, b) = (t.accent_fill.luminance(), t.text_on_accent.luminance());
            let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
            assert!(contrast >= 4.5, "{mode:?}: contrast {contrast}");
        }
    }
}
