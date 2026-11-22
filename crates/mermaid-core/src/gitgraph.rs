//! `gitGraph`: commits along the branches they were made on, with their merges and tags.

use crate::common::Draw;
use crate::{Diagram, Error, Fill, Measure};

const LANE: f32 = 50.0;
const COL: f32 = 56.0;
const RADIUS: f32 = 8.0;
const TOP: f32 = 28.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Type {
    Normal,
    Highlight,
    Reverse,
}

struct Commit {
    id: String,
    /// Whether the source gave the commit an id, in which case it is written next to it.
    named: bool,
    tag: String,
    lane: usize,
    /// The column it sits in, left to right.
    col: usize,
    kind: Type,
    /// The commits it comes from: the one its branch was on, and the tip of a merged branch.
    parents: Vec<usize>,
    /// A commit picked from another branch, drawn with a dashed line.
    picked: bool,
}

struct Branch {
    name: String,
    lane: usize,
    tip: Option<usize>,
}

pub fn render(lines: &[&str], m: &mut dyn Measure) -> Result<Diagram, Error> {
    let mut commits: Vec<Commit> = Vec::new();
    let mut branches: Vec<Branch> = vec![Branch { name: "main".into(), lane: 0, tip: None }];
    let mut current = 0usize;
    let mut col = 0usize;
    let mut next_lane = 1usize;
    for raw in lines {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let word = line.split_whitespace().next().unwrap_or("");
        let rest = line[word.len()..].trim();
        match word {
            "commit" => {
                let (id, tag, kind) = options(rest);
                let tip = branches[current].tip;
                let lane = branches[current].lane;
                commits.push(Commit {
                    named: id.is_some(),
                    id: id.unwrap_or_else(|| format!("c{}", col + 1)),
                    tag: tag.unwrap_or_default(),
                    lane,
                    col,
                    kind,
                    parents: tip.into_iter().collect(),
                    picked: false,
                });
                branches[current].tip = Some(commits.len() - 1);
                col += 1;
            }
            "branch" => {
                // `branch develop`, and `branch develop order: 1` in some sources.
                let name = rest.split_whitespace().next().unwrap_or("").to_string();
                let name = if name.is_empty() { format!("rama{}", branches.len()) } else { name };
                let tip = branches[current].tip;
                branches.push(Branch { name, lane: next_lane, tip });
                next_lane += 1;
                current = branches.len() - 1;
            }
            "checkout" | "switch" => {
                let name = rest.split_whitespace().next().unwrap_or("");
                match branches.iter().position(|b| b.name == name) {
                    Some(i) => current = i,
                    None => {
                        let tip = branches[current].tip;
                        branches.push(Branch { name: name.to_string(), lane: next_lane, tip });
                        next_lane += 1;
                        current = branches.len() - 1;
                    }
                }
            }
            "merge" => {
                let target = rest.split_whitespace().next().unwrap_or("");
                let (id, tag, kind) = options(&rest[target.len()..]);
                let mut parents = Vec::new();
                if let Some(t) = branches[current].tip {
                    parents.push(t);
                }
                if let Some(i) = branches.iter().position(|b| b.name == target) {
                    if let Some(t) = branches[i].tip {
                        parents.push(t);
                    }
                }
                let lane = branches[current].lane;
                commits.push(Commit {
                    id: id.unwrap_or_else(|| format!("merge{}", col + 1)),
                    named: false,
                    tag: tag.unwrap_or_default(),
                    lane,
                    col,
                    kind,
                    parents,
                    picked: false,
                });
                branches[current].tip = Some(commits.len() - 1);
                col += 1;
            }
            "cherry-pick" => {
                let (id, tag, kind) = options(rest);
                let parent = id.as_deref().and_then(|id| commits.iter().position(|c| c.id == id));
                let mut parents: Vec<usize> = parent.into_iter().collect();
                if let Some(t) = branches[current].tip {
                    parents.push(t);
                }
                let lane = branches[current].lane;
                commits.push(Commit {
                    id: id.unwrap_or_else(|| format!("pick{}", col + 1)),
                    named: true,
                    tag: tag.unwrap_or_default(),
                    lane,
                    col,
                    kind,
                    parents,
                    picked: true,
                });
                branches[current].tip = Some(commits.len() - 1);
                col += 1;
            }
            "accTitle" | "accDescr" | "title" => {}
            other => {
                return Err(Error::Syntax(format!("no entiendo la instrucción `{other}`")));
            }
        }
    }
    if commits.is_empty() {
        return Err(Error::Syntax("el gráfico de git no tiene commits".into()));
    }
    Ok(paint(&commits, &branches, m))
}

