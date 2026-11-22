//! `journey`: the steps of a task laid along a score, section by section, with who does them.

use crate::common::{self, Draw};
use crate::{Diagram, Error, Fill, Measure};

struct Task {
    label: String,
    score: usize,
    actors: Vec<String>,
}

struct Section {
    name: String,
    tasks: Vec<Task>,
}

/// The best score a step can have.
const TOP: usize = 5;
const STEP: f32 = 38.0;
const COL: f32 = 122.0;

pub fn render(lines: &[&str], m: &mut dyn Measure) -> Result<Diagram, Error> {
    let mut title = String::new();
    let mut sections: Vec<Section> = Vec::new();
    for raw in lines {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("title") {
            title = common::clean(rest);
            continue;
        }
        if matches!(line.split_whitespace().next().unwrap_or(""), "accTitle" | "accDescr") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("section") {
            sections.push(Section { name: common::clean(rest), tasks: Vec::new() });
            continue;
        }
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() < 2 {
            return Err(Error::Syntax(format!("no entiendo el paso `{line}`")));
        }
        let label = common::clean(parts[0]);
        let score: usize = parts[1].trim().parse().unwrap_or(3).clamp(1, TOP);
        let actors = parts
            .get(2)
            .map_or_else(Vec::new, |a| a.split(',').map(common::clean).filter(|x| !x.is_empty()).collect());
        if sections.is_empty() {
            sections.push(Section { name: String::new(), tasks: Vec::new() });
        }
        sections.last_mut().expect("section").tasks.push(Task { label, score, actors });
    }
    if sections.iter().all(|s| s.tasks.is_empty()) {
        return Err(Error::Syntax("el viaje no tiene pasos".into()));
    }

    let title_h = if title.is_empty() { 0.0 } else { 26.0 };
    let band_h = 22.0;
    let grid_top = title_h + band_h + 12.0;
    let grid_h = (TOP as f32 - 1.0) * STEP;
    let labels_y = grid_top + grid_h + 20.0;
    let mut d = Draw::new();
    if !title.is_empty() {
        d.text_left(m, 0.0, title_h / 2.0, &title, true, 14.0);
    }

    let mut x = 30.0;
    let mut dots: Vec<(f32, f32, usize)> = Vec::new();
    for section in &sections {
        let start = x;
        for task in &section.tasks {
            let cx = x + COL / 2.0;
            let y = grid_top + (TOP - task.score) as f32 * STEP;
            dots.push((cx, y, task.score));
            let stars = format!("{}{}", "★".repeat(task.score), "☆".repeat(TOP - task.score));
            d.text(m, cx, y - 14.0, &stars, false, 10.0);
            d.text(m, cx, labels_y, &task.label, true, 11.5);
            if !task.actors.is_empty() {
                let who = task.actors.join(", ");
                for (i, line) in common::wrap(m, &who, 10.0, COL - 12.0).iter().enumerate() {
                    d.text(m, cx, labels_y + 15.0 + i as f32 * 12.0, line, false, 10.0);
                }
            }
            x += COL;
        }
        if !section.name.is_empty() {
            d.band(start - 6.0, title_h + 4.0, (x - start).max(50.0), band_h, 5.0, Fill::Accent);
            d.text(m, (start + x) / 2.0, title_h + 4.0 + band_h / 2.0, &section.name, true, 11.5);
        }
        d.line(vec![(x, grid_top - 10.0), (x, labels_y + 30.0)], true, 0.6);
    }
    let width = x;
    // A faint line per score, with the score at the left of it.
    for level in 1..=TOP {
        let y = grid_top + (TOP - level) as f32 * STEP;
        d.line(vec![(26.0, y), (width, y)], false, 0.5);
        d.text_right(m, 20.0, y, &level.to_string(), false, 10.0);
    }
    for pair in dots.windows(2) {
        d.line(vec![(pair[0].0, pair[0].1), (pair[1].0, pair[1].1)], false, 2.0);
    }
    for (cx, y, score) in dots {
        d.ellipse(cx, y, 5.0, 5.0, Fill::Node);
        if score == TOP {
            d.ellipse(cx, y, 2.0, 2.0, Fill::Background);
        }
    }
    Ok(d.finish(14.0))
}
