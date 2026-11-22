//! Checking for and installing new releases from GitHub.
//!
//! Network access goes through the system `curl` and archives are unpacked with the system
//! `tar` (both ship with Windows 10 and later, macOS and Linux), so no HTTP or TLS stack is
//! compiled in. Every download is checked against the release's `SHA256SUMS.txt` before the
//! running binary is replaced.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver};

pub const LATEST_URL: &str = "https://api.github.com/repos/Diez111/MetMind/releases/latest";
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// Automatic checks run at most this often.
const CHECK_EVERY_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub tag: String,
    pub page: String,
    pub asset: String,
    pub asset_url: String,
    pub sums_url: String,
}

/// The release archive name suffix for this build, as published by the release workflow.
pub fn platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("windows-x86_64.zip"),
        ("macos", "aarch64") => Some("macos-arm64.tar.gz"),
        ("macos", "x86_64") => Some("macos-x86_64.tar.gz"),
        ("linux", "x86_64") => Some("linux-x86_64.tar.gz"),
        _ => None,
    }
}

fn version(s: &str) -> Option<(u64, u64, u64)> {
    let core = s.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((it.next()??, it.next().flatten().unwrap_or(0), it.next().flatten().unwrap_or(0)))
}

/// Whether `tag` names a later version than `current`.
pub fn is_newer(tag: &str, current: &str) -> bool {
    matches!((version(tag), version(current)), (Some(a), Some(b)) if a > b)
}

/// Read the GitHub "latest release" response for the archive matching `suffix`.
pub fn parse_release(json: &str, suffix: &str) -> Result<Release, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("respuesta inesperada: {e}"))?;
    if let Some(msg) = v.get("message").and_then(Value::as_str) {
        return Err(format!("GitHub: {msg}"));
    }
    let tag = v.get("tag_name").and_then(Value::as_str).ok_or("la respuesta no tiene tag_name")?.to_string();
    let assets = v.get("assets").and_then(Value::as_array).cloned().unwrap_or_default();
    let find = |pred: &dyn Fn(&str) -> bool| {
        assets.iter().find_map(|a| {
            let name = a.get("name")?.as_str()?;
            let url = a.get("browser_download_url")?.as_str()?;
            pred(name).then(|| (name.to_string(), url.to_string()))
        })
    };
    let (asset, asset_url) =
        find(&|n| n.ends_with(suffix)).ok_or_else(|| format!("{tag} no tiene descarga para {suffix}"))?;
    let (_, sums_url) =
        find(&|n| n == "SHA256SUMS.txt").ok_or_else(|| format!("{tag} no tiene sumas de verificación"))?;
    let page = v.get("html_url").and_then(Value::as_str).unwrap_or_default().to_string();
    Ok(Release { tag, page, asset, asset_url, sums_url })
}

/// A command that opens no console window when started from the windowed app on Windows.
fn tool(name: &str) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(name);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn curl(url: &str, out: Option<&Path>) -> Result<Vec<u8>, String> {
    let mut cmd = tool("curl");
    // `-q` first: the user's `~/.curlrc` cannot add flags (an `--insecure` there would turn
    // certificate checking off for the update) and a redirect cannot drop to `file://` or another
    // protocol. TLS verification itself is curl's default and is never disabled here.
    cmd.args(["-q", "-fsSL", "--proto-redir", "=https,http", "--max-time", "300"]);
    cmd.args(["-H", "Accept: application/vnd.github+json"]);
    cmd.args(["-A", concat!("metamind/", env!("CARGO_PKG_VERSION"))]);
    if let Some(p) = out {
        cmd.arg("-o").arg(p);
    }
    let o = cmd.arg(url).output().map_err(|e| format!("no se pudo ejecutar curl: {e}"))?;
    if !o.status.success() {
        let err = String::from_utf8_lossy(&o.stderr);
        return Err(format!("falló la descarga: {}", err.trim()));
    }
    Ok(o.stdout)
}

/// Ask GitHub for the latest release. `Ok(None)` means this build is up to date.
pub fn check() -> Result<Option<Release>, String> {
    let suffix = platform().ok_or("no hay compilaciones publicadas para este sistema")?;
    let body = curl(LATEST_URL, None)?;
    let r = parse_release(&String::from_utf8_lossy(&body), suffix)?;
    Ok(is_newer(&r.tag, CURRENT).then_some(r))
}

/// The expected digest of `name` in a `sha256sum` listing.
pub fn expected_sum(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (hash, file) = l.split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name).then(|| hash.to_ascii_lowercase())
    })
}