/// The `key: value` pairs a `commit`, a `branch` or a `merge` can carry, which mermaid writes
/// one after the other, with or without commas: `commit id: "a1" tag: "v1" type: HIGHLIGHT`.
fn options(rest: &str) -> (Option<String>, Option<String>, Type) {
    let mut id = None;
    let mut tag = None;
    let mut kind = Type::Normal;
    let mut text = rest.trim();
    while let Some((key, value, tail)) = take_option(text) {
        match key {
            "id" => id = Some(value),
            "tag" => tag = Some(value),
            "type" => {
                kind = match value.to_ascii_uppercase().as_str() {
                    "HIGHLIGHT" => Type::Highlight,
                    "REVERSE" => Type::Reverse,
                    _ => Type::Normal,
                }
            }
            _ => {}
        }
        text = tail;
    }
    (id, tag, kind)
}

/// One `key: value` at the front of `text`, and what is left after it. A value in quotes ends at
/// its closing quote; one without them ends at the next comma.
fn take_option(text: &str) -> Option<(&str, String, &str)> {
    let t = text.trim_start_matches([',', ' ']);
    let (key, after) = t.split_once(':')?;
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let after = after.trim_start();
    let (value, tail) = if let Some(rest) = after.strip_prefix('"') {
        match rest.find('"') {
            Some(end) => (&rest[..end], &rest[end + 1..]),
            None => (rest, ""),
        }
    } else {
        match after.find(',') {
            Some(end) => (&after[..end], &after[end..]),
            None => (after, ""),
        }
    };
    Some((key, value.trim().to_string(), tail))
}

fn at(c: &Commit) -> (f32, f32) {
    (c.col as f32 * COL, TOP + c.lane as f32 * LANE)
}

fn paint(commits: &[Commit], branches: &[Branch], m: &mut dyn Measure) -> Diagram {
    let mut d = Draw::new();
    // The name of each branch, on the left of the lane it runs along.
    for b in branches {
        if let Some(first) = commits.iter().position(|c| c.lane == b.lane) {
            let x = commits[first].col as f32 * COL;
            d.text_right(m, x - 18.0, TOP + b.lane as f32 * LANE, &b.name, true, 12.0);
        }
    }
    // The line of each branch, and the elbow of every merge or pick.
    for (i, c) in commits.iter().enumerate() {
        let (x, y) = at(c);
        for &p in &c.parents {
            if p == i || p >= commits.len() {
                continue;
            }
            let (px, py) = at(&commits[p]);
            let pts = if (py - y).abs() < 0.5 {
                vec![(px + RADIUS, py), (x - RADIUS, y)]
            } else {
                let mid = (px + x) / 2.0;
                vec![(px + RADIUS, py), (mid, py), (mid, y), (x - RADIUS, y)]
            };
            d.line(pts, c.picked, 2.0);
        }
    }
    for (i, c) in commits.iter().enumerate() {
        let (x, y) = at(c);
        if i + 1 < commits.len() && commits[i + 1].lane == c.lane && commits[i + 1].col == c.col + 1 {
            d.line(vec![(x + RADIUS, y), (x + COL - RADIUS, y)], false, 2.0);
        }
        let fill = if c.kind == Type::Highlight { Fill::Accent } else { Fill::Node };
        d.ellipse(x, y, RADIUS, RADIUS, fill);
        if c.kind == Type::Reverse {
            d.ellipse(x, y, RADIUS - 3.0, RADIUS - 3.0, Fill::Background);
        }
        if c.named {
            d.text(m, x, y - 17.0, &c.id, false, 10.5);
        }
        if !c.tag.is_empty() {
            let w = m.width(&c.tag, 10.0, true) + 14.0;
            d.rect(x - w / 2.0, y + 12.0, w, 16.0, 8.0, Fill::Accent);
            d.text(m, x, y + 20.0, &c.tag, true, 10.0);
        }
    }
    d.finish(14.0)
}
