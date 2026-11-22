//! The welcome screen drawn at a desktop size, in both themes: every rect inside the viewport
//! and the three actions on screen.

use metamind_app::recent;
use metamind_app::welcome::Welcome;
use metamind_ui::{Dom, Fonts, Pixmap};

const W: u32 = 1600;
const H: u32 = 900;

fn route(dark: bool) -> Dom {
    let cfg = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("METAMIND_CONFIG_DIR", cfg.path());
    std::env::set_var("HOME", home.path());
    std::env::set_var("USERPROFILE", home.path());
    let vault = home.path().join("Documentos/boveda");
    std::fs::create_dir_all(&vault).unwrap();
    recent::remember(&vault);

    let mut w = Welcome::new();
    w.dark = dark;
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(W, H).unwrap();
    let dom = w.render(&mut fonts, &mut pm, 1.0);
    // The intro animation must not leave the card off centre by the time it is over.
    let card = dom.nodes.iter().find(|n| n.el.classes.iter().any(|c| c == "welcome-card")).unwrap();
    assert!(card.rect.x > 0.0 && card.rect.y > 0.0 && card.rect.y + card.rect.h < H as f32);
    dom
}

#[test]
fn welcome_fits_and_lists_the_three_actions_in_dark_and_light() {
    for dark in [true, false] {
        let dom = route(dark);
        let theme = if dark { "theme-dark" } else { "theme-light" };
        assert!(dom.nodes[0].el.classes.iter().any(|c| c == theme), "{theme} is set");
        for action in ["welcome:open", "welcome:create", "welcome:sample"] {
            let n = dom.nodes.iter().find(|n| n.el.action.as_deref() == Some(action));
            let n = n.unwrap_or_else(|| panic!("{action} is on screen"));
            assert!(n.rect.w > 0.0 && n.rect.h > 0.0, "{action} has a box");
            assert!(n.rect.x >= 0.0 && n.rect.x + n.rect.w <= W as f32, "{action} fits across");
            assert!(n.rect.y >= 0.0 && n.rect.y + n.rect.h <= H as f32, "{action} fits down");
        }
        // Every piece of text sits inside the viewport: nothing is cut off at the edges.
        for n in dom.nodes.iter().filter(|n| n.el.text.is_some()) {
            let r = n.rect;
            assert!(r.x >= 0.0 && r.y >= 0.0, "{:?} at {:?}", n.el.text, r);
            assert!(r.x + r.w <= W as f32 && r.y + r.h <= H as f32, "{:?} at {:?}", n.el.text, r);
        }
    }
}
