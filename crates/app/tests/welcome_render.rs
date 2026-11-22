//! The welcome screen drawn at a desktop size, in both themes: every rect inside the viewport
//! and the three actions on screen.

use metamind_app::recent;
use metamind_app::welcome::Welcome;
use metamind_ui::{Dom, Fonts, Pixmap};

const W: u32 = 1600;
const H: u32 = 900;

/// Whether a node lives inside a list that scrolls, so its rows may sit past the fold.
fn in_scroller(dom: &Dom, i: usize) -> bool {
    let mut cur = Some(i);
    while let Some(n) = cur {
        if dom.nodes[n].el.classes.iter().any(|c| c == "welcome-list") {
            return true;
        }
        cur = dom.nodes[n].parent;
    }
    false
}

fn route(dark: bool) -> Dom {
    let cfg = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("METAMIND_CONFIG_DIR", cfg.path());
    std::env::set_var("HOME", home.path());
    std::env::set_var("USERPROFILE", home.path());
    let vault = home.path().join("Documentos/boveda");
    std::fs::create_dir_all(&vault).unwrap();
    recent::remember(&vault);
    // More vaults than fit without scrolling: the card must not grow off screen.
    for i in 0..8 {
        let path = home.path().join(format!("notas/boveda-{i}"));
        std::fs::create_dir_all(&path).unwrap();
        recent::remember(&path);
    }

    let mut w = Welcome::new();
    w.dark = dark;
    let mut fonts = Fonts::new();
    let mut pm = Pixmap::new(W, H).unwrap();
    let dom = w.render(&mut fonts, &mut pm, 1.0);
    // The intro animation must not leave the card off centre by the time it is over.
    let card = dom.nodes.iter().find(|n| n.el.classes.iter().any(|c| c == "welcome-card")).unwrap();
    assert!(card.rect.x > 0.0 && card.rect.w > 0.0 && card.rect.h > 0.0, "{:?}", card.rect);
    assert!(card.rect.x + card.rect.w <= W as f32 && card.rect.y + card.rect.h <= H as f32);
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
        // Every piece of text sits inside the card across. Down the page it is either inside the
        // card, or below the fold of the list that scrolls (a row can be half out at the fold).
        let card = dom.nodes.iter().find(|n| n.el.classes.iter().any(|c| c == "welcome-card")).unwrap();
        let (left, right) = (card.rect.x, card.rect.x + card.rect.w);
        let bottom = card.rect.y + card.rect.h;
        for (i, n) in dom.nodes.iter().enumerate().filter(|(_, n)| n.el.text.is_some()) {
            let r = n.rect;
            assert!(r.x >= left - 0.5 && r.x + r.w <= right + 0.5, "{:?} across at {:?}", n.el.text, r);
            assert!(r.y >= 0.0, "{:?} above the window at {:?}", n.el.text, r);
            if !in_scroller(&dom, i) {
                assert!(r.y + r.h <= bottom + 0.5, "{:?} cut at {:?}", n.el.text, r);
            }
        }
    }
}
