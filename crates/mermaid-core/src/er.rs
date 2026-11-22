//! `erDiagram`: entities with their attributes, joined by the relations that say how many of
//! each take part.

use crate::common::{self, Dir, Draw, Link};
use crate::{Diagram, Error, Fill, Measure};

struct Attr {
    kind: String,
    name: String,
    keys: String,
}

struct Entity {
    name: String,
    attrs: Vec<Attr>,
    size: (f32, f32),
}

struct Rel {
    from: usize,
    to: usize,
    /// The cardinality written at each end: `|`, `o`, `{`, `}` and their pairs.
    left: String,
    right: String,
    label: String,
    dashed: bool,
}

impl Entity {
    fn measure(&mut self, m: &mut dyn Measure) -> (f32, f32) {
        let tw = m.width(&self.name, 12.5, true);
        let mut w = tw + 26.0;
        for a in &self.attrs {
            let line = format!("{} {}", a.kind, a.name);
            let kw = if a.keys.is_empty() { 0.0 } else { m.width(&a.keys, 9.5, true) + 12.0 };
            w = w.max(m.width(&line, 11.0, false) + kw + 22.0);
        }
        let h = 28.0 + self.attrs.len() as f32 * 18.0 + if self.attrs.is_empty() { 6.0 } else { 0.0 };
        self.size = (w.max(96.0), h);
        self.size
    }

    fn paint(&self, d: &mut Draw, m: &mut dyn Measure, c: (f32, f32)) {
        let (w, h) = self.size;
        let (x, y) = (c.0 - w / 2.0, c.1 - h / 2.0);
        d.rect(x, y, w, h, 6.0, Fill::Node);
        d.text(m, c.0, y + 15.0, &self.name, true, 12.5);
        if self.attrs.is_empty() {
            return;
        }
        d.line(vec![(x, y + 28.0), (x + w, y + 28.0)], false, 1.0);
        for (i, a) in self.attrs.iter().enumerate() {
            let row = y + 28.0 + i as f32 * 18.0;
            if i > 0 {
                d.line(vec![(x + 1.0, row), (x + w - 1.0, row)], false, 0.6);
            }
            let line = format!("{} {}", a.kind, a.name);
            d.text_left(m, x + 11.0, row + 9.5, &line, false, 11.0);
            if !a.keys.is_empty() {
                d.text_right(m, x + w - 11.0, row + 9.5, &a.keys, true, 9.5);
            }
        }
    }
}

/// `name` or `name["Label"]`: the entity's id and the text drawn in its title.
fn entity_name(text: &str) -> String {
    let t = text.split('[').next().unwrap_or(text).trim();
    common::clean(t)
}

pub fn render(lines: &[&str], m: &mut dyn Measure) -> Result<Diagram, Error> {
    let mut entities: Vec<Entity> = Vec::new();
    let mut rels: Vec<Rel> = Vec::new();
    let mut open: Option<usize> = None;

    for raw in lines {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(idx) = open {
            if line == "}" {
                open = None;
                continue;
            }
            let mut it = line.split_whitespace();
            let kind = it.next().unwrap_or("").to_string();
            let name = it.next().unwrap_or("").to_string();
            if kind.is_empty() || name.is_empty() {
                return Err(Error::Syntax(format!("atributo sin tipo o sin nombre: `{line}`")));
            }
            let keys = it.map(common::clean).collect::<Vec<_>>().join(" ");
            entities[idx].attrs.push(Attr { kind, name: name.replace('"', ""), keys });
            continue;
        }
        if let Some(head) = line.strip_suffix('{') {
            let name = entity_name(head);
            if name.is_empty() {
                return Err(Error::Syntax("entidad sin nombre".into()));
            }
            open = Some(entity(&mut entities, &name));
            continue;
        }
        let word = line.split_whitespace().next().unwrap_or("");
        if matches!(word, "title" | "accTitle" | "accDescr" | "direction" | "classDef" | "style") {
            continue;
        }
        relation(line, &mut entities, &mut rels)?;
    }
    if entities.is_empty() {
        return Err(Error::Syntax("el diagrama entidad-relación no tiene entidades".into()));
    }
    let mut sizes = Vec::with_capacity(entities.len());
    for e in &mut entities {
        sizes.push(e.measure(m));
    }
    let mut links = Vec::with_capacity(rels.len());
    for r in &rels {
        let mut link = Link::new(r.from, r.to).with_label(&r.label);
        link.dashed = r.dashed;
        links.push(link);
    }
    let lay = common::place(&sizes, &links, Dir::Down);
    let mut d = Draw::new();
    for r in &rels {
        let (a, b) = (lay.centers[r.from], lay.centers[r.to]);
        let pa = common::border(a, sizes[r.from], b);
        let pb = common::border(b, sizes[r.to], a);
        let pts = vec![pa, pb];
        d.line(pts.clone(), r.dashed, 1.5);
        common::mid_label(&mut d, m, &pts, &r.label);
        let inward = |from: (f32, f32), to: (f32, f32)| (to.0 - from.0, to.1 - from.1);
        let da = inward(pa, pb);
        let db = inward(pb, pa);
        crowd_foot(&mut d, pa, da, &r.left);
        crowd_foot(&mut d, pb, db, &r.right);
    }
    for (i, e) in entities.iter().enumerate() {
        e.paint(&mut d, m, lay.centers[i]);
    }
    Ok(d.finish(16.0))
}

