//! The system clipboard: text, and images for paste and Copy screenshot. Windows is reached through the Win32 API; macOS and Linux
//! through the standard command-line tools (`pbcopy`, `wl-copy`, `xclip`, `xsel`), so no
//! windowing-toolkit clipboard crate is needed.

use std::path::PathBuf;

/// Put text on the system clipboard.
pub fn set(text: &str) -> Result<(), String> {
    imp::set(text)
}

/// Read text from the system clipboard. Line endings come back as `\n`.
pub fn get() -> Result<String, String> {
    imp::get().map(|t| t.replace("\r\n", "\n"))
}

/// Put an image on the system clipboard.
pub fn set_image(pm: &metamind_ui::Pixmap) -> Result<(), String> {
    let png = pm.encode_png().map_err(|e| e.to_string())?;
    imp::set_png(&png, pm)
}

/// What a paste into a note inserts.
pub enum Paste {
    Text(String),
    Files(Vec<PathBuf>),
    Png(Vec<u8>),
}

/// Read the clipboard for a paste into a note. Copied files win over the file names that file
/// managers also put on the clipboard as text; an image is used only when there is no text, so
/// copying from an app that offers both still pastes text.
pub fn get_paste() -> Result<Paste, String> {
    let files = imp::files()?;
    if !files.is_empty() {
        return Ok(Paste::Files(files));
    }
    let text = get()?;
    if text.is_empty() {
        if let Some(png) = imp::png()? {
            return Ok(Paste::Png(png));
        }
    }
    Ok(Paste::Text(text))
}

/// Local paths from a `text/uri-list`; entries that are not `file://` URIs are skipped.
#[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
fn uri_list(list: &str) -> Vec<PathBuf> {
    list.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.strip_prefix("file://"))
        // `file://host/path`: only the local host (empty or `localhost`) is reachable.
        .filter_map(|l| l.strip_prefix("localhost").unwrap_or(l).strip_prefix('/'))
        .map(|p| PathBuf::from(format!("/{}", vault_core::path::percent_decode(p))))
        .collect()
}

/// An image as a 32-bit bottom-up `CF_DIB`, for programs that do not read the "PNG" format.
#[cfg_attr(not(windows), allow(dead_code))]
fn to_dib(pm: &metamind_ui::Pixmap) -> Vec<u8> {
    let (w, h) = (pm.width(), pm.height());
    // A pixmap this large cannot exist (the allocation would have failed first); saturating keeps
    // the arithmetic well defined on a 32-bit target all the same.
    let image_bytes = w.saturating_mul(h).saturating_mul(4);
    let mut dib = Vec::with_capacity(40 + image_bytes as usize);
    for v in [40u32, w, h] {
        dib.extend(v.to_le_bytes());
    }
    dib.extend(1u16.to_le_bytes());
    dib.extend(32u16.to_le_bytes());
    dib.extend(0u32.to_le_bytes());
    dib.extend(image_bytes.to_le_bytes());
    dib.extend([0u8; 16]);
    for row in pm.pixels().chunks(w as usize).rev() {
        for p in row {
            let c = p.demultiply();
            dib.extend([c.blue(), c.green(), c.red(), c.alpha()]);
        }
    }
    dib
}

