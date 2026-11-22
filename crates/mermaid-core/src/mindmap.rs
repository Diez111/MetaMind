//! `mindmap`: a tree written as a nested list, drawn as boxes joined by branches.

use crate::common::{self, Draw};
use crate::{Diagram, Error, Fill, Measure};

const SIDE: f32 = 40.0;
const GAP: f32 = 12.0;

struct Node {
    text: String,
    kids: Vec<Node>,
    size: (f32, f32),
    /// The height of the whole branch, children included.
    tall: f32,
}

fn indent(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ' || *c == '\t').map(|c| if c == '\t' { 4 } else { 1 }).sum()
}

fn size_of(text: &str, depth: usize, m: &mut dyn Measure) -> ((f32, f32), Vec<String>) {
    let size = if depth == 0 {
        15.0
    } else if depth == 1 {
        13.5
    } else {
        12.5
    };
    let lines = common::wrap(m, text, size, 150.0);
    let w = lines.iter().map(|l| m.width(l, size, false)).fold(0.0f32, f32::max) + 26.0;
    let h = lines.len() as f32 * common::lead(size) + 12.0;
    ((w.max(70.0), h.max(34.0)), lines)
}

/// The height of the branch a node carries: its own box, or the stack of its children.
fn measure(n: &mut Node, depth: usize, m: &mut dyn Measure) {
    let (size, _) = size_of(&n.text, depth, m);
    n.size = size;
    let mut tall = 0.0;
    let count = n.kids.len();
    for kid in &mut n.kids {
        measure(kid, depth + 1, m);
        tall += kid.tall + GAP;
    }
    n.tall = n.size.1.max(if count > 0 { tall - GAP } else { 0.0 });
}

fn paint(n: &Node, depth: usize, left: f32, mid: f32, d: &mut Draw, m: &mut dyn Measure) {
    let (w, h) = n.size;
    let fill = match depth {
        0 => Fill::Node,
        1 => Fill::Tone(1),
        2 => Fill::Tone(3),
        _ => Fill::Tone(5),
    };
    let radius = if depth == 0 { 14.0 } else { 9.0 };
    d.rect(left, mid - h / 2.0, w, h, radius, fill);
    let size = if depth == 0 {
        15.0
    } else if depth == 1 {
        13.5
    } else {
        12.5
    };
    let lines = common::wrap(m, &n.text, size, 150.0);
    let top = mid - lines.len() as f32 * common::lead(size) / 2.0;
    d.lines(m, left + w / 2.0, top, &lines, depth == 0, size);

    let mut at = mid - n.tall / 2.0;
    for kid in &n.kids {
        let kid_mid = at + kid.tall / 2.0;
        let child_left = left + w + SIDE;
        // The branch: out of the parent, across, and into the child.
        let elbow = vec![
            (left + w, mid),
            (left + w + SIDE / 2.0, mid),
            (left + w + SIDE / 2.0, kid_mid),
            (child_left, kid_mid),
        ];
        d.line(elbow, false, 1.4);
        paint(kid, depth + 1, child_left, kid_mid, d, m);
        at += kid.tall + GAP;
    }
}

pub fn render(lines: &[&str], m: &mut dyn Measure) -> Result<Diagram, Error> {
    let mut roots: Vec<Node> = Vec::new();
    // The path of the node being filled, and the indentation it was written at.
    let mut open: Vec<(Vec<usize>, usize)> = Vec::new();
    for raw in lines {
        let line = raw.trim_end();
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let word = text.split_whitespace().next().unwrap_or("");
        if matches!(word, "classDef" | "class" | "style" | "::icon") {
            continue;
        }
        let depth = indent(line);
        let node = Node { text: common::node_text(text), kids: Vec::new(), size: (0.0, 0.0), tall: 0.0 };
        while open.last().is_some_and(|(_, ind)| *ind >= depth) {
            open.pop();
        }
        if let Some((path, _)) = open.last().cloned() {
            let parent = at(&mut roots, &path);
            parent.kids.push(node);
            let mut next = path;
            next.push(parent.kids.len() - 1);
            open.push((next, depth));
        } else {
            roots.push(node);
            open.push((vec![roots.len() - 1], depth));
        }
    }
    if roots.is_empty() {
        return Err(Error::Syntax("el mapa mental está vacío".into()));
    }
    // A mindmap has one root; anything else written at its indentation hangs off the first.
    let mut root = roots.remove(0);
    for extra in roots {
        root.kids.push(extra);
    }
    measure(&mut root, 0, m);
    let mut d = Draw::new();
    paint(&root, 0, 0.0, root.tall / 2.0, &mut d, m);
    Ok(d.finish(14.0))
}

/// The node a path names, walking down the children.
fn at<'a>(roots: &'a mut [Node], path: &[usize]) -> &'a mut Node {
    let mut node = &mut roots[path[0]];
    for &i in &path[1..] {
        node = &mut node.kids[i];
    }
    node
}
