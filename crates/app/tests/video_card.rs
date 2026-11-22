//! The card of a video embed: its own first frame with the play button on top, and nothing but
//! the button in a compact window. The frame comes from an injected decoder, so no test here
//! starts `ffmpeg`: the real one is one process per file and belongs to the app, not to a run of
//! the suite.

use metamind_app::posters::Posters;
use metamind_app::{render, App, Frame};
use metamind_ui::{Fonts, Image, Pixmap};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A note that embeds a video, and the file it points at. The bytes are not a film on purpose:
/// the card is what is being checked, and the decoder is a fake.
fn vault() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("Video.md"), "# Video\n\n![[clip.mp4]]\n\ntexto después\n").unwrap();
    std::fs::write(d.path().join("clip.mp4"), b"esto no es un video, es un archivo que existe").unwrap();
    d
}

fn poster() -> Image {
    Image(Arc::new(Pixmap::new(768, 432).unwrap()))
}

fn frame(app: &mut App, fonts: &mut Fonts, w: u32, h: u32) -> Frame {
    let mut pm = Pixmap::new(w, h).unwrap();
    render(app, fonts, &mut pm, 1.0);
    render(app, fonts, &mut pm, 1.0)
}

/// Renders until the injected decode has landed (it runs in its own thread), then one more frame
/// so the card is built with the poster in the cache.
fn settle(app: &mut App, fonts: &mut Fonts, w: u32, h: u32) -> Frame {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut f = frame(app, fonts, w, h);
    while app.posters.busy() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(2));
        f = frame(app, fonts, w, h);
    }
    f
}

fn node(f: &Frame, class: &str) -> Option<metamind_ui::dom::NodeId> {
    f.dom.nodes.iter().position(|n| n.el.classes.iter().any(|c| c == class))
}

fn open(poster: bool) -> (tempfile::TempDir, App) {
    let d = vault();
    let mut app = App::open(d.path()).unwrap();
    app.posters = if poster { Posters::with_decoder(|_| Some(self::poster())) } else { Posters::default() };
    app.open_note("Video.md");
    (d, app)
}

#[test]
fn the_card_shows_the_frame_with_the_play_button_on_top() {
    let (_d, mut app) = open(true);
    let mut fonts = Fonts::new();
    let f = settle(&mut app, &mut fonts, 1366, 768);

    let poster = node(&f, "media-card-poster").expect("the card shows the frame");
    let play = node(&f, "media-card-play").expect("the play button is over the frame");
    let card = node(&f, "media-card").expect("the card under the frame");
    let editor = f.info.editor.map(|n| &f.dom.nodes[n]).expect("the note column");

    let p = &f.dom.nodes[poster];
    let button = &f.dom.nodes[play];
    let c = &f.dom.nodes[card];
    let e = editor;
    // The picture is the note's width, inside the note's own column, and the card follows it:
    // one block, one left edge, nothing hanging out of the pane.
    assert!(p.rect.w > 0.0 && p.rect.h > 0.0, "the frame has a box: {:?}", p.rect);
    assert!(p.rect.x >= e.rect.x && p.rect.x + p.rect.w <= e.rect.x + e.rect.w + 1.0, "the frame stays in the column");
    assert!((p.rect.w / p.rect.h - 768.0 / 432.0).abs() < 0.05, "the frame keeps its own shape");
    assert!((p.rect.w - e.rect.w.min(700.0)).abs() <= 1.0, "the frame is as wide as the column");
    assert!(p.rect.y + p.rect.h <= c.rect.y + 1.0, "the card sits under the frame: {:?} {:?}", p.rect, c.rect);
    assert!((c.rect.x - p.rect.x).abs() <= 1.0, "and shares its left edge");
    assert!((button.rect.x + button.rect.w / 2.0 - (p.rect.x + p.rect.w / 2.0)).abs() < 2.0, "the button is centred");
    assert!((button.rect.y + button.rect.h / 2.0 - (p.rect.y + p.rect.h / 2.0)).abs() < 2.0, "and over the middle");
    assert!(button.rect.w <= p.rect.w && button.rect.h <= p.rect.h, "the button fits inside the frame");

    // The button is a real target: a press on it opens the player, not the note.
    let (x, y) = (button.rect.x + button.rect.w / 2.0, button.rect.y + button.rect.h / 2.0);
    let (_, action) = f.dom.action_at(x, y).expect("the button answers the pointer");
    assert!(action.starts_with("video:open:"), "it opens the player: {action}");
}

#[test]
fn a_compact_window_keeps_the_card_and_its_button() {
    let (_d, mut app) = open(true);
    let mut fonts = Fonts::new();
    let f = settle(&mut app, &mut fonts, 360, 640);

    assert!(app.compact, "360 px is a compact window");
    assert!(node(&f, "media-card-poster").is_none(), "no frame is shown");
    assert!(node(&f, "media-card-play").is_none(), "and no button over it");
    let action = node(&f, "media-card-action").expect("the card keeps its own button");
    let a = &f.dom.nodes[action];
    assert!(f.dom.action_at(a.rect.x + a.rect.w / 2.0, a.rect.y + a.rect.h / 2.0).is_some());
    assert!(a.rect.x >= 0.0 && a.rect.x + a.rect.w <= 360.0, "the card fits the window");
}

#[test]
fn a_file_that_does_not_decode_keeps_the_card_as_it_was() {
    let (_d, mut app) = open(true);
    app.posters = Posters::with_decoder(|_| None);
    let mut fonts = Fonts::new();
    let f = settle(&mut app, &mut fonts, 1366, 768);

    assert!(node(&f, "media-card-poster").is_none(), "there is no frame to show");
    let action = node(&f, "media-card-action").expect("the card still opens the player");
    let a = &f.dom.nodes[action];
    assert!(f.dom.action_at(a.rect.x + a.rect.w / 2.0, a.rect.y + a.rect.h / 2.0).is_some());
}
