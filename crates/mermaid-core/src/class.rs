//! `classDiagram`: boxes with a name and its compartments, joined by associations, inheritance,
//! composition and aggregation.

use crate::common::{self, Dir, Draw, Head, Link};
use crate::{Diagram, Error, Fill, Measure};

const TITLE: f32 = 13.0;
const MEMBER: f32 = 11.5;

/// The operators a class relation can be written with, longest first, and what they paint at
/// each end.
const RELS: &[(&str, Head, Head)] = &[
    ("<|--", Head::Triangle, Head::None),
    ("<|..", Head::Triangle, Head::None),
    ("--|>", Head::None, Head::Triangle),
    ("..|>", Head::None, Head::Triangle),
    ("*--", Head::Diamond, Head::None),
    ("--*", Head::None, Head::Diamond),
    ("o--", Head::DiamondOpen, Head::None),
    ("--o", Head::None, Head::DiamondOpen),
    ("<-->", Head::Arrow, Head::Arrow),
    ("<--", Head::Arrow, Head::None),
    ("-->", Head::None, Head::Arrow),
    ("<..", Head::Open, Head::None),
    ("..>", Head::None, Head::Open),
    ("--", Head::None, Head::None),
    ("..", Head::None, Head::None),
];

struct Class {
    id: String,
    name: String,
    /// `<<interface>>`, `<<abstract>>`, and the like.
    stereo: Option<String>,
    attrs: Vec<String>,
    methods: Vec<String>,
    size: (f32, f32),
}

impl Class {
    fn new(id: &str) -> Self {
        Class {
            id: id.to_string(),
            name: id.to_string(),
            stereo: None,
            attrs: Vec::new(),
            methods: Vec::new(),
            size: (0.0, 0.0),
        }
    }

    fn measure(&mut self, m: &mut dyn Measure) -> (f32, f32) {
        let title = common::wrap(m, &self.name, TITLE, 180.0);
        let tw = title.iter().map(|l| m.width(l, TITLE, true)).fold(0.0f32, f32::max);
        let sw = self.stereo.as_ref().map_or(0.0, |s| m.width(s, 10.0, false));
        let aw = self.attrs.iter().map(|l| m.width(l, MEMBER, false)).fold(0.0f32, f32::max);
        let mw = self.methods.iter().map(|l| m.width(l, MEMBER, false)).fold(0.0f32, f32::max);
        let w = tw.max(sw).max(aw + 22.0).max(mw + 22.0).max(96.0) + 24.0;
        let mut h = 18.0 + title.len() as f32 * 17.0;
        if self.stereo.is_some() {
            h += 14.0;
        }
        if !self.attrs.is_empty() {
            h += 7.0 + self.attrs.len() as f32 * 15.0;
        }
        if !self.methods.is_empty() {
            h += 7.0 + self.methods.len() as f32 * 15.0;
        }
        self.size = (w, h.max(36.0));
        self.size
    }

    fn paint(&self, d: &mut Draw, m: &mut dyn Measure, c: (f32, f32)) {
        let (w, h) = self.size;
        let (x, y) = (c.0 - w / 2.0, c.1 - h / 2.0);
        d.rect(x, y, w, h, 6.0, Fill::Node);
        let mut at = y + 10.0;
        for l in common::wrap(m, &self.name, TITLE, 180.0) {
            d.text(m, c.0, at + 8.5, &l, true, TITLE);
            at += 17.0;
        }
        if let Some(s) = &self.stereo {
            d.text(m, c.0, at + 7.0, s, false, 10.0);
            at += 14.0;
        }
        at += 4.0;
        for members in [&self.attrs, &self.methods] {
            if members.is_empty() {
                continue;
            }
            d.line(vec![(x, at), (x + w, at)], false, 1.0);
            at += 7.0;
            for line in members {
                d.text_left(m, x + 11.0, at + 7.5, line, false, MEMBER);
                at += 15.0;
            }
        }
    }
}

/// The class with that id, created empty if this is the first time it is named.
fn class(classes: &mut Vec<Class>, id: &str) -> usize {
    let id = id.trim();
    match classes.iter().position(|c| c.id == id) {
        Some(i) => i,
        None => {
            classes.push(Class::new(id));
            classes.len() - 1
        }
    }
}

