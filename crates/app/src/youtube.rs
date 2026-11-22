//! YouTube links in notes, played inside MetaMind.
//!
//! A note that carries a YouTube link (`youtube.com/watch?v=…`, `youtu.be/…`, `…/shorts/…`,
//! `…/embed/…`) shows a card under the line, the same shape the local media cards use. Pressing it
//! asks `yt-dlp` — the only thing this needs, no key and no paid service — for the direct URL of
//! the stream, and hands that URL to the player in [`crate::video`], which reads https as well as
//! it reads a file. Without `yt-dlp` the card offers the browser instead and says why.
//!
//! `yt-dlp` takes seconds, so it runs on a thread of its own and the interface polls for the
//! answer; nothing here blocks a frame.

use crate::video::find_program;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

/// A YouTube video a note links to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// The eleven characters that identify the video.
    pub id: String,
    /// The watch URL, which is what the player is keyed by and what the browser opens.
    pub url: String,
}

impl Link {
    fn new(id: &str) -> Link {
        Link { id: id.to_string(), url: format!("https://www.youtube.com/watch?v={id}") }
    }
}

/// The markers a YouTube link can carry, and where the id sits after each.
const MARKERS: [&str; 4] = ["youtu.be/", "youtube.com/watch?v=", "youtube.com/shorts/", "youtube.com/embed/"];

/// Length of a YouTube video id.
const ID_LEN: usize = 11;

/// The first YouTube video `text` links to, if it links to one.
///
/// Both a bare URL and a Markdown link `[title](https://…)` are read, and the match is refused
/// when the marker is glued to another word (`notyoutube.com/…`), so a link to some other site
/// never turns into a card.
pub fn find(text: &str) -> Option<Link> {
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some((at, marker)) =
        MARKERS.iter().filter_map(|m| lower[from..].find(m).map(|i| (from + i, *m))).min_by_key(|(i, _)| *i)
    {
        let before = &lower[..at];
        // The marker has to start the host, not sit inside another name: `notyoutube.com` and
        // `evil-youtube.com` are some other site, `www.` and `music.` are YouTube's own.
        let starts_a_url = before.chars().next_back().is_none_or(|c| {
            !(c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
                || before.ends_with("www.")
                || before.ends_with("music.")
                || before.ends_with("m.")
        });
        if starts_a_url {
            // The id comes out of the original text: YouTube's ids are case sensitive, and the
            // lower-cased copy is only there to find the marker.
            if let Some(id) = video_id(&text[at + marker.len()..]) {
                return Some(Link::new(id));
            }
        }
        from = at + marker.len();
        if from >= lower.len() {
            break;
        }
    }
    None
}

/// The eleven characters of an id at the start of `rest`, when they look like one.
fn video_id(rest: &str) -> Option<&str> {
    let id: String =
        rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    // `then_some` would slice the string even when the id is too short.
    (id.len() == ID_LEN).then(|| &rest[..ID_LEN])
}

/// The `yt-dlp` on `PATH`, if there is one.
pub fn tool() -> Option<PathBuf> {
    find_program("yt-dlp", std::env::var_os("PATH"))
}

/// What `yt-dlp` is asked for: the direct URL of the best video stream it can hand over without a
/// key. A progressive `mp4` first, since it plays from the first byte; the picture only anyway,
/// as the player is muted.
pub fn resolve_args(url: &str) -> Vec<String> {
    [
        "-f",
        "bv*[ext=mp4][height<=720]/bv*[height<=720]/bv*/best[ext=mp4]/best",
        "-g",
        "--no-playlist",
        "--no-warnings",
        // Nothing is written to disk: the URL is the whole answer.
        "--no-simulate",
        url,
    ]
    .map(String::from)
    .to_vec()
}
/// The first URL `yt-dlp` printed, out of everything it said.
pub fn first_url(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("https://") || l.starts_with("http://"))
        .map(str::to_string)
}

