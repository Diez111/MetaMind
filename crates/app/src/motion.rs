//! The operating system's "reduce motion" accessibility preference.

/// True when the OS asks applications to keep animation to a minimum.
/// An unreadable setting counts as "animations allowed", the OS default everywhere.
pub fn os_reduced_motion() -> bool {
    imp::reduced()
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::ffi::c_void;

    // Settings > Accessibility > Visual effects > Animation effects.
    const SPI_GETCLIENTAREAANIMATION: u32 = 0x1042;

    #[link(name = "user32")]
    extern "system" {
        fn SystemParametersInfoW(action: u32, param: u32, value: *mut c_void, ini: u32) -> i32;
    }

    pub fn reduced() -> bool {
        let mut on: i32 = 1;
        // SAFETY: this action writes one BOOL through the pointer.
        let ok =
            unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&mut on as *mut i32).cast(), 0) };
        ok != 0 && on == 0
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
    pub fn reduced() -> bool {
        read(&["defaults", "read", "com.apple.universalaccess", "reduceMotion"]).is_some_and(|v| v == "1")
    }

    /// GNOME's switch, which GTK desktops and portals also follow.
    #[cfg(not(target_os = "macos"))]
    pub fn reduced() -> bool {
        read(&["gsettings", "get", "org.gnome.desktop.interface", "enable-animations"])
            .is_some_and(|v| v == "false")
    }
}
