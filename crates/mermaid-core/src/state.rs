//! `stateDiagram-v2`: the states of a machine, the transitions between them, and the states that
//! are themselves a machine.

use crate::common::{self, Block, Dir, Draw, Link};
use crate::{Diagram, Error, Fill, Measure};

const TITLE: f32 = 18.0;
const PAD: f32 = 12.0;

fn current(stack: &mut [Group]) -> Result<&mut Group, Error> {
    stack.last_mut().ok_or_else(|| Error::Syntax("el diagrama de estados está vacío".into()))
}

enum Kind {
    State,
    /// `[*]` where a machine starts: a filled dot.
    Start,
    /// `[*]` where it ends: a dot in a ring.
    End,
    Note,
}

struct Node {
    id: String,
    label: String,
    desc: Option<String>,
    kind: Kind,
}

struct Edge {
    from: String,
    to: String,
    label: String,
}

struct Group {
    /// The state this group is the inside of; empty for the root.
    name: String,
    nodes: Vec<Node>,
    groups: Vec<Group>,
    edges: Vec<Edge>,
}

impl Group {
    fn new(name: &str) -> Self {
        Group { name: name.to_string(), nodes: Vec::new(), groups: Vec::new(), edges: Vec::new() }
    }

    /// The state with that id, created with the same text as its label if this is the first time
    /// the source names it.
    fn state(&mut self, id: &str) -> usize {
        match self.nodes.iter().position(|n| n.id == id) {
            Some(i) => i,
            None => {
                self.nodes.push(Node {
                    id: id.to_string(),
                    label: id.to_string(),
                    desc: None,
                    kind: Kind::State,
                });
                self.nodes.len() - 1
            }
        }
    }

    fn index(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }
}

/// `state "Long name" as S` and `state S`: the id, and the text to draw in the box.
fn declaration(rest: &str) -> (String, String) {
    let rest = rest.trim().trim_end_matches('{').trim();
    if let Some(i) = rest.find(" as ") {
        let left = rest[..i].trim();
        let id = rest[i + 4..].trim().to_string();
        (id, common::clean(left))
    } else {
        (rest.to_string(), common::clean(rest))
    }
}

pub fn render(lines: &[&str], m: &mut dyn Measure) -> Result<Diagram, Error> {
    let mut dir = Dir::Down;
    let mut stack: Vec<Group> = vec![Group::new("")];
    let mut counter = 0usize;
    for raw in lines {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let word = line.split_whitespace().next().unwrap_or("");
        match word {
            "direction" => dir = Dir::parse(&line["direction".len()..], Dir::Down)?,
            "state" => {
                let rest = line["state".len()..].trim();
                let (id, label) = declaration(rest);
                if line.ends_with('{') {
                    let mut g = Group::new(&id);
                    g.name = label;
                    stack.push(g);
                } else {
                    let g = current(&mut stack)?;
                    let idx = g.state(&id);
                    g.nodes[idx].label = label;
                }
            }
            "}" => {
                if stack.len() > 1 {
                    if let Some(g) = stack.pop() {
                        current(&mut stack)?.groups.push(g);
                    }
                }
            }
            "note" => {
                let rest = line["note".len()..].trim();
                let (who, text) = match rest.split_once(':') {
                    Some((head, text)) => {
                        let who = head
                            .trim()
                            .trim_start_matches("right of ")
                            .trim_start_matches("left of ")
                            .trim_start_matches("over ")
                            .split(',')
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        (who, common::clean(text.trim()))
                    }
                    None => (String::new(), String::new()),
                };
                if who.is_empty() || text.is_empty() {
                    continue;
                }
                let g = current(&mut stack)?;
                counter += 1;
                let id = format!("__nota{counter}");
                g.nodes.push(Node { id: id.clone(), label: text, desc: None, kind: Kind::Note });
                g.edges.push(Edge { from: who, to: id, label: String::new() });
            }
            "class" | "classDef" | "style" | "hide" | "title" | "accTitle" | "accDescr" => {}
            _ => {
                let Some(i) = line.find("-->") else {
                    return Err(Error::Syntax(format!(
                        "no entiendo la línea del diagrama de estados: `{line}`"
                    )));
                };
                let from = line[..i].trim();
                let rest = line[i + 3..].trim();
                let (to, label) = match rest.split_once(':') {
                    Some((t, l)) => (t.trim(), common::clean(l.trim())),
                    None => (rest, String::new()),
                };
                let g = current(&mut stack)?;
                let from = end(g, from, Kind::Start, &mut counter);
                let to = end(g, to, Kind::End, &mut counter);
                let link = g.edges.iter_mut().find(|e| e.from == from && e.to == to);
                match link {
                    Some(e) if e.label.is_empty() => e.label = label,
                    _ => g.edges.push(Edge { from, to, label }),
                }
            }
        }
    }
    while stack.len() > 1 {
        if let Some(g) = stack.pop() {
            current(&mut stack)?.groups.push(g);
        }
    }
    let root = stack.pop().unwrap_or_else(|| Group::new(""));
    if root.nodes.is_empty() && root.groups.is_empty() {
        return Err(Error::Syntax("el diagrama de estados no tiene estados".into()));
    }
    let mut d = Draw::new();
    let b = block(&root, dir, false, m);
    d.place(0.0, 0.0, &b);
    Ok(d.finish(16.0))
}