/// Download, verify and unpack `r`, then put its binary in place of `exe`. The running
/// program keeps working; the new version starts on the next launch.
pub fn install(r: &Release, exe: &Path) -> Result<(), String> {
    let work = std::env::temp_dir().join(format!("metamind-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("no se pudo crear {}: {e}", work.display()))?;
    let result = install_in(r, exe, &work);
    // Leftovers in the temp folder are harmless, so a failed cleanup is not reported.
    let _ = std::fs::remove_dir_all(&work);
    result
}

/// Every member of a release archive has to stay inside the folder it is unpacked into.
///
/// The archive is verified against the published checksum first, so what this stops is a
/// corrupted or hostile archive (or a `tar` that would follow a `-C` target out of the way)
/// writing outside the temporary folder: `tar` alone happily honours `../` members.
fn safe_members(list: &str) -> Result<(), String> {
    use std::path::Component;
    for name in list.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let escapes =
            Path::new(name).components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir));
        if escapes {
            return Err(format!("la descarga trae {name:?}, que sale de la carpeta de trabajo"));
        }
    }
    Ok(())
}

fn install_in(r: &Release, exe: &Path, work: &Path) -> Result<(), String> {
    let archive = work.join(&r.asset);
    curl(&r.asset_url, Some(&archive))?;
    let sums = String::from_utf8_lossy(&curl(&r.sums_url, None)?).into_owned();
    let want =
        expected_sum(&sums, &r.asset).ok_or_else(|| format!("{} no está en SHA256SUMS.txt", r.asset))?;
    let bytes = std::fs::read(&archive).map_err(|e| format!("no se pudo leer la descarga: {e}"))?;
    let got = hex(&sha256(&bytes));
    if got != want {
        return Err(format!(
            "suma de verificación incorrecta para {} (se esperaba {want}, se obtuvo {got})",
            r.asset
        ));
    }
    let listing =
        tool("tar").arg("-tf").arg(&archive).output().map_err(|e| format!("no se pudo ejecutar tar: {e}"))?;
    if !listing.status.success() {
        return Err(format!(
            "no se pudo leer el contenido de {}: {}",
            r.asset,
            String::from_utf8_lossy(&listing.stderr).trim()
        ));
    }
    safe_members(&String::from_utf8_lossy(&listing.stdout))?;
    let o = tool("tar")
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(work)
        .output()
        .map_err(|e| format!("no se pudo ejecutar tar: {e}"))?;
    if !o.status.success() {
        return Err(format!(
            "no se pudo descomprimir {}: {}",
            r.asset,
            String::from_utf8_lossy(&o.stderr).trim()
        ));
    }
    let name = exe.file_name().ok_or("ruta del programa no válida")?;
    let dir = r.asset.trim_end_matches(".zip").trim_end_matches(".tar.gz");
    let new = work.join(dir).join(name);
    if !new.is_file() {
        return Err(format!("{} no contiene {}", r.asset, Path::new(dir).join(name).display()));
    }
    replace(&new, exe)
}

/// The previous binary, moved aside on Windows where a running program cannot be overwritten.
pub fn old_path(exe: &Path) -> PathBuf {
    let mut s = exe.as_os_str().to_owned();
    s.push(".old");
    PathBuf::from(s)
}

fn replace(new: &Path, exe: &Path) -> Result<(), String> {
    let staged = {
        let mut s = exe.as_os_str().to_owned();
        s.push(".new");
        PathBuf::from(s)
    };
    let err = |what: &str, e: std::io::Error| format!("no se pudo {what} en {}: {e}", exe.display());
    std::fs::copy(new, &staged).map_err(|e| err("escribir la versión nueva", e))?;
    // Windows allows renaming a running executable but not replacing it, so the old one is
    // moved aside first and deleted on the next start.
    if cfg!(windows) {
        let old = old_path(exe);
        let _ = std::fs::remove_file(&old);
        std::fs::rename(exe, &old).map_err(|e| err("mover la versión anterior", e))?;
        if let Err(e) = std::fs::rename(&staged, exe) {
            let _ = std::fs::rename(&old, exe);
            return Err(err("instalar la versión nueva", e));
        }
    } else {
        std::fs::rename(&staged, exe).map_err(|e| err("instalar la versión nueva", e))?;
    }
    Ok(())
}

/// Remove the binary left behind by an update on Windows.
pub fn clean_old() {
    if let Ok(exe) = std::env::current_exe() {
        let old = old_path(&exe);
        if old.exists() {
            if let Err(e) = std::fs::remove_file(&old) {
                eprintln!("metamind: cannot remove {}: {e}", old.display());
            }
        }
    }
}