/// Splits `text` into the id of a class and the cardinality written next to it, which mermaid
/// allows on either side of a relation: `Order "1" *-- "many" Line`.
fn take_card(text: &str, at_start: bool) -> (String, String) {
    let s = text.trim();
    if at_start {
        if let Some(rest) = s.strip_prefix('"') {
            if let Some(end) = rest.find('"') {
                return (rest[end + 1..].trim().to_string(), rest[..end].to_string());
            }
        }
        return (s.to_string(), String::new());
    }
    if let Some(end) = s.rfind('"') {
        if let Some(start) = s[..end].rfind('"') {
            return (s[..start].trim().to_string(), s[start + 1..end].to_string());
        }
    }
    (s.to_string(), String::new())
}

/// The class a `class` statement names, and the label it gives it.
fn declaration(rest: &str) -> (String, Option<String>) {
    let rest = rest.trim();
    let head = match rest.find('{') {
        Some(i) => rest[..i].trim(),
        None => rest,
    };
    let head = head.split(":::").next().unwrap_or("").trim();
    let (id, label) = match (head.find('['), head.rfind(']')) {
        (Some(a), Some(b)) if b > a => (head[..a].trim().to_string(), Some(common::clean(&head[a + 1..b]))),
        _ => (head.to_string(), None),
    };
    (id, label)
}

pub fn render(lines: &[&str], m: &mut dyn Measure) -> Result<Diagram, Error> {
    let mut classes: Vec<Class> = Vec::new();
    let mut links: Vec<Link> = Vec::new();
    let mut notes: Vec<(usize, String)> = Vec::new();
    let mut dir = Dir::Down;
    let mut body: Option<usize> = None;
    let mut last: Option<usize> = None;

    for raw in lines {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(idx) = body {
            match line {
                "}" => {
                    body = None;
                    continue;
                }
                "{" | "" => continue,
                _ => {
                    let member = common::clean(line);
                    if let Some(s) = member.strip_prefix("<<").and_then(|r| r.strip_suffix(">>")) {
                        classes[idx].stereo = Some(format!("<<{s}>>"));
                    } else if member.contains('(') {
                        classes[idx].methods.push(member);
                    } else {
                        classes[idx].attrs.push(member);
                    }
                    continue;
                }
            }
        }
        let word = line.split_whitespace().next().unwrap_or("");
        match word {
            "class" => {
                let (id, label) = declaration(&line["class".len()..]);
                let opens = line.contains('{');
                if id.is_empty() {
                    if opens {
                        body = last;
                    }
                    continue;
                }
                let idx = class(&mut classes, &id);
                if let Some(l) = label {
                    classes[idx].name = l;
                }
                last = Some(idx);
                if opens {
                    body = Some(idx);
                }
            }
            "namespace" | "}" => {}
            "note" => {
                let rest = line["note".len()..].trim();
                let (who, text) = if let Some(r) = rest.strip_prefix("for ") {
                    let (id, text) = r.split_once(char::is_whitespace).unwrap_or((r, ""));
                    (id.to_string(), text)
                } else {
                    let (text, id) = match rest.rfind(" for ") {
                        Some(i) => (rest[..i].trim(), rest[i + 5..].trim()),
                        None => (rest, ""),
                    };
                    (id.to_string(), text)
                };
                if who.is_empty() {
                    continue;
                }
                let idx = class(&mut classes, &who);
                notes.push((idx, common::clean(text)));
            }
            "direction" => dir = Dir::parse(&line["direction".len()..], Dir::Down)?,
            "title" | "accTitle" | "accDescr" | "classDef" | "cssClass" | "style" | "click" | "link"
            | "callback" => {}
            _ => {
                // `<<interface>> Shape`: the stereotype of a class named on the line.
                if let Some((stereo, id)) = line.strip_prefix("<<").and_then(|r| r.split_once(">>")) {
                    let idx = class(&mut classes, id.trim());
                    classes[idx].stereo = Some(format!("<<{}>>", stereo.trim()));
                    continue;
                }
                match (find_relation(line), line.split_once(':')) {
                    // A member of a class written on a line of its own: `Animal : +int age`.
                    (Some((at, _, _, _)), Some((id, _))) if at > line.find(':').unwrap_or(at) => {
                        member(&mut classes, id, line.split_once(':').map(|c| c.1).unwrap_or(""));
                    }
                    (Some(_), _) => relation(line, &mut classes, &mut links)?,
                    (None, Some((id, member_text))) if !id.trim().is_empty() => {
                        member(&mut classes, id, member_text);
                    }
                    (None, _) => {
                        return Err(Error::Syntax(format!(
                            "no entiendo la línea del diagrama de clases: `{line}`"
                        )))
                    }
                }
            }
        }
    }
    if classes.is_empty() {
        return Err(Error::Syntax("el diagrama de clases no tiene clases".into()));
    }
    Ok(paint(classes, links, notes, dir, m))
}

