//! Hand a request to an already running MetMind that has the same vault open, so links and
//! file associations reuse its window instead of opening the vault twice.
//!
//! Each running window listens on a loopback port and records the port and a random token in
//! `<config>/instances/<pid>`. A new process tries each record; the one showing the vault answers
//! `ok`. Records whose process is gone are removed.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(2);

pub fn dir() -> Option<PathBuf> {
    crate::recent::config_dir().map(|d| d.join("instances"))
}

/// A listening instance, before it starts answering.
pub struct Instance {
    listener: TcpListener,
    token: String,
}

impl Instance {
    /// Start listening and write this process's record into `dir`.
    pub fn register(dir: &Path) -> std::io::Result<Instance> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        let token = token();
        std::fs::create_dir_all(dir)?;
        write_private(&dir.join(std::process::id().to_string()), &format!("{port}\n{token}\n"))?;
        Ok(Instance { listener, token })
    }

    /// Answer requests until the process ends. `accept(vault, note)` says whether this instance
    /// shows `vault` and, if so, takes the note.
    pub fn serve(self, accept: impl Fn(&Path, Option<String>) -> bool + Send + 'static) {
        let Instance { listener, token } = self;
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = answer(stream, &token, &accept);
            }
        });
    }
}

/// Remove this process's record. Called on a normal exit, since the serving thread never ends.
pub fn unregister(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(std::process::id().to_string()));
}

fn answer(
    stream: TcpStream,
    token: &str,
    accept: &impl Fn(&Path, Option<String>) -> bool,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(TIMEOUT))?;
    let mut lines = BufReader::new(&stream).lines();
    let mut next = || lines.next().transpose().map(Option::unwrap_or_default);
    let (given, vault, note) = (next()?, next()?, next()?);
    let ok = given == token
        && !vault.is_empty()
        && accept(Path::new(&vault), Some(note).filter(|n| !n.is_empty()));
    (&stream).write_all(if ok { b"ok\n" } else { b"no\n" })
}

/// Ask a running instance to show `note` in `vault`. True when one took it.
pub fn forward(dir: &Path, vault: &Path, note: Option<&str>) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    let vault = std::fs::canonicalize(vault).unwrap_or_else(|_| vault.to_path_buf());
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let mut it = text.lines();
        let (Some(Ok(port)), Some(token)) = (it.next().map(str::parse::<u16>), it.next()) else {
            let _ = std::fs::remove_file(&path);
            continue;
        };
        match ask(port, token, &vault, note) {
            Ok(true) => return true,
            Ok(false) => {}
            Err(_) => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    false
}

fn ask(port: u16, token: &str, vault: &Path, note: Option<&str>) -> std::io::Result<bool> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&addr, TIMEOUT)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    let line = |s: &str| s.replace(['\n', '\r'], " ");
    let msg = format!("{token}\n{}\n{}\n", line(&vault.to_string_lossy()), line(note.unwrap_or("")));
    stream.write_all(msg.as_bytes())?;
    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply)?;
    Ok(reply.trim() == "ok")
}

/// Whether two vault paths name the same folder.
pub fn same_vault(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canon(a) == canon(b)
}

/// 128 bits from the OS-seeded hasher keys `std` uses, so a process that only can reach the port
/// cannot drive the window.
fn token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let part = || {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()),
        );
        h.finish()
    };
    format!("{:016x}{:016x}", part(), part())
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(path)?.write_all(text.as_bytes())
}
