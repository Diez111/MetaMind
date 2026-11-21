//! The screen-reader tree, built from the laid-out DOM of the frame on screen. Node `i` of the DOM
//! is accessibility node `i + 1`; node 0 is the window. Clickable elements become buttons whose
//! label is their `aria-label`, text or command name. Other text becomes labels, or headings when
//! it carries `aria-level`. Images need an `aria-label`; containers without text are flattened.

use accesskit::{Action, Node, NodeId, Rect, Role, TreeId, TreeInfo, TreeUpdate};
use rustsidian_app::shell::COMMANDS;
use rustsidian_ui::Dom;

pub const ROOT: NodeId = NodeId(0);

/// DOM node behind an accessibility node, if any.
pub fn dom_node(id: NodeId) -> Option<usize> {
    (id.0 as usize).checked_sub(1)
}

pub fn tree(dom: &Dom, scale: f64, title: &str) -> TreeUpdate {
    let mut nodes = Vec::new();
    let mut kids = Vec::new();
    let mut focus = ROOT;
    collect(dom, 0, None, scale, &mut nodes, &mut kids, &mut focus);
    let mut root = Node::new(Role::Window);
    root.set_label(title);
    root.set_children(kids);
    nodes.push((ROOT, root));
    TreeUpdate { nodes, tree: Some(TreeInfo::new(ROOT)), tree_id: TreeId::ROOT, focus }
}

type Clip = Option<rustsidian_ui::Rect>;

fn collect(
    dom: &Dom,
    id: usize,
    clip: Clip,
    scale: f64,
    nodes: &mut Vec<(NodeId, Node)>,
    out: &mut Vec<NodeId>,
    focus: &mut NodeId,
) {
    let n = &dom.nodes[id];
    if n.style.display_none {
        return;
    }
    let r = n.rect;
    // A clipping ancestor (a scrolled sidebar, the editor) hides what lies outside it.
    if let Some(c) = clip {
        if r.x >= c.x + c.w || r.y >= c.y + c.h || r.x + r.w <= c.x || r.y + r.h <= c.y {
            return;
        }
    }
    let clip = if n.style.clip { Some(clip.map_or(r, |c| intersect(c, r))) } else { clip };
    let attr = |k: &str| n.el.attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
    let text = n.el.text.as_deref().map(str::trim).filter(|t| !t.is_empty());
    let mut node = if let Some(action) = &n.el.action {
        let mut b = Node::new(Role::Button);
        let mut words = Vec::new();
        descendant_text(dom, id, &mut words);
        let label = match attr("aria-label") {
            Some(l) => l.to_string(),
            None if !words.is_empty() => words.join(" "),
            None => COMMANDS.iter().find(|c| c.0 == action).map_or(action.clone(), |c| c.1.to_string()),
        };
        b.set_label(label);
        b.add_action(Action::Click);
        b
    } else if let Some(alt) = n.el.image.as_ref().and(attr("aria-label").or(text)) {
        // An image without a label (a logo) is decoration.
        let mut i = Node::new(Role::Image);
        i.set_label(alt);
        i
    } else if let Some(text) = text {
        let level = attr("aria-level").and_then(|l| l.parse().ok());
        // Screen readers take a heading's name from its label and a text label's from its value.
        if let Some(level) = level {
            let mut h = Node::new(Role::Heading);
            h.set_level(level);
            h.set_label(text);
            h
        } else {
            let mut l = Node::new(Role::Label);
            l.set_value(text);
            l
        }
    } else {
        for &c in &n.children {
            collect(dom, c, clip, scale, nodes, out, focus);
        }
        return;
    };
    let nid = NodeId(id as u64 + 1);
    let (x, y, w, h) = (f64::from(r.x), f64::from(r.y), f64::from(r.w), f64::from(r.h));
    node.set_bounds(Rect { x0: x * scale, y0: y * scale, x1: (x + w) * scale, y1: (y + h) * scale });
    if n.focus {
        *focus = nid;
    }
    // A button's text is already its label.
    if n.el.action.is_none() {
        let mut kids = Vec::new();
        for &c in &n.children {
            collect(dom, c, clip, scale, nodes, &mut kids, focus);
        }
        node.set_children(kids);
    }
    nodes.push((nid, node));
    out.push(nid);
}

fn descendant_text<'a>(dom: &'a Dom, id: usize, out: &mut Vec<&'a str>) {
    let n = &dom.nodes[id];
    if let Some(t) = n.el.text.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        out.push(t);
    }
    for &c in &n.children {
        descendant_text(dom, c, out);
    }
}