/// Whether `url` is a plain web link to YouTube itself: an `http`/`https` scheme, no user
/// info, and a host of YouTube (or one of its subdomains). This is the gate in front of
/// `yt-dlp`, which is handed nothing else.
pub fn is_youtube_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else { return false };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    // `user:pass@host` would let the real host hide after the `@`.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') || authority.contains('\\') {
        return false;
    }
    // Port and an empty host are not accepted either.
    let host = match authority.rsplit_once(':') {
        Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) && !port.is_empty() => h,
        _ => authority,
    };
    let host = host.to_ascii_lowercase();
    let host = host
        .strip_prefix("www.")
        .or_else(|| host.strip_prefix("music."))
        .or_else(|| host.strip_prefix("m."))
        .unwrap_or(&host);
    host == "youtube.com" || host == "youtu.be" || host.ends_with(".youtube.com")
}

/// Run `yt-dlp` for `url` and wait for the stream URL. Blocking: the caller runs it on a thread.
pub fn resolve(tool: &Path, url: &str) -> Result<String, String> {
    if !is_youtube_url(url) {
        return Err(format!("no es una dirección de YouTube: {url}"));
    }
    let out = std::process::Command::new(tool)
        .args(resolve_args(url))
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("no se pudo ejecutar yt-dlp: {e}"))?;
    if let Some(found) = first_url(&String::from_utf8_lossy(&out.stdout)) {
        return Ok(found);
    }
    let reason = last_line(&String::from_utf8_lossy(&out.stderr));
    Err(if reason.is_empty() {
        "yt-dlp no dio ninguna dirección para ese video".to_string()
    } else {
        reason
    })
}

/// The last thing a tool said: the first lines are warnings, the last one is the reason.
fn last_line(text: &str) -> String {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    lines.next_back().unwrap_or("").to_string()
}

/// A `yt-dlp` run in the background, and the answer once it arrives.
#[derive(Default)]
pub struct Jobs {
    /// `yt-dlp` found on `PATH` when the application started; `None` means the card offers the
    /// browser instead.
    tool: Option<PathBuf>,
    /// The video being resolved, and the channel its answer will come down.
    pending: Option<Pending>,
}

struct Pending {
    /// What the card looks the job up by: the watch URL.
    watch: String,
    /// The note whose card started it, so the player is drawn there.
    note: String,
    rx: Receiver<Result<String, String>>,
}

/// What a finished job produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A stream URL to play, for the video the card shows.
    Ready { watch: String, url: String, note: String },
    /// Why there is nothing to play; the message goes to the notice.
    Failed(String),
}

impl Jobs {
    /// The jobs of a running application: `yt-dlp` is looked for once.
    pub fn new() -> Jobs {
        Jobs { tool: tool(), pending: None }
    }

    /// Whether videos can be played inside the application at all.
    pub fn available(&self) -> bool {
        self.tool.is_some()
    }

    /// The watch URL of the video being resolved right now, if any.
    pub fn resolving(&self) -> Option<&str> {
        self.pending.as_ref().map(|p| p.watch.as_str())
    }

    /// Whether a `yt-dlp` run is in flight, so the window keeps waking up for it.
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// Ask `yt-dlp` for the stream of `watch`, to be shown in `note`. Returns why not, when it
    /// cannot even start.
    pub fn start(&mut self, watch: &str, note: &str) -> Result<(), String> {
        let Some(tool) = self.tool.clone() else {
            return Err("No está instalado yt-dlp: el video solo se puede ver en el navegador.".to_string());
        };
        let (tx, rx) = channel();
        let url = watch.to_string();
        std::thread::spawn(move || {
            let _ = tx.send(resolve(&tool, &url));
        });
        self.pending = Some(Pending { watch: watch.to_string(), note: note.to_string(), rx });
        Ok(())
    }