/// Adds a `+int age` or `+isMammal() bool` line to the class it belongs to.
fn member(classes: &mut Vec<Class>, id: &str, text: &str) {
    let idx = class(classes, id.trim());
    let member = common::clean(text);
    if member.is_empty() {
        return;
    }
    if member.contains('(') {
        classes[idx].methods.push(member);
    } else {
        classes[idx].attrs.push(member);
    }
}

/// The relation operator of a line, if it has one: where it starts, its text, and the heads it
/// paints at each end.
fn find_relation(line: &str) -> Option<(usize, &str, Head, Head)> {
    let mut best: Option<(usize, &str, Head, Head)> = None;
    for &(op, from_head, to_head) in RELS {
        if let Some(i) = line.find(op) {
            let better = match best {
                None => true,
                Some((bi, bop, _, _)) => i < bi || (i == bi && op.len() > bop.len()),
            };
            if better {
                best = Some((i, op, from_head, to_head));
            }
        }
    }
    best
}

/// Whether `text` can be the name of a class.
fn valid_id(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty() && t.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-'))
}

/// A relation line: `A "1" *-- "many" B : contains`.
fn relation(line: &str, classes: &mut Vec<Class>, links: &mut Vec<Link>) -> Result<(), Error> {
    let Some((i, op, from_head, to_head)) = find_relation(line) else {
        return Err(Error::Syntax(format!("no entiendo la línea del diagrama de clases: `{line}`")));
    };
    let (left, left_card) = take_card(&line[..i], false);
    let right = &line[i + op.len()..];
    let (right_rest, right_card) = take_card(right, true);
    let (right_id, label) = match right_rest.split_once(':') {
        Some((id, l)) => (id.trim().to_string(), l.trim().to_string()),
        None => (right_rest.clone(), String::new()),
    };
    if !valid_id(&left) || !valid_id(&right_id) {
        return Err(Error::Syntax(format!("relación incompleta: `{line}`")));
    }
    let a = class(classes, &left);
    let b = class(classes, &right_id);
    let mut link = Link::new(a, b).with_label(&label);
    link.from_label = left_card;
    link.to_label = right_card;
    link.head_from = from_head;
    link.head_to = to_head;
    link.dashed = op.contains('.');
    links.push(link);
    Ok(())
}

fn paint(
    mut classes: Vec<Class>,
    mut links: Vec<Link>,
    notes: Vec<(usize, String)>,
    dir: Dir,
    m: &mut dyn Measure,
) -> Diagram {
    let mut sizes: Vec<(f32, f32)> = Vec::with_capacity(classes.len() + notes.len());
    for c in &mut classes {
        sizes.push(c.measure(m));
    }
    let mut note_nodes: Vec<(String, f32, f32)> = Vec::new();
    for (i, (owner, text)) in notes.iter().enumerate() {
        let lines = common::wrap(m, text, MEMBER, 170.0);
        let w = lines.iter().map(|l| m.width(l, MEMBER, false)).fold(0.0f32, f32::max) + 20.0;
        let h = lines.len() as f32 * 15.0 + 18.0;
        sizes.push((w, h));
        let mut link = Link::new(*owner, classes.len() + i);
        link.hidden = true;
        links.push(link);
        note_nodes.push((text.clone(), w, h));
    }
    let lay = common::place(&sizes, &links, dir);
    let mut d = Draw::new();
    common::draw_links(&mut d, m, &sizes, &lay, &links, dir);
    for (i, c) in classes.iter().enumerate() {
        c.paint(&mut d, m, lay.centers[i]);
    }
    for (i, (text, w, h)) in note_nodes.iter().enumerate() {
        let c = lay.centers[classes.len() + i];
        d.rect(c.0 - w / 2.0, c.1 - h / 2.0, *w, *h, 4.0, Fill::Accent);
        let lines = common::wrap(m, text, MEMBER, 170.0);
        d.lines(m, c.0, c.1 - h / 2.0 + 9.0, &lines, false, MEMBER);
    }
    d.finish(16.0)
}