fn intersect(a: rustsidian_ui::Rect, b: rustsidian_ui::Rect) -> rustsidian_ui::Rect {
    let (x0, y0) = (a.x.max(b.x), a.y.max(b.y));
    let (x1, y1) = ((a.x + a.w).min(b.x + b.w), (a.y + a.h).min(b.y + b.h));
    rustsidian_ui::Rect { x: x0, y: y0, w: (x1 - x0).max(0.0), h: (y1 - y0).max(0.0) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustsidian_ui::El;

    fn lay_out(dom: &mut Dom) {
        let theme = rustsidian_ui::Theme::default();
        dom.style(&theme, rustsidian_ui::Env::default());
        dom.layout(&mut rustsidian_ui::Fonts::new(), 400.0, 300.0);
    }

    #[test]
    fn buttons_headings_and_text_are_exposed() {
        let mut dom = Dom::new();
        let bar = dom.add(0, El::div());
        let b = dom.add(bar, El::div().action("workspace:export-pdf"));
        dom.add(b, El::div());
        dom.add(bar, El::div().action("app:open-vault").attr("aria-label", "Open"));
        let link = dom.add(bar, El::div().action("open:Note"));
        dom.add(link, El::div().text("Note"));
        dom.add(0, El::div().attr("aria-level", "2").text("Title"));
        dom.add(0, El::div().text("Body text."));
        dom.add(0, El::div().text("   "));
        lay_out(&mut dom);
        let update = tree(&dom, 2.0, "Vault");
        let get = |id: NodeId| &update.nodes.iter().find(|(n, _)| *n == id).unwrap().1;
        let root = get(ROOT);
        assert_eq!(root.label(), Some("Vault"));
        let kids: Vec<_> = root.children().iter().map(|&k| get(k)).collect();
        let summary: Vec<_> = kids.iter().map(|k| (k.role(), k.label().or(k.value()))).collect();
        assert_eq!(
            summary,
            [
                (Role::Button, Some("Exportar a PDF")),
                (Role::Button, Some("Open")),
                (Role::Button, Some("Note")),
                (Role::Heading, Some("Title")),
                (Role::Label, Some("Body text.")),
            ]
        );
        assert_eq!(kids[3].level(), Some(2));
        assert!(kids[0].supports_action(Action::Click));
        let body = kids[4].bounds().unwrap();
        let laid = dom.nodes[dom_node(root.children()[4]).unwrap()].rect;
        assert_eq!(body.x1 - body.x0, f64::from(laid.w) * 2.0);
    }

    #[test]
    fn the_app_window_reads_as_buttons_and_headings() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("A.md"), "# Alpha\n\nSome prose.\n").unwrap();
        std::fs::write(d.path().join("B.md"), "b\n").unwrap();
        let mut app = rustsidian_app::App::open(d.path()).unwrap();
        app.open_note("A");
        let mut fonts = rustsidian_ui::Fonts::new();
        let mut pm = rustsidian_ui::Pixmap::new(1100, 700).unwrap();
        let frame = rustsidian_app::render(&mut app, &mut fonts, &mut pm, 1.0);
        let update = tree(&frame.dom, 1.0, "v");
        let find = |role: Role, label: &str| {
            update
                .nodes
                .iter()
                .find(|(_, n)| n.role() == role && n.label().or(n.value()) == Some(label))
                .map(|(id, _)| *id)
        };
        let all: Vec<_> = update
            .nodes
            .iter()
            .map(|(_, n)| (n.role(), n.label().or(n.value()).map(str::to_string)))
            .collect();
        // The caret sits on the heading, so live preview shows its markup.
        assert!(find(Role::Heading, "# Alpha").is_some(), "{all:?}");
        assert!(find(Role::Button, "Cambiar orden").is_some(), "{all:?}");
        assert!(find(Role::Button, "Plegar encabezado").is_some(), "{all:?}");
        assert!(find(Role::Label, "Some prose.").is_some());
        let b = find(Role::Button, "B").expect("file explorer row");
        let r = frame.dom.nodes[dom_node(b).unwrap()].rect;
        app.click(&frame, &mut fonts, r.x + r.w / 2.0, r.y + r.h / 2.0, false);
        app.release();
        let frame = rustsidian_app::render(&mut app, &mut fonts, &mut pm, 1.0);
        let update = tree(&frame.dom, 1.0, "v");
        assert!(update.nodes.iter().any(|(_, n)| n.value() == Some("b")), "B is open");
    }
}