/// Per-user update preferences in the configuration folder.
fn prefs_file() -> Option<PathBuf> {
    crate::recent::config_dir().map(|d| d.join("updates.json"))
}

fn prefs() -> Value {
    prefs_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null)
}

fn write_prefs(v: &Value) -> Result<(), String> {
    let f = prefs_file().ok_or("no hay carpeta de configuración")?;
    if let Some(d) = f.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("no se pudo crear {}: {e}", d.display()))?;
    }
    std::fs::write(&f, v.to_string()).map_err(|e| format!("no se pudo escribir {}: {e}", f.display()))
}

pub fn auto_check() -> bool {
    prefs().get("autoCheck").and_then(Value::as_bool).unwrap_or(true)
}

pub fn set_auto_check(on: bool) -> Result<(), String> {
    let mut v = prefs();
    if !v.is_object() {
        v = serde_json::json!({});
    }
    v["autoCheck"] = Value::Bool(on);
    write_prefs(&v)
}

/// Whether an automatic check is due, recording the time if so.
pub fn auto_check_due(now_ms: u64) -> bool {
    let mut v = prefs();
    if !v.is_object() {
        v = serde_json::json!({});
    }
    if !v.get("autoCheck").and_then(Value::as_bool).unwrap_or(true) {
        return false;
    }
    let last = v.get("lastCheck").and_then(Value::as_u64).unwrap_or(0);
    if now_ms.saturating_sub(last) < CHECK_EVERY_MS {
        return false;
    }
    v["lastCheck"] = now_ms.into();
    if let Err(e) = write_prefs(&v) {
        eprintln!("metamind: {e}");
    }
    true
}

pub enum State {
    Idle,
    Checking { manual: bool, rx: Receiver<Result<Option<Release>, String>> },
    UpToDate,
    Available(Release),
    Installing { tag: String, rx: Receiver<Result<(), String>> },
    Installed(String),
    Failed(String),
}

impl State {
    pub fn busy(&self) -> bool {
        matches!(self, State::Checking { .. } | State::Installing { .. })
    }

