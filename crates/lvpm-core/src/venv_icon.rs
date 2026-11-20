//! The venv's icon: LabVIEW's own, with a badge that names the venv.
//!
//! Built at runtime from the bound `LabVIEW.exe`, so lvpm ships none of NI's
//! artwork and each LabVIEW version's icon stays recognisable. The badge is a
//! disc in the upper right (LabVIEW 2026 puts its version number in the lower
//! right), half the icon's width, ringed in white so it reads
//! on LabVIEW's artwork; its colour is picked from the project's name, so two
//! venvs differ and one venv keeps its colour. No letter: at taskbar sizes it
//! was unreadable, and the colour alone tells venvs apart.

// The icon is drawn only on Windows; elsewhere its helpers are compiled but unused.
#![cfg_attr(not(windows), allow(dead_code))]

use crate::venv::Venv;
use anyhow::Result;
use std::path::PathBuf;

/// Sizes written to the `.ico`: what Explorer, the taskbar and Alt+Tab ask for
/// at 100–200 % scaling.
const SIZES: [u32; 5] = [16, 24, 32, 48, 256];

/// Badge colours, as RGB: saturated, and none of them near LabVIEW's yellow.
const PALETTE: [[u8; 3]; 7] = [
    [0x25, 0x63, 0xEB], // blue
    [0x16, 0xA3, 0x4A], // green
    [0x7C, 0x3A, 0xED], // violet
    [0xDC, 0x26, 0x26], // red
    [0x0D, 0x94, 0x88], // teal
    [0xDB, 0x27, 0x77], // pink
    [0x4F, 0x46, 0xE5], // indigo
];

/// FNV-1a: stable across Rust releases, unlike `DefaultHasher`, which matters
/// wherever the value outlives one lvpm (a colour, a pinned app id).
pub fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The project's name as the badge shows it: the repo folder's.
fn project_name(v: &Venv) -> String {
    v.repo
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "venv".into())
}

fn badge_color(name: &str) -> [u8; 3] {
    PALETTE[(fnv1a(&name.to_lowercase()) % PALETTE.len() as u64) as usize]
}

/// Where the venv's icon lives.
pub fn path(v: &Venv) -> PathBuf {
    v.dir.join("venv.ico")
}

/// Write the venv's icon (always: LabVIEW may have been updated in place)
/// and return its path.
#[cfg(windows)]
pub fn write(v: &Venv) -> Result<PathBuf> {
    use anyhow::Context;
    let name = project_name(v);
    let color = badge_color(&name);
    let exe = v.target.exe();
    let mut images = Vec::new();
    for size in SIZES {
        let mut px = win::extract(&exe, size)
            .with_context(|| format!("reading the icon of {}", exe.display()))?;
        draw_badge(&mut px, size, color);
        images.push((size, px));
    }
    let out = path(v);
    std::fs::write(&out, encode_ico(&images))
        .with_context(|| format!("writing {}", out.display()))?;
    Ok(out)
}

#[cfg(not(windows))]
pub fn write(_v: &Venv) -> Result<PathBuf> {
    anyhow::bail!("venv icons are Windows-only for now")
}

/// The venv's icon, or LabVIEW's own with why the venv's could not be made.
pub fn or_labviews(v: &Venv) -> (PathBuf, Option<String>) {
    match write(v) {
        Ok(ico) => (ico, None),
        Err(e) => (v.target.exe(), Some(format!("{e:#}"))),
    }
}

fn badge_diameter(size: u32) -> u32 {
    (size / 2).max(9)
}

/// One image: `size`×`size` pixels, top row first, each `[b, g, r, a]` with
/// straight (not premultiplied) alpha — the layout an icon stores.
type Pixels = Vec<[u8; 4]>;