/// The id a transition names, creating the `[*]` pseudo state it stands for.
fn end(g: &mut Group, id: &str, kind: Kind, counter: &mut usize) -> String {
    if id != "[*]" {
        g.state(id);
        return id.to_string();
    }
    *counter += 1;
    let name = format!("__pseudo{counter}");
    g.nodes.push(Node { id: name.clone(), label: String::new(), desc: None, kind });
    name
}

/// Lays out what is inside a group and draws it, with the frame it is enclosed in if it is the
/// inside of a state of its own.
fn block(g: &Group, dir: Dir, framed: bool, m: &mut dyn Measure) -> Block {
    let mut sizes: Vec<(f32, f32)> = Vec::with_capacity(g.nodes.len() + g.groups.len());
    for n in &g.nodes {
        sizes.push(node_size(n, m));
    }
    let mut subs: Vec<Block> = Vec::new();
    for sub in &g.groups {
        let b = block(sub, dir, true, m);
        sizes.push((b.w, b.h));
        subs.push(b);
    }
    let mut links: Vec<Link> = Vec::new();
    for e in &g.edges {
        let (a, b) = (slot(g, &e.from), slot(g, &e.to));
        let (Some(a), Some(b)) = (a, b) else { continue };
        let mut link = Link::new(a, b).with_label(&e.label);
        // A note is only there to have room of its own next to the state it talks about.
        link.hidden = matches!(g.nodes.get(a), Some(Node { kind: Kind::Note, .. }));
        links.push(link);
    }
    let lay = common::place(&sizes, &links, dir);
    let mut d = Draw::new();
    common::draw_links(&mut d, m, &sizes, &lay, &links, dir);
    for (i, n) in g.nodes.iter().enumerate() {
        paint_node(&mut d, m, n, lay.centers[i]);
    }
    for (i, b) in subs.iter().enumerate() {
        let c = lay.centers[g.nodes.len() + i];
        d.place(c.0 - b.w / 2.0, c.1 - b.h / 2.0, b);
    }
    let content = d.into_block(if framed { PAD } else { 0.0 });
    if !framed {
        return content;
    }
    let mut outer = Draw::new();
    let (w, h) = (content.w + 2.0 * PAD, content.h + 2.0 * PAD + TITLE);
    outer.rect(0.0, 0.0, w, h, 10.0, Fill::Accent);
    outer.text_left(m, PAD + 2.0, TITLE / 2.0 + 3.0, &g.name, true, 12.0);
    outer.place(PAD, PAD + TITLE, &content);
    outer.into_block(0.0)
}

/// Where a transition endpoint sits in the layer list of its group: the states first, then the
/// groups that are themselves a state.
fn slot(g: &Group, id: &str) -> Option<usize> {
    if let Some(i) = g.index(id) {
        return Some(i);
    }
    g.groups.iter().position(|s| s.name == id).map(|i| g.nodes.len() + i)
}

fn node_size(n: &Node, m: &mut dyn Measure) -> (f32, f32) {
    match n.kind {
        Kind::Start => (18.0, 18.0),
        Kind::End => (24.0, 24.0),
        _ => {
            let size = if matches!(n.kind, Kind::Note) { 11.0 } else { 12.0 };
            let lines = common::wrap(m, &n.label, size, 160.0);
            let w = lines.iter().map(|l| m.width(l, size, false)).fold(0.0f32, f32::max);
            let desc = n.desc.as_ref().map_or(0.0, |d| m.width(d, 10.0, false) + 8.0);
            let h = lines.len() as f32 * common::lead(size) + 16.0 + if desc > 0.0 { 13.0 } else { 0.0 };
            (w.max(desc) + 28.0, h.max(34.0))
        }
    }
}

fn paint_node(d: &mut Draw, m: &mut dyn Measure, n: &Node, c: (f32, f32)) {
    match n.kind {
        Kind::Start => d.ellipse(c.0, c.1, 7.0, 7.0, Fill::Node),
        Kind::End => {
            d.ellipse(c.0, c.1, 9.0, 9.0, Fill::Node);
            d.ellipse(c.0, c.1, 4.0, 4.0, Fill::Background);
        }
        Kind::State | Kind::Note => {
            let (w, h) = node_size(n, m);
            let note = matches!(n.kind, Kind::Note);
            let fill = if note { Fill::Accent } else { Fill::Node };
            d.rect(c.0 - w / 2.0, c.1 - h / 2.0, w, h, if note { 4.0 } else { 9.0 }, fill);
            let size = if note { 11.0 } else { 12.0 };
            let lines = common::wrap(m, &n.label, size, 160.0);
            let total = lines.len() as f32 * common::lead(size) + if n.desc.is_some() { 13.0 } else { 0.0 };
            let top = c.1 - total / 2.0;
            d.lines(m, c.0, top - 1.5, &lines, false, size);
            if let Some(desc) = &n.desc {
                let y = top + lines.len() as f32 * common::lead(size) + 5.0;
                d.text(m, c.0, y, desc, false, 10.0);
            }
        }
    }
}
