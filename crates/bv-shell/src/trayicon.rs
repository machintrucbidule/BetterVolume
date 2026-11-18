//! Tray icon images: from design-system pixels to a Windows icon handle.

use bv_ui_kit::backend::Pixels;
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, DestroyIcon, HICON, ICONINFO};

/// An icon handle we own; destroyed when dropped.
#[derive(Debug)]
pub(crate) struct OwnedIcon(pub HICON);

impl Drop for OwnedIcon {
    fn drop(&mut self) {
        // SAFETY: the icon was created by `CreateIconIndirect` and is destroyed once.
        let _ = unsafe { DestroyIcon(self.0) };
    }
}

struct OwnedBitmap(HBITMAP);

impl Drop for OwnedBitmap {
    fn drop(&mut self) {
        // SAFETY: the bitmap was created here and is deleted once.
        let _ = unsafe { DeleteObject(self.0.into()) };
    }
}

/// Icons hold straight (not premultiplied) alpha, unlike the design-system pixels.
pub(crate) fn unpremultiply(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for px in data.as_chunks::<4>().0 {
        let a = u32::from(px[3]);
        let un =
            |c: u8| (u32::from(c) * 255 + a / 2).checked_div(a).map_or(0, |v| v.min(255) as u8);
        out.extend_from_slice(&[un(px[0]), un(px[1]), un(px[2]), px[3]]);
    }
    out
}

/// Creates a 32-bit icon with alpha from premultiplied BGRA pixels.
pub(crate) fn create(pixels: &Pixels) -> Option<OwnedIcon> {
    // The copy below relies on this size: refuse anything else.
    let expected = (pixels.width as usize).checked_mul(pixels.height as usize)?.checked_mul(4)?;
    if pixels.data.len() != expected || expected == 0 {
        return None;
    }
    let (w, h) = (i32::try_from(pixels.width).ok()?, i32::try_from(pixels.height).ok()?);
    let header = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: w,
        biHeight: -h, // top-down rows, like the pixels
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };
    let info = BITMAPINFO { bmiHeader: header, ..Default::default() };
    let mut bits = std::ptr::null_mut();
    // SAFETY: `info` describes a 32-bit top-down bitmap; `bits` receives its memory.
    let section = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) };
    let color = OwnedBitmap(section.ok()?);
    if bits.is_null() {
        return None;
    }
    let straight = unpremultiply(&pixels.data);
    // SAFETY: the section holds exactly width * height * 4 bytes, the size of `straight`.
    unsafe { std::ptr::copy_nonoverlapping(straight.as_ptr(), bits.cast::<u8>(), straight.len()) };
    // The mask is unused for 32-bit icons with alpha but must exist (all zero = opaque
    // where the alpha says so).
    let mask_bits = vec![0u8; ((w + 15) / 16 * 2 * h) as usize];
    // SAFETY: one-bit bitmap whose rows are word-aligned, as `mask_bits` is sized.
    let mask = OwnedBitmap(unsafe { CreateBitmap(w, h, 1, 1, Some(mask_bits.as_ptr().cast())) });
    if mask.0.is_invalid() {
        return None;
    }
    let icon =
        ICONINFO { fIcon: true.into(), hbmMask: mask.0, hbmColor: color.0, ..Default::default() };
    // SAFETY: both bitmaps are valid; the icon copies them (dropped right after).
    unsafe { CreateIconIndirect(&icon) }.ok().map(OwnedIcon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpremultiply_restores_the_colour() {
        // White at 50 % opacity, premultiplied: 128,128,128,128.
        assert_eq!(unpremultiply(&[128, 128, 128, 128]), vec![255, 255, 255, 128]);
        assert_eq!(unpremultiply(&[0, 0, 0, 0]), vec![0, 0, 0, 0]);
        assert_eq!(unpremultiply(&[10, 20, 30, 255]), vec![10, 20, 30, 255]);
    }

    #[test]
    fn icons_can_be_created() {
        let pixels = Pixels { width: 16, height: 16, data: vec![200; 16 * 16 * 4] };
        assert!(create(&pixels).is_some());
        let pixels = Pixels { width: 20, height: 20, data: vec![0; 20 * 20 * 4] };
        assert!(create(&pixels).is_some());
        // Pixels whose size does not match their dimensions are refused.
        let wrong = Pixels { width: 16, height: 16, data: vec![0; 10] };
        assert!(create(&wrong).is_none());
    }
}