/// Paint `src` (RGB, coverage `a` 0..1) over the pixel `dst`.
fn blend(dst: &mut [u8; 4], src: [u8; 3], a: f32) {
    if a <= 0.0 {
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let out_a = a + da * (1.0 - a);
    let mix = |s: u8, d: u8| {
        ((s as f32 * a + d as f32 * da * (1.0 - a)) / out_a)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    *dst = [
        mix(src[2], dst[0]),
        mix(src[1], dst[1]),
        mix(src[0], dst[2]),
        (out_a * 255.0).round() as u8,
    ];
}

/// The badge, anti-aliased by distance from the disc's edge.
fn draw_badge(px: &mut Pixels, size: u32, color: [u8; 3]) {
    let d = badge_diameter(size) as f32;
    let r = d / 2.0;
    let ring = (size as f32 / 20.0).max(1.0);
    let (cx, cy) = (size as f32 - r, r);
    let x0 = (size as f32 - d) as u32;
    let y_end = d.ceil() as u32;
    for y in 0..y_end.min(size) {
        for x in x0..size {
            let dist = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let p = &mut px[(y * size + x) as usize];
            blend(p, [255, 255, 255], (r - dist + 0.5).clamp(0.0, 1.0));
            blend(p, color, (r - ring - dist + 0.5).clamp(0.0, 1.0));
        }
    }
}

/// An `.ico` of 32-bit BMP images, the format every Windows since Vista reads
/// at every size, 256 included.
fn encode_ico(images: &[(u32, Pixels)]) -> Vec<u8> {
    let mut dir = Vec::new();
    let mut data = Vec::new();
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, px) in images {
        let s = *size;
        let mask_row = s.div_ceil(32) * 4;
        let mut img = Vec::new();
        // BITMAPINFOHEADER; an icon's height counts the XOR and AND halves.
        for v in [40u32, s, s * 2] {
            img.extend(v.to_le_bytes());
        }
        img.extend(1u16.to_le_bytes());
        img.extend(32u16.to_le_bytes());
        for v in [0u32, s * s * 4 + mask_row * s, 0, 0, 0, 0] {
            img.extend(v.to_le_bytes());
        }
        for y in (0..s).rev() {
            for x in 0..s {
                img.extend(px[(y * s + x) as usize]);
            }
        }
        for y in (0..s).rev() {
            let mut row = vec![0u8; mask_row as usize];
            for x in 0..s {
                if px[(y * s + x) as usize][3] == 0 {
                    row[(x / 8) as usize] |= 0x80 >> (x % 8);
                }
            }
            img.extend(row);
        }
        let dim = if s >= 256 { 0 } else { s as u8 };
        dir.extend([dim, dim, 0, 0]);
        dir.extend(1u16.to_le_bytes());
        dir.extend(32u16.to_le_bytes());
        dir.extend((img.len() as u32).to_le_bytes());
        dir.extend(offset.to_le_bytes());
        offset += img.len() as u32;
        data.extend(img);
    }
    let mut out = vec![0, 0, 1, 0];
    out.extend((images.len() as u16).to_le_bytes());
    out.extend(dir);
    out.extend(data);
    out
}

#[cfg(windows)]
mod win {
    use super::Pixels;
    use anyhow::{Result, bail};
    use std::ffi::c_void;
    use std::path::Path;

    type Handle = *mut c_void;

    #[repr(C)]
    struct IconInfo {
        f_icon: i32,
        x_hotspot: u32,
        y_hotspot: u32,
        hbm_mask: Handle,
        hbm_color: Handle,
    }

    #[repr(C)]
    struct BitmapInfoHeader {
        size: u32,
        width: i32,
        height: i32,
        planes: u16,
        bit_count: u16,
        compression: u32,
        size_image: u32,
        x_ppm: i32,
        y_ppm: i32,
        clr_used: u32,
        clr_important: u32,
    }

    #[repr(C)]
    struct BitmapInfo {
        header: BitmapInfoHeader,
        colors: [u32; 3],
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn PrivateExtractIconsW(
            file: *const u16,
            index: i32,
            cx: i32,
            cy: i32,
            icons: *mut Handle,
            ids: *mut u32,
            n: u32,
            flags: u32,
        ) -> u32;
        fn GetIconInfo(icon: Handle, info: *mut IconInfo) -> i32;
        fn DestroyIcon(icon: Handle) -> i32;
    }

    #[link(name = "gdi32")]
    unsafe extern "system" {
        fn CreateCompatibleDC(dc: Handle) -> Handle;
        fn DeleteDC(dc: Handle) -> i32;
        fn GetDIBits(
            dc: Handle,
            bm: Handle,
            start: u32,
            lines: u32,
            bits: *mut c_void,
            info: *mut BitmapInfo,
            usage: u32,
        ) -> i32;
        fn DeleteObject(obj: Handle) -> i32;
    }

    const DIB_RGB_COLORS: u32 = 0;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn header(size: u32) -> BitmapInfo {
        BitmapInfo {
            header: BitmapInfoHeader {
                size: std::mem::size_of::<BitmapInfoHeader>() as u32,
                width: size as i32,
                // Negative: top row first.
                height: -(size as i32),
                planes: 1,
                bit_count: 32,
                compression: 0,
                size_image: 0,
                x_ppm: 0,
                y_ppm: 0,
                clr_used: 0,
                clr_important: 0,
            },
            colors: [0; 3],
        }
    }

    /// The exe's first icon at `size`, as straight-alpha BGRA.
    pub fn extract(exe: &Path, size: u32) -> Result<Pixels> {
        let file = wide(&exe.to_string_lossy());
        let mut icon: Handle = std::ptr::null_mut();
        let mut id = 0u32;
        // SAFETY: one icon handle and one id slot, matching n = 1.
        let n = unsafe {
            PrivateExtractIconsW(
                file.as_ptr(),
                0,
                size as i32,
                size as i32,
                &mut icon,
                &mut id,
                1,
                0,
            )
        };
        if n != 1 || icon.is_null() {
            bail!("no icon at {size} px");
        }
        let mut info = IconInfo {
            f_icon: 0,
            x_hotspot: 0,
            y_hotspot: 0,
            hbm_mask: std::ptr::null_mut(),
            hbm_color: std::ptr::null_mut(),
        };
        // SAFETY: `info` is a valid ICONINFO to fill; both bitmaps it returns
        // are ours to delete, and the icon is destroyed after.
        let ok = unsafe { GetIconInfo(icon, &mut info) } != 0;
        let result = if ok && !info.hbm_color.is_null() {
            let color = read_bits(info.hbm_color, size);
            let mask = read_bits(info.hbm_mask, size);
            match (color, mask) {
                (Some(mut px), Some(mask)) => {
                    // An icon without an alpha channel keeps its transparency
                    // in the mask: white there means transparent.
                    if px.iter().all(|p| p[3] == 0) {
                        for (p, m) in px.iter_mut().zip(&mask) {
                            p[3] = if m[0] == 0 { 255 } else { 0 };
                        }
                    }
                    Ok(px)
                }
                _ => Err(anyhow::anyhow!(
                    "could not read the icon's pixels at {size} px"
                )),
            }
        } else {
            Err(anyhow::anyhow!("icon at {size} px has no colour bitmap"))
        };
        unsafe {
            if !info.hbm_color.is_null() {
                DeleteObject(info.hbm_color);
            }
            if !info.hbm_mask.is_null() {
                DeleteObject(info.hbm_mask);
            }
            DestroyIcon(icon);
        }
        result
    }

    fn read_bits(bm: Handle, size: u32) -> Option<Pixels> {
        let mut px: Pixels = vec![[0; 4]; (size * size) as usize];
        let mut bi = header(size);
        // SAFETY: a memory DC for GetDIBits; the buffer holds size² 32-bit
        // pixels, which is what the header asks for.
        unsafe {
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            let lines = GetDIBits(
                dc,
                bm,
                0,
                size,
                px.as_mut_ptr().cast(),
                &mut bi,
                DIB_RGB_COLORS,
            );
            DeleteDC(dc);
            (lines == size as i32).then_some(px)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_is_the_published_function() {
        assert_eq!(fnv1a(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a("a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn ico_header_counts_and_sizes() {
        let px: Pixels = vec![[0, 0, 0, 255]; 16 * 16];
        let ico = encode_ico(&[(16, px)]);
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 1, 0]);
        assert_eq!(ico[6], 16);
        let len = u32::from_le_bytes(ico[14..18].try_into().unwrap()) as usize;
        assert_eq!(ico.len(), 6 + 16 + len);
        assert_eq!(len, 40 + 16 * 16 * 4 + 4 * 16);
    }
}