/// A Windows device-independent bitmap (`CF_DIB`: header, optional masks and palette, pixels)
/// as PNG. Screenshots are 24- or 32-bit, which is all that is handled.
#[cfg_attr(not(windows), allow(dead_code))]
fn dib_to_png(dib: &[u8]) -> Result<Vec<u8>, String> {
    let u32_at = |i: usize| dib.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let bad = || "imagen del portapapeles ilegible".to_string();
    let header = u32_at(0).ok_or_else(bad)? as usize;
    let width = u32_at(4).ok_or_else(bad)? as i32;
    let height = u32_at(8).ok_or_else(bad)? as i32;
    let bits = dib.get(14..16).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or_else(bad)?;
    let compression = u32_at(16).ok_or_else(bad)?;
    let colors = u32_at(32).ok_or_else(bad)? as usize;
    // BI_RGB = 0 or BI_BITFIELDS = 3. The masks sit right after a 40-byte header, or inside a
    // larger one at the same offset; only the usual BGR order is accepted.
    match compression {
        0 => {}
        3 if (u32_at(40), u32_at(44), u32_at(48)) == (Some(0x00ff_0000), Some(0x0000_ff00), Some(0xff)) => {}
        3 => return Err("la imagen del portapapeles tiene una disposición de píxeles inusual".into()),
        _ => return Err(format!("la compresión de imagen {compression} del portapapeles no es compatible")),
    }
    if bits != 24 && bits != 32 {
        return Err(format!("las imágenes del portapapeles de {bits} bits no son compatibles"));
    }
    let (w, h) = (width.unsigned_abs(), height.unsigned_abs());
    // The header comes from another program, so every size in it is checked before it is used:
    // a hostile width, height, palette size or header length must not overflow the slice
    // arithmetic (which would panic and take the app down with it).
    let start = header
        .checked_add(if compression == 3 && header == 40 { 12 } else { 0 })
        .and_then(|s| colors.checked_mul(4).and_then(|c| s.checked_add(c)))
        .ok_or_else(bad)?;
    let stride = (w as usize)
        .checked_mul(bits as usize)
        .map(|bits| bits.div_ceil(32))
        .and_then(|rows| rows.checked_mul(4))
        .ok_or_else(bad)?;
    let len = stride.checked_mul(h as usize).ok_or_else(bad)?;
    let end = start.checked_add(len).ok_or_else(bad)?;
    let pixels = dib.get(start..end).ok_or_else(bad)?;
    let mut pm = metamind_ui::Pixmap::new(w, h).ok_or_else(bad)?;
    let step = bits as usize / 8;
    // 32-bit bitmaps often leave alpha at zero; then the image is opaque.
    let has_alpha = bits == 32 && pixels.chunks(4).any(|p| p.get(3).is_some_and(|&a| a != 0));
    let out = pm.data_mut();
    for y in 0..h as usize {
        // Positive height means rows are stored bottom-up.
        let src_row = if height > 0 { h as usize - 1 - y } else { y };
        let row = &pixels[src_row * stride..];
        for x in 0..w as usize {
            let p = &row[x * step..x * step + step];
            let a = if has_alpha { p[3] } else { 255 };
            let pre = |c: u8| (c as u16 * a as u16 / 255) as u8;
            let o = (y * w as usize + x) * 4;
            out[o..o + 4].copy_from_slice(&[pre(p[2]), pre(p[1]), pre(p[0]), a]);
        }
    }
    pm.encode_png().map_err(|e| e.to_string())
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::ffi::c_void;

    type Handle = *mut c_void;
    const CF_UNICODETEXT: u32 = 13;
    const CF_DIB: u32 = 8;
    const CF_HDROP: u32 = 15;
    const GMEM_MOVEABLE: u32 = 2;

    #[link(name = "user32")]
    extern "system" {
        fn OpenClipboard(owner: Handle) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn GetClipboardData(format: u32) -> Handle;
        fn SetClipboardData(format: u32, mem: Handle) -> Handle;
        fn RegisterClipboardFormatW(name: *const u16) -> u32;
    }
    #[link(name = "shell32")]
    extern "system" {
        fn DragQueryFileW(drop: Handle, index: u32, buf: *mut u16, len: u32) -> u32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> Handle;
        fn GlobalLock(mem: Handle) -> *mut c_void;
        fn GlobalUnlock(mem: Handle) -> i32;
        fn GlobalFree(mem: Handle) -> Handle;
        fn GlobalSize(mem: Handle) -> usize;
    }

    /// Another program may hold the clipboard for a moment, so opening is retried briefly.
    fn open() -> Result<Open, String> {
        for _ in 0..10 {
            // SAFETY: no pointers are passed; a null owner is allowed.
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return Ok(Open(()));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Err("otro programa está usando el portapapeles".into())
    }

    /// The clipboard while this program holds it open.
    ///
    /// Dropping the guard closes it, so an early return (the usual `?` in the middle of a copy)
    /// cannot leave the clipboard locked against every other program until the app exits. There
    /// is no way to obtain the window between `OpenClipboard` and this guard, and a copy that
    /// fails to put its data only closes early.
    struct Open(());

    impl Drop for Open {
        fn drop(&mut self) {
            // SAFETY: the only way to get an `Open` is `open()`, which calls `OpenClipboard`, and
            // nothing else closes it; the guard is neither `Copy` nor `Clone`.
            unsafe { CloseClipboard() };
        }
    }

    pub fn set(text: &str) -> Result<(), String> {
        let wide: Vec<u8> = text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
        let open = open()?;
        // SAFETY: the clipboard is open and stays open until `open` is dropped at the end.
        unsafe { EmptyClipboard() };
        put(&open, CF_UNICODETEXT, &wide)
    }

    /// The image as "PNG" for programs that keep transparency, and as a bitmap for the rest.
    pub fn set_png(png: &[u8], pm: &metamind_ui::Pixmap) -> Result<(), String> {
        let name: Vec<u16> = "PNG".encode_utf16().chain([0]).collect();
        let dib = super::to_dib(pm);
        let open = open()?;
        // SAFETY: the clipboard is open; `name` is zero-terminated.
        unsafe { EmptyClipboard() };
        // SAFETY: same; a zero return means the format is not available and is skipped.
        let format = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
        if format != 0 {
            put(&open, format, png)?;
        }
        put(&open, CF_DIB, &dib)
    }

    /// Hand a copy of `data` to the clipboard as `format`.
    ///
    /// Takes the open-clipboard guard, so the precondition is enforced by the type rather than
    /// by a comment. The block is allocated with the size of `data`, only written while locked,
    /// and freed on every path where the system does not take ownership of it.
    fn put(_open: &Open, format: u32, data: &[u8]) -> Result<(), String> {
        // SAFETY: plain Win32 calls with no aliasing: the destination is freshly allocated and
        // only the source slice is read.
        unsafe {
            let mem = GlobalAlloc(GMEM_MOVEABLE, data.len());
            if mem.is_null() {
                return Err("sin memoria".to_string());
            }
            let dst = GlobalLock(mem) as *mut u8;
            if dst.is_null() {
                GlobalFree(mem);
                return Err("no se pudo bloquear la memoria del portapapeles".to_string());
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), dst, data.len());
            GlobalUnlock(mem);
            if SetClipboardData(format, mem).is_null() {
                // The system took nothing, so the block is still ours to free.
                GlobalFree(mem);
                return Err("no se pudieron establecer los datos del portapapeles".to_string());
            }
        }
        Ok(())
    }

    pub fn get() -> Result<String, String> {
        let _open = open()?;
        // SAFETY: the clipboard is open; the handle is only read while locked, and the scan is
        // bounded by the size of the block, so an owner that never writes the terminating zero
        // cannot make this read past the end of the allocation.
        unsafe {
            let mem = GetClipboardData(CF_UNICODETEXT);
            if mem.is_null() {
                return Ok(String::new());
            }
            let src = GlobalLock(mem) as *const u16;
            if src.is_null() {
                return Err("no se pudo bloquear la memoria del portapapeles".to_string());
            }
            let max = GlobalSize(mem) / 2;
            let mut len = 0;
            while len < max && *src.add(len) != 0 {
                len += 1;
            }
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(src, len));
            GlobalUnlock(mem);
            Ok(text)
        }
    }

    /// Files copied in Explorer (`CF_HDROP`).
    pub fn files() -> Result<Vec<std::path::PathBuf>, String> {
        let _open = open()?;
        // SAFETY: the clipboard is open, so the drop handle stays valid until it is closed. Each
        // name is written into a buffer one longer than the length reported for it.
        let files = unsafe {
            let drop = GetClipboardData(CF_HDROP);
            if drop.is_null() {
                Vec::new()
            } else {
                let count = DragQueryFileW(drop, u32::MAX, std::ptr::null_mut(), 0);
                (0..count)
                    .filter_map(|i| {
                        let len = DragQueryFileW(drop, i, std::ptr::null_mut(), 0);
                        if len == 0 || len > 32_768 {
                            return None;
                        }
                        let mut buf = vec![0u16; len as usize + 1];
                        let got = DragQueryFileW(drop, i, buf.as_mut_ptr(), len + 1) as usize;
                        (got > 0).then(|| String::from_utf16_lossy(&buf[..got]).into())
                    })
                    .collect()
            }
        };
        Ok(files)
    }

    /// An image: the "PNG" format browsers and the Snipping Tool offer, else the bitmap.
    pub fn png() -> Result<Option<Vec<u8>>, String> {
        let name: Vec<u16> = "PNG".encode_utf16().chain([0]).collect();
        let open = open()?;
        // SAFETY: the clipboard is open; `name` is zero-terminated.
        let (png, dib) = unsafe {
            let format = RegisterClipboardFormatW(name.as_ptr());
            match (format != 0).then(|| bytes(&open, format)).flatten() {
                Some(png) => (Some(png), None),
                None => (None, bytes(&open, CF_DIB)),
            }
        };
        match (png, dib) {
            (Some(png), _) => Ok(Some(png)),
            (None, Some(dib)) => super::dib_to_png(&dib).map(Some),
            (None, None) => Ok(None),
        }
    }

    /// A copy of the data in `format`, while the open-clipboard guard proves it can be read.
    fn bytes(_open: &Open, format: u32) -> Option<Vec<u8>> {
        // SAFETY: the clipboard is open and the block is read while locked, for exactly the
        // number of bytes the system reports for it.
        unsafe {
            let mem = GetClipboardData(format);
            if mem.is_null() {
                return None;
            }
            let src = GlobalLock(mem) as *const u8;
            if src.is_null() {
                return None;
            }
            let size = GlobalSize(mem);
            let data = if size == 0 { Vec::new() } else { std::slice::from_raw_parts(src, size).to_vec() };
            GlobalUnlock(mem);
            Some(data)
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io::Write;
    use std::process::{Command, Stdio};

    /// Candidate tools as (copy command, paste command), in order of preference.
    fn tools() -> Vec<(&'static [&'static str], &'static [&'static str])> {
        if cfg!(target_os = "macos") {
            return vec![(&["pbcopy"], &["pbpaste"])];
        }
        let mut t: Vec<(&'static [&'static str], &'static [&'static str])> = Vec::new();
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            t.push((&["wl-copy"], &["wl-paste", "--no-newline"]));
        }
        t.push((&["xclip", "-selection", "clipboard"], &["xclip", "-selection", "clipboard", "-o"]));
        t.push((&["xsel", "--clipboard", "--input"], &["xsel", "--clipboard", "--output"]));
        t
    }

    const MISSING: &str = "no clipboard tool found (install wl-clipboard, xclip or xsel)";

    pub fn set(text: &str) -> Result<(), String> {
        for (copy, _) in tools() {
            let Ok(mut child) = Command::new(copy[0])
                .args(&copy[1..])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            else {
                continue;
            };
            let wrote = child.stdin.take().map(|mut s| s.write_all(text.as_bytes()));
            let status = child.wait().map_err(|e| e.to_string())?;
            return match wrote {
                Some(Ok(())) if status.success() => Ok(()),
                Some(Err(e)) => Err(format!("{}: {e}", copy[0])),
                _ => Err(format!("{} failed", copy[0])),
            };
        }
        Err(MISSING.into())
    }

    #[cfg(not(target_os = "macos"))]
    pub fn set_png(png: &[u8], _: &metamind_ui::Pixmap) -> Result<(), String> {
        let mut t: Vec<&[&str]> = Vec::new();
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            t.push(&["wl-copy", "--type", "image/png"]);
        }
        t.push(&["xclip", "-selection", "clipboard", "-target", "image/png"]);
        for copy in t {
            let Ok(mut child) = Command::new(copy[0])
                .args(&copy[1..])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            else {
                continue;
            };
            let wrote = child.stdin.take().map(|mut s| s.write_all(png));
            let status = child.wait().map_err(|e| e.to_string())?;
            return match wrote {
                Some(Ok(())) if status.success() => Ok(()),
                Some(Err(e)) => Err(format!("{}: {e}", copy[0])),
                _ => Err(format!("{} failed", copy[0])),
            };
        }
        Err("no image clipboard tool found (install wl-clipboard or xclip)".into())
    }

    /// AppleScript reads the image from a file; a long hex literal could pass the argument limit.
    #[cfg(target_os = "macos")]
    pub fn set_png(png: &[u8], _: &metamind_ui::Pixmap) -> Result<(), String> {
        let file = std::env::temp_dir().join("metamind-screenshot.png");
        std::fs::write(&file, png).map_err(|e| e.to_string())?;
        let script = format!(
            "set the clipboard to (read (POSIX file \"{}\") as «class PNGf»)",
            file.display().to_string().replace('\\', "\\\\").replace('"', "\\\"")
        );
        let ok = Command::new("osascript").args(["-e", &script]).stderr(Stdio::null()).status();
        let _ = std::fs::remove_file(&file);
        match ok {
            Ok(s) if s.success() => Ok(()),
            Ok(_) => Err("osascript failed".into()),
            Err(e) => Err(format!("osascript: {e}")),
        }
    }

    pub fn get() -> Result<String, String> {
        for (_, paste) in tools() {
            let Ok(out) = Command::new(paste[0]).args(&paste[1..]).stderr(Stdio::null()).output() else {
                continue;
            };
            return if out.status.success() {
                Ok(String::from_utf8_lossy(&out.stdout).into_owned())
            } else {
                // wl-paste and xclip exit non-zero when the clipboard holds no text.
                Ok(String::new())
            };
        }
        Err(MISSING.into())
    }

    /// Standard output of a command that ran and succeeded.
    fn run(cmd: &[&str]) -> Option<Vec<u8>> {
        let out = Command::new(cmd[0]).args(&cmd[1..]).stderr(Stdio::null()).output().ok()?;
        out.status.success().then_some(out.stdout)
    }

    /// The clipboard's data of type `mime`, if it offers that type.
    #[cfg(not(target_os = "macos"))]
    fn fetch(mime: &str) -> Option<Vec<u8>> {
        let offered = |list: Vec<u8>| String::from_utf8_lossy(&list).lines().any(|l| l.trim() == mime);
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            if let Some(list) = run(&["wl-paste", "--list-types"]) {
                return offered(list).then(|| run(&["wl-paste", "--type", mime])).flatten();
            }
        }
        let list = run(&["xclip", "-selection", "clipboard", "-t", "TARGETS", "-o"])?;
        offered(list).then(|| run(&["xclip", "-selection", "clipboard", "-t", mime, "-o"])).flatten()
    }

    /// Files copied in a file manager, which offers them as `text/uri-list`.
    #[cfg(not(target_os = "macos"))]
    pub fn files() -> Result<Vec<std::path::PathBuf>, String> {
        Ok(fetch("text/uri-list").map(|b| super::uri_list(&String::from_utf8_lossy(&b))).unwrap_or_default())
    }

    #[cfg(not(target_os = "macos"))]
    pub fn png() -> Result<Option<Vec<u8>>, String> {
        Ok(fetch("image/png"))
    }

    /// A file copied in Finder. AppleScript only gives the first of several.
    #[cfg(target_os = "macos")]
    pub fn files() -> Result<Vec<std::path::PathBuf>, String> {
        let out = run(&["osascript", "-e", "POSIX path of (the clipboard as «class furl»)"]);
        let path = out.map(|b| String::from_utf8_lossy(&b).trim_end_matches('\n').to_string());
        Ok(path.filter(|p| !p.is_empty()).map(|p| vec![p.into()]).unwrap_or_default())
    }

    /// An image, which AppleScript prints as `«data PNGf89504E…»`.
    #[cfg(target_os = "macos")]
    pub fn png() -> Result<Option<Vec<u8>>, String> {
        let Some(out) = run(&["osascript", "-e", "the clipboard as «class PNGf»"]) else { return Ok(None) };
        let out = String::from_utf8_lossy(&out);
        let Some((_, rest)) = out.split_once("«data PNGf") else { return Ok(None) };
        let hex = rest.split('»').next().unwrap_or("").as_bytes();
        let digit = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
        let bytes: Option<Vec<u8>> =
            hex.chunks(2).map(|p| Some(digit(p[0])? << 4 | digit(*p.get(1)?)?)).collect();
        bytes.map(Some).ok_or_else(|| "imagen del portapapeles ilegible".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metamind_ui::Pixmap;

    #[test]
    fn uri_lists_become_local_paths() {
        let list =
            "# copied\r\nfile:///home/me/My%20Pic.png\r\nfile://localhost/tmp/a.txt\r\nhttps://x.org/b\r\n";
        assert_eq!(uri_list(list), [PathBuf::from("/home/me/My Pic.png"), PathBuf::from("/tmp/a.txt")]);
    }

    /// The list is whatever another program put on the clipboard: a `..` in it is kept as part of
    /// a local path (the paste handler copies the file in), never resolved into a different host
    /// and never turned into something that is not a path.
    #[test]
    fn a_file_uri_with_dots_or_a_remote_host_is_not_mistaken_for_a_local_file() {
        let list = "file://remote.example/etc/passwd\nfile://../../etc/passwd\nfile://\nfile:///a/../b.png\n";
        assert_eq!(uri_list(list), [PathBuf::from("/a/../b.png")]);
    }

    /// The bitmap header comes from another program. None of these may panic: an oversized
    /// width, height, palette or header length is an error, like a truncated bitmap.
    #[test]
    fn hostile_bitmaps_do_not_panic() {
        let mut huge = header(i32::MIN, i32::MIN, 32, 3);
        huge.extend([0u8; 12]);
        huge.extend([0u8; 8]);
        assert!(dib_to_png(&huge).is_err());

        let mut palette = header(2, 2, 8, 0);
        palette[32..36].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
        palette.extend([0u8; 64]);
        assert!(dib_to_png(&palette).is_err());

        let mut far = header(2, 2, 24, 0);
        far[0..4].copy_from_slice(&0xffff_fff0u32.to_le_bytes());
        far.extend([0u8; 64]);
        assert!(dib_to_png(&far).is_err());

        let mut pm = Pixmap::new(4, 4).unwrap();
        pm.fill(metamind_ui::SkColor::from_rgba8(1, 2, 3, 255));
        let dib = to_dib(&pm);
        for n in 0..dib.len() {
            assert!(dib_to_png(&dib[..n]).is_err(), "truncated to {n} of {}", dib.len());
        }
        assert!(dib_to_png(&dib).is_ok());
    }

    fn header(w: i32, h: i32, bits: u16, compression: u32) -> Vec<u8> {
        let mut b = Vec::new();
        for v in [40u32, w as u32, h as u32] {
            b.extend(v.to_le_bytes());
        }
        b.extend(1u16.to_le_bytes());
        b.extend(bits.to_le_bytes());
        b.extend(compression.to_le_bytes());
        b.extend([0u8; 20]);
        b
    }

    fn rgba(png: &[u8], x: u32, y: u32) -> (u8, u8, u8, u8) {
        let p = Pixmap::decode_png(png).unwrap().pixel(x, y).unwrap().demultiply();
        (p.red(), p.green(), p.blue(), p.alpha())
    }

    #[test]
    fn images_round_trip_through_bitmaps() {
        let mut pm = Pixmap::new(3, 2).unwrap();
        pm.fill(metamind_ui::SkColor::from_rgba8(10, 20, 30, 255));
        pm.data_mut()[..4].copy_from_slice(&[200, 100, 50, 255]);
        let png = dib_to_png(&to_dib(&pm)).unwrap();
        assert_eq!(rgba(&png, 0, 0), (200, 100, 50, 255));
        assert_eq!(rgba(&png, 2, 1), (10, 20, 30, 255));
    }

    #[test]
    fn bottom_up_24_bit_bitmaps_convert() {
        // 2x2, rows padded to 8 bytes, stored bottom row first, pixels as BGR.
        let mut dib = header(2, 2, 24, 0);
        dib.extend([0, 0, 255, 0, 255, 0, 0, 0]); // bottom: red, green
        dib.extend([255, 0, 0, 255, 255, 255, 0, 0]); // top: blue, white
        let png = dib_to_png(&dib).unwrap();
        assert_eq!(rgba(&png, 0, 0), (0, 0, 255, 255));
        assert_eq!(rgba(&png, 1, 0), (255, 255, 255, 255));
        assert_eq!(rgba(&png, 0, 1), (255, 0, 0, 255));
        assert_eq!(rgba(&png, 1, 1), (0, 255, 0, 255));
    }

    #[test]
    fn top_down_32_bit_bitmaps_keep_alpha_and_masks_are_checked() {
        let mut dib = header(1, -2, 32, 3);
        dib.extend([0x00, 0x00, 0xff, 0x00, 0x00, 0xff, 0x00, 0x00, 0xff, 0x00, 0x00, 0x00]);
        dib.extend([0, 0, 255, 255, 255, 0, 0, 0]); // red opaque, then transparent blue
        let png = dib_to_png(&dib).unwrap();
        assert_eq!(rgba(&png, 0, 0), (255, 0, 0, 255));
        assert_eq!(rgba(&png, 0, 1).3, 0);

        // Alpha left at zero everywhere means an opaque image.
        let mut dib = header(1, 1, 32, 0);
        dib.extend([0, 255, 0, 0]);
        assert_eq!(rgba(&dib_to_png(&dib).unwrap(), 0, 0), (0, 255, 0, 255));

        let mut odd = header(1, 1, 32, 3);
        odd.extend([0u8; 12]);
        odd.extend([0u8; 4]);
        assert!(dib_to_png(&odd).unwrap_err().contains("inusual"));
        assert!(dib_to_png(&header(1, 1, 8, 0)).is_err());
        assert!(dib_to_png(&dib[..10]).is_err());
    }
}
