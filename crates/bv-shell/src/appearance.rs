//! Reads the Windows appearance preferences the design system resolves themes from
//! (FR-004): light/dark modes, accent palette, transparency, high contrast, animations,
//! text scaling. Cheap registry and system-parameter reads, done at start, whenever
//! Windows broadcasts a setting change, and before each popup opens.

use bv_ui_kit::theme::{AccentPalette, HighContrastColors, ThemeInputs, ThemeMode};
use bv_ui_kit::Color;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, ERROR_SUCCESS};
use windows::Win32::Graphics::Gdi::{
    GetSysColor, COLOR_BTNFACE, COLOR_BTNTEXT, COLOR_GRAYTEXT, COLOR_HIGHLIGHT,
    COLOR_HIGHLIGHTTEXT, COLOR_HOTLIGHT, COLOR_WINDOW, COLOR_WINDOWTEXT, SYS_COLOR_INDEX,
};
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD,
};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::WindowsAndMessaging::{
    SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SPI_GETHIGHCONTRAST,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};

const PERSONALIZE: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
const ACCENT: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Accent");
const ACCESSIBILITY: PCWSTR = w!("Software\\Microsoft\\Accessibility");

/// The current preferences.
pub fn read() -> ThemeInputs {
    let mode = |light: Option<u32>| match light {
        Some(0) => ThemeMode::Dark,
        _ => ThemeMode::Light,
    };
    ThemeInputs {
        system_mode: mode(dword(PERSONALIZE, w!("SystemUsesLightTheme"))),
        apps_mode: mode(dword(PERSONALIZE, w!("AppsUseLightTheme"))),
        accent: binary::<32>(ACCENT, w!("AccentPalette")).map(|b| parse_accent_palette(&b)),
        high_contrast: high_contrast(),
        transparency: dword(PERSONALIZE, w!("EnableTransparency")) != Some(0),
        animations: animations(),
        text_scale: text_scale(dword(ACCESSIBILITY, w!("TextScaleFactor"))),
    }
}

/// The accent palette Windows stores: eight RGBA entries, lightest to darkest shade
/// (the eighth is unused).
pub(crate) fn parse_accent_palette(bytes: &[u8; 32]) -> AccentPalette {
    let at = |i: usize| Color::from_bytes(bytes[i * 4], bytes[i * 4 + 1], bytes[i * 4 + 2]);
    AccentPalette {
        light3: at(0),
        light2: at(1),
        light1: at(2),
        base: at(3),
        dark1: at(4),
        dark2: at(5),
        dark3: at(6),
    }
}

/// Text scaling from its registry value in percent (absent = 100 %).
pub(crate) fn text_scale(percent: Option<u32>) -> f32 {
    percent.map_or(1.0, |p| (p as f32 / 100.0).clamp(1.0, 2.25))
}

fn dword(key: PCWSTR, value: PCWSTR) -> Option<u32> {
    let mut data = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: `data` and `size` are valid for the call and sized for a DWORD.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            value,
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut data).cast()),
            Some(&mut size),
        )
    };
    (status == ERROR_SUCCESS).then_some(data)
}

fn binary<const N: usize>(key: PCWSTR, value: PCWSTR) -> Option<[u8; N]> {
    let mut data = [0u8; N];
    let mut size = N as u32;
    // SAFETY: `data` holds `size` bytes; a larger value makes the call fail, not overflow.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            value,
            RRF_RT_REG_BINARY,
            None,
            Some(data.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    (status == ERROR_SUCCESS && size as usize == N).then_some(data)
}

fn high_contrast() -> Option<HighContrastColors> {
    let mut info =
        HIGHCONTRASTW { cbSize: size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
    // SAFETY: `info` is a valid structure with its size set, as the call requires.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            info.cbSize,
            Some((&raw mut info).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if ok.is_err() || (info.dwFlags & HCF_HIGHCONTRASTON).0 == 0 {
        return None;
    }
    let sys = |index: SYS_COLOR_INDEX| {
        // SAFETY: plain call with a valid colour index.
        let COLORREF(c) = COLORREF(unsafe { GetSysColor(index) });
        Color::from_bytes((c & 0xFF) as u8, ((c >> 8) & 0xFF) as u8, ((c >> 16) & 0xFF) as u8)
    };
    Some(HighContrastColors {
        window: sys(COLOR_WINDOW),
        window_text: sys(COLOR_WINDOWTEXT),
        highlight: sys(COLOR_HIGHLIGHT),
        highlight_text: sys(COLOR_HIGHLIGHTTEXT),
        gray_text: sys(COLOR_GRAYTEXT),
        button_face: sys(COLOR_BTNFACE),
        button_text: sys(COLOR_BTNTEXT),
        hotlight: sys(COLOR_HOTLIGHT),
    })
}

fn animations() -> bool {
    let mut enabled = windows::core::BOOL(1);
    // SAFETY: `enabled` is a valid BOOL for the call to fill.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&raw mut enabled).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_err() || enabled.as_bool()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accent_palette_is_read_lightest_first() {
        let mut bytes = [0u8; 32];
        for i in 0..8 {
            bytes[i * 4] = i as u8 * 10;
            bytes[i * 4 + 1] = 1;
            bytes[i * 4 + 2] = 2;
            bytes[i * 4 + 3] = 0xFF;
        }
        let palette = parse_accent_palette(&bytes);
        assert_eq!(palette.light3, Color::from_bytes(0, 1, 2));
        assert_eq!(palette.base, Color::from_bytes(30, 1, 2));
        assert_eq!(palette.dark3, Color::from_bytes(60, 1, 2));
    }

    #[test]
    fn text_scale_comes_in_percent() {
        assert_eq!(text_scale(None), 1.0);
        assert_eq!(text_scale(Some(150)), 1.5);
        assert_eq!(text_scale(Some(400)), 2.25);
        assert_eq!(text_scale(Some(10)), 1.0);
    }

    #[test]
    fn reading_the_real_preferences_never_fails() {
        let inputs = read();
        assert!((1.0..=2.25).contains(&inputs.text_scale));
    }
}