    /// The answer of the running job, once it has one.
    pub fn poll(&mut self) -> Option<Outcome> {
        let pending = self.pending.as_ref()?;
        let got = pending.rx.try_recv().ok()?;
        let Pending { watch, note, .. } = self.pending.take()?;
        Some(match got {
            Ok(url) => Outcome::Ready { watch, url, note },
            Err(e) => Outcome::Failed(format!("No se pudo preparar el video de YouTube: {e}")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str) -> Option<String> {
        find(text).map(|l| l.id)
    }

    #[test]
    fn the_usual_shapes_of_a_youtube_link_are_read() {
        for text in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "http://youtube.com/watch?v=dQw4w9WgXcQ&t=42s",
            "youtube.com/watch?v=dQw4w9WgXcQ",
            "www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ?t=30",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
            "https://www.youtube.com/embed/dQw4w9WgXcQ",
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ",
            // What comes after the id does not change which video it is.
            "https://youtu.be/dQw4w9WgXcQ/extra",
            "mirá esto: [el video](https://youtu.be/dQw4w9WgXcQ \"título\") y listo",
            "<https://youtu.be/dQw4w9WgXcQ>",
        ] {
            assert_eq!(found(text).as_deref(), Some("dQw4w9WgXcQ"), "{text}");
        }
    }

    #[test]
    fn the_card_opens_the_watch_url() {
        let link = find("https://youtu.be/dQw4w9WgXcQ?t=9").expect("a link");
        assert_eq!(link.url, "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
        assert_eq!(link.clone().id, "dQw4w9WgXcQ");
    }

    #[test]
    fn what_is_not_a_youtube_video_is_not_a_card() {
        for text in [
            "",
            "https://example.com/watch?v=dQw4w9WgXcQ",
            // The marker inside another host name: not YouTube.
            "https://notyoutube.com/watch?v=dQw4w9WgXcQ",
            "https://evil-youtube.com/watch?v=dQw4w9WgXcQ",
            // An id one character short, and one that is not an id at all.
            "https://youtu.be/dQw4w9WgXc",
            "https://youtu.be/",
            "https://www.youtube.com/watch?list=PL1234567890",
            "sin enlaces acá",
        ] {
            assert_eq!(found(text), None, "{text}");
        }
    }

    #[test]
    fn a_link_is_read_once_per_line_and_the_first_one_wins() {
        let text = "primero https://youtu.be/aaaaaaaaaaa\nsegundo https://youtu.be/bbbbbbbbbbb";
        assert_eq!(found(text).as_deref(), Some("aaaaaaaaaaa"));
    }

    /// `yt-dlp` is asked for a direct stream, in Spanish-free argv nobody has to guess.
    #[test]
    fn yt_dlp_is_asked_for_a_url_and_not_for_a_download() {
        let args = resolve_args("https://www.youtube.com/watch?v=aaaaaaaaaaa");
        assert!(args.iter().any(|a| a == "-g"), "{args:?}");
        assert!(args.iter().any(|a| a == "--no-playlist"), "{args:?}");
        assert!(!args.iter().any(|a| a == "-o" || a.starts_with("--output")), "{args:?}");
        assert_eq!(args.last().map(String::as_str), Some("https://www.youtube.com/watch?v=aaaaaaaaaaa"));
        let f = args.iter().position(|a| a == "-f").expect("a format is asked for");
        assert_eq!(args[f + 1], "bv*[ext=mp4][height<=720]/bv*[height<=720]/bv*/best[ext=mp4]/best");
    }

    #[test]
    fn only_a_youtube_link_reaches_yt_dlp() {
        for good in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com/watch?v=dQw4w9WgXcQ",
            "http://youtu.be/dQw4w9WgXcQ",
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://m.youtube.com/shorts/dQw4w9WgXcQ",
            "HTTPS://WWW.YOUTUBE.COM/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com:443/watch?v=dQw4w9WgXcQ",
        ] {
            assert!(is_youtube_url(good), "{good}");
        }
        for bad in [
            "",
            "file:///etc/passwd",
            "javascript://youtube.com/x",
            "ftp://youtube.com/x",
            "https://example.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com.evil.example/watch?v=dQw4w9WgXcQ",
            "https://notyoutube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com@evil.example/watch?v=dQw4w9WgXcQ",
            "https://evil.example\\@youtube.com/x",
            "https://",
            "--help",
            "https://youtube.com.evil:80/x",
        ] {
            assert!(!is_youtube_url(bad), "{bad}");
        }
        // A tool that is not there is never asked, but a URL that is not YouTube is refused
        // before the spawn anyway (here `true` is the tool: it would succeed if it ran).
        let err = resolve(Path::new("/bin/true"), "file:///etc/passwd").unwrap_err();
        assert!(err.contains("YouTube"), "{err}");
    }

    #[test]
    fn the_first_url_is_the_answer() {
        assert_eq!(
            first_url("WARNING: something\nhttps://rr1.googlevideo.com/x?y=1\nhttps://rr2/audio\n")
                .as_deref(),
            Some("https://rr1.googlevideo.com/x?y=1")
        );
        assert_eq!(first_url("ERROR: nope\n"), None);
        assert_eq!(first_url(""), None);
        assert_eq!(last_line("WARNING: a\n\nERROR: [youtube] nope\n"), "ERROR: [youtube] nope");
        assert_eq!(last_line(""), "");
    }

    /// Without `yt-dlp` the card says so and nothing is spawned.
    #[test]
    fn a_job_without_yt_dlp_does_not_start() {
        let mut jobs = Jobs { tool: None, pending: None };
        assert!(!jobs.available());
        assert!(!jobs.busy());
        let err = jobs.start("https://youtu.be/aaaaaaaaaaa", "Note.md").unwrap_err();
        assert!(err.contains("yt-dlp"), "{err}");
        assert!(!jobs.busy());
        assert!(jobs.poll().is_none());
    }

    /// A job resolves on its own thread, and the answer names the video and the note.
    #[test]
    fn a_finished_job_hands_over_the_stream_url() {
        let mut jobs = Jobs { tool: Some(PathBuf::from("/bin/true")), pending: None };
        let (tx, rx) = channel();
        jobs.pending = Some(Pending {
            watch: "https://www.youtube.com/watch?v=aaaaaaaaaaa".to_string(),
            note: "Note.md".to_string(),
            rx,
        });
        assert!(jobs.busy());
        assert_eq!(jobs.resolving(), Some("https://www.youtube.com/watch?v=aaaaaaaaaaa"));
        // Nothing yet: the thread has not answered.
        assert!(jobs.poll().is_none());
        tx.send(Ok("https://rr1.googlevideo.com/v?x=1".to_string())).unwrap();
        let out = jobs.poll().expect("the answer");
        assert_eq!(
            out,
            Outcome::Ready {
                watch: "https://www.youtube.com/watch?v=aaaaaaaaaaa".to_string(),
                url: "https://rr1.googlevideo.com/v?x=1".to_string(),
                note: "Note.md".to_string(),
            }
        );
        assert!(!jobs.busy());
        assert!(jobs.poll().is_none(), "the answer is handed over once");
    }

    #[test]
    fn a_failed_job_says_why() {
        let mut jobs = Jobs { tool: Some(PathBuf::from("/bin/true")), pending: None };
        let (tx, rx) = channel();
        jobs.pending = Some(Pending { watch: "w".to_string(), note: "N.md".to_string(), rx });
        let _ = tx.send(Err("ERROR: la extracción falló".to_string()));
        match jobs.poll() {
            Some(Outcome::Failed(msg)) => assert!(msg.contains("la extracción falló"), "{msg}"),
            other => panic!("{other:?}"),
        }
        assert!(!jobs.busy());
    }
}