fn entity(entities: &mut Vec<Entity>, name: &str) -> usize {
    match entities.iter().position(|e| e.name == name) {
        Some(i) => i,
        None => {
            entities.push(Entity { name: name.to_string(), attrs: Vec::new(), size: (0.0, 0.0) });
            entities.len() - 1
        }
    }
}

/// The marks mermaid writes at each end of a relation, drawn from the box outwards: a bar for
/// `|`, a circle for `o` and a crow's foot for `{` or `}`.
fn crowd_foot(d: &mut Draw, at: (f32, f32), dir: (f32, f32), card: &str) {
    let len = (dir.0 * dir.0 + dir.1 * dir.1).sqrt();
    if len < 1e-3 || card.is_empty() {
        return;
    }
    let (ux, uy) = (dir.0 / len, dir.1 / len);
    let (px, py) = (-uy, ux);
    let count = card.chars().count();
    for (i, c) in card.chars().enumerate() {
        let along = 12.0 * (count - i) as f32;
        let (x, y) = (at.0 + ux * along, at.1 + uy * along);
        match c {
            '|' => {
                d.line(vec![(x - px * 7.0, y - py * 7.0), (x + px * 7.0, y + py * 7.0)], false, 1.3);
            }
            'o' => d.ellipse(x, y, 4.0, 4.0, Fill::None),
            '{' | '}' => {
                for k in [-1.0f32, 0.0, 1.0] {
                    let (ex, ey) = if k == 0.0 {
                        (x - ux * 12.0, y - uy * 12.0)
                    } else {
                        (x - ux * 12.0 + px * 8.0 * k, y - uy * 12.0 + py * 8.0 * k)
                    };
                    d.line(vec![(x, y), (ex, ey)], false, 1.2);
                }
            }
            _ => {}
        }
    }
}

/// A relation: `CUSTOMER ||--o{ ORDER : places`.
fn relation(line: &str, entities: &mut Vec<Entity>, rels: &mut Vec<Rel>) -> Result<(), Error> {
    let (head, label) = match line.split_once(':') {
        Some((h, l)) => (h.trim(), common::clean(l)),
        None => (line.trim(), String::new()),
    };
    let mut it = head.split_whitespace();
    let (Some(a), Some(op), Some(b)) = (it.next(), it.next(), it.next()) else {
        return Err(Error::Syntax(format!("relación incompleta: `{line}`")));
    };
    let (left, right, dashed) = if let Some(i) = op.find("--") {
        (&op[..i], &op[i + 2..], false)
    } else if let Some(i) = op.find("..") {
        (&op[..i], &op[i + 2..], true)
    } else {
        return Err(Error::Syntax(format!("no entiendo la relación `{op}`")));
    };
    if !left.chars().all(|c| matches!(c, '|' | 'o' | '{' | '}'))
        || !right.chars().all(|c| matches!(c, '|' | 'o' | '{' | '}'))
    {
        return Err(Error::Syntax(format!("cardinalidad desconocida en `{op}`")));
    }
    let from = entity(entities, &entity_name(a));
    let to = entity(entities, &entity_name(b));
    rels.push(Rel { from, to, left: left.to_string(), right: right.to_string(), label, dashed });
    Ok(())
}
