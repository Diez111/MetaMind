//! Operating system accessibility preferences: reduce motion and increase contrast.
//! An unreadable setting counts as off, the OS default everywhere.

/// True when the OS asks applications to keep animation to a minimum.
pub fn reduced_motion() -> bool {
    imp::reduced_motion()
}

/// True when the OS asks for higher contrast (Windows high-contrast themes, macOS Increase
/// contrast, GNOME High Contrast).
pub fn high_contrast() -> bool {
    imp::high_contrast()
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::ffi::c_void;

    // Settings > Accessibility > Visual effects > Animation effects.
    const SPI_GETCLIENTAREAANIMATION: u32 = 0x1042;
    const SPI_GETHIGHCONTRAST: u32 = 0x0042;
    const HCF_HIGHCONTRASTON: u32 = 1;

    #[repr(C)]
    struct HighContrast {
        size: u32,
        flags: u32,
        scheme: *mut u16,
    }

    #[link(name = "user32")]
    extern "system" {
        fn SystemParametersInfoW(action: u32, param: u32, value: *mut c_void, ini: u32) -> i32;
    }

    pub fn reduced_motion() -> bool {
        let mut on: i32 = 1;
        // SAFETY: this action writes one BOOL through the pointer.
        let ok =
            unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&mut on as *mut i32).cast(), 0) };
        ok != 0 && on == 0
    }

    pub fn high_contrast() -> bool {
        let size = std::mem::size_of::<HighContrast>() as u32;
        let mut hc = HighContrast { size, flags: 0, scheme: std::ptr::null_mut() };
        // SAFETY: this action fills a HIGHCONTRASTW whose size is given in both places.
        let ok = unsafe {
            SystemParametersInfoW(SPI_GETHIGHCONTRAST, size, (&mut hc as *mut HighContrast).cast(), 0)
        };
        ok != 0 && hc.flags & HCF_HIGHCONTRASTON != 0
    }
}

#[cfg(not(windows))]
mod imp {
    use std::process::{Command, Stdio};

    fn read(cmd: &[&str]) -> Option<String> {
        let out = Command::new(cmd[0]).args(&cmd[1..]).stderr(Stdio::null()).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    #[cfg(target_os = "macos")]
    pub fn reduced_motion() -> bool {
        read(&["defaults", "read", "com.apple.universalaccess", "reduceMotion"]).is_some_and(|v| v == "1")
    }

    #[cfg(target_os = "macos")]
    pub fn high_contrast() -> bool {
        read(&["defaults", "read", "com.apple.universalaccess", "increaseContrast"]).is_some_and(|v| v == "1")
    }

    /// GNOME's switches, which GTK desktops and portals also follow.
    #[cfg(not(target_os = "macos"))]
    pub fn reduced_motion() -> bool {
        read(&["gsettings", "get", "org.gnome.desktop.interface", "enable-animations"])
            .is_some_and(|v| v == "false")
    }

    #[cfg(not(target_os = "macos"))]
    pub fn high_contrast() -> bool {
        read(&["gsettings", "get", "org.gnome.desktop.a11y.interface", "high-contrast"])
            .is_some_and(|v| v == "true")
    }
}