    /// Start a check in the background.
    pub fn start_check(&mut self, manual: bool) {
        if self.busy() {
            return;
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(check());
        });
        *self = State::Checking { manual, rx };
    }

    pub fn start_install(&mut self) -> Result<(), String> {
        let State::Available(r) = self else {
            return Err("no hay ninguna actualización para instalar".into());
        };
        let exe = std::env::current_exe()
            .map_err(|e| format!("no se pudo encontrar el archivo del programa: {e}"))?;
        let (tx, rx) = channel();
        let r = r.clone();
        let tag = r.tag.clone();
        std::thread::spawn(move || {
            let _ = tx.send(install(&r, &exe));
        });
        *self = State::Installing { tag, rx };
        Ok(())
    }

    /// Collect a finished background job. Returns a message to show, if any.
    pub fn poll(&mut self) -> Option<String> {
        let (next, msg) = match self {
            State::Checking { manual, rx } => match rx.try_recv().ok()? {
                Ok(Some(r)) => {
                    let msg =
                        format!("MetaMind {} está disponible. Instálala desde Ajustes → Acerca de.", r.tag);
                    (State::Available(r), Some(msg))
                }
                Ok(None) => {
                    let msg = manual.then(|| format!("MetaMind v{CURRENT} es la última versión."));
                    (State::UpToDate, msg)
                }
                // Automatic checks fail quietly offline; the error is still in Settings.
                Err(e) => {
                    let msg = manual.then(|| format!("No se pudo comprobar si hay actualizaciones: {e}"));
                    (State::Failed(e), msg)
                }
            },
            State::Installing { tag, rx } => match rx.try_recv().ok()? {
                Ok(()) => {
                    let msg = format!("MetaMind {tag} está instalada. Reinicia para usarla.");
                    (State::Installed(tag.clone()), Some(msg))
                }
                Err(e) => (State::Failed(e.clone()), Some(format!("Falló la actualización: {e}"))),
            },
            _ => return None,
        };
        *self = next;
        msg
    }

    /// One line for the About section.
    pub fn describe(&self) -> String {
        match self {
            State::Idle => "Aún no se ha comprobado.".into(),
            State::Checking { .. } => "Comprobando…".into(),
            State::UpToDate => "Tienes la última versión.".into(),
            State::Available(r) => format!("{} está disponible.", r.tag),
            State::Installing { tag, .. } => format!("Instalando {tag}…"),
            State::Installed(tag) => format!("{tag} está instalada. Reinicia MetaMind para usarla."),
            State::Failed(e) => e.clone(),
        }
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// SHA-256 (FIPS 180-4).
// `as_chunks` needs Rust 1.88; the workspace supports 1.85.
#[allow(clippy::chunks_exact_to_as_chunks)]
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] =
        [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, c) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 32];
    for (i, x) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_vectors() {
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let long = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(hex(&sha256(long)), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("v0.10.0", "0.2.0"));
        assert!(is_newer("v0.2.1", "0.2.0"));
        assert!(!is_newer("v0.2.0", "0.2.0"));
        assert!(!is_newer("v0.1.9", "0.2.0"));
        assert!(!is_newer("nightly", "0.2.0"));
    }

    #[test]
    fn parses_release_and_sums() {
        let json = r#"{"tag_name":"v0.3.0","html_url":"https://example.invalid/r","assets":[
            {"name":"metamind-v0.3.0-linux-x86_64.tar.gz","browser_download_url":"u1"},
            {"name":"metamind-v0.3.0-windows-x86_64.zip","browser_download_url":"u2"},
            {"name":"SHA256SUMS.txt","browser_download_url":"u3"}]}"#;
        let r = parse_release(json, "windows-x86_64.zip").unwrap();
        assert_eq!((r.tag.as_str(), r.asset_url.as_str(), r.sums_url.as_str()), ("v0.3.0", "u2", "u3"));
        assert!(parse_release(json, "macos-arm64.tar.gz")
            .unwrap_err()
            .contains("no tiene descarga para macos-arm64"));
        assert!(parse_release(r#"{"message":"Not Found"}"#, "x").unwrap_err().contains("Not Found"));
        let sums = "abc  metamind-v0.3.0-windows-x86_64.zip\nDEF *other.tar.gz\n";
        assert_eq!(expected_sum(sums, "metamind-v0.3.0-windows-x86_64.zip").as_deref(), Some("abc"));
        assert_eq!(expected_sum(sums, "other.tar.gz").as_deref(), Some("def"));
        assert_eq!(expected_sum(sums, "missing"), None);
    }

    /// A hostile or corrupted archive must not be unpacked outside the work folder.
    #[test]
    fn an_archive_that_climbs_out_of_the_work_folder_is_refused() {
        assert!(safe_members("").is_ok());
        assert!(
            safe_members("metamind-v9.0.0-linux-x86_64/\nmetamind-v9.0.0-linux-x86_64/metamind\n").is_ok()
        );
        for bad in ["../evil", "../../.bashrc", "/etc/cron.d/job", "pkg/../../evil", "pkg/.."] {
            assert!(safe_members(bad).is_err(), "{bad}");
        }
        #[cfg(windows)]
        for bad in ["C:/Windows/evil.dll", "\\\\server\\share\\evil", "c:evil"] {
            assert!(safe_members(bad).is_err(), "{bad}");
        }
    }

    /// The full download, verify, unpack and replace path against local `file://` URLs.
    #[cfg(unix)]
    #[test]
    fn installs_from_archive_and_rejects_bad_checksum() {
        let d = tempfile::tempdir().unwrap();
        let pkg = d.path().join("metamind-v9.0.0-linux-x86_64");
        std::fs::create_dir(&pkg).unwrap();
        std::fs::write(pkg.join("metamind"), b"new build").unwrap();
        let asset = "metamind-v9.0.0-linux-x86_64.tar.gz";
        let ok = Command::new("tar")
            .args(["czf", asset, "metamind-v9.0.0-linux-x86_64"])
            .current_dir(d.path())
            .status()
            .unwrap();
        assert!(ok.success());
        let sum = hex(&sha256(&std::fs::read(d.path().join(asset)).unwrap()));
        std::fs::write(d.path().join("SHA256SUMS.txt"), format!("{sum}  {asset}\n")).unwrap();
        let bin = d.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let exe = bin.join("metamind");
        std::fs::write(&exe, b"old build").unwrap();
        let url = |f: &str| format!("file://{}", d.path().join(f).display());
        let mut r = Release {
            tag: "v9.0.0".into(),
            page: String::new(),
            asset: asset.into(),
            asset_url: url(asset),
            sums_url: url("SHA256SUMS.txt"),
        };
        install(&r, &exe).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new build");

        std::fs::write(&exe, b"old build").unwrap();
        std::fs::write(d.path().join("bad.txt"), format!("{}  {asset}\n", "0".repeat(64))).unwrap();
        r.sums_url = url("bad.txt");
        assert!(install(&r, &exe).unwrap_err().contains("suma de verificación incorrecta"));
        assert_eq!(std::fs::read(&exe).unwrap(), b"old build");
    }
}
