//! `timeline`: periods along an axis, each with the events that happened in it, in sections.

use crate::common::{self, Draw};
use crate::{Diagram, Error, Fill, Measure};

struct Period {
    label: String,
    events: Vec<String>,
    /// The columns its section covers, for the band drawn behind it.
    w: f32,
}

struct Section {
    name: String,
    periods: Vec<Period>,
}

const COL_GAP: f32 = 18.0;
const LABEL: f32 = 12.5;

pub fn render(lines: &[&str], m: &mut dyn Measure) -> Result<Diagram, Error> {
    let mut title = String::new();
    let mut sections: Vec<Section> = Vec::new();
    for raw in lines {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let word = line.split_whitespace().next().unwrap_or("");
        if let Some(rest) = line.strip_prefix("title") {
            title = common::clean(rest);
            continue;
        }
        if matches!(word, "accTitle" | "accDescr") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("section") {
            sections.push(Section { name: common::clean(rest), periods: Vec::new() });
            continue;
        }
        let (label, rest) = match line.split_once(':') {
            Some((l, r)) => (common::clean(l), r),
            None => match sections.last_mut().and_then(|s| s.periods.last_mut()) {
                // A line of its own after a period continues that period.
                Some(p) => {
                    p.events.push(common::clean(line));
                    continue;
                }
                None => {
                    return Err(Error::Syntax(format!("no entiendo la línea `{line}`")));
                }
            },
        };
        let events: Vec<String> = rest.split(':').map(common::clean).filter(|e| !e.is_empty()).collect();
        if sections.is_empty() {
            sections.push(Section { name: String::new(), periods: Vec::new() });
        }
        sections.last_mut().expect("section").periods.push(Period { label, events, w: 0.0 });
    }
    let periods = sections.iter().map(|s| s.periods.len()).sum::<usize>();
    if periods == 0 {
        return Err(Error::Syntax("la línea de tiempo no tiene períodos".into()));
    }
    for section in &mut sections {
        for p in &mut section.periods {
            let label_w = m.width(&p.label, LABEL + 1.0, true);
            let events_w = p.events.iter().map(|e| m.width(e, 11.0, false)).fold(0.0f32, f32::max);
            p.w = label_w.max(events_w + 22.0).max(88.0) + COL_GAP;
        }
    }

    let title_h = if title.is_empty() { 0.0 } else { 26.0 };
    let band_y = title_h + 4.0;
    let label_y = band_y + 34.0;
    let axis_y = label_y + 18.0;
    let events_y = axis_y + 16.0;

    let mut d = Draw::new();
    if !title.is_empty() {
        d.text_left(m, 0.0, title_h / 2.0, &title, true, 14.0);
    }
    let mut x = 0.0;
    for section in &sections {
        let start = x;
        let width: f32 = section.periods.iter().map(|p| p.w).sum();
        if !section.name.is_empty() {
            d.band(start, band_y, width.max(60.0), 22.0, 5.0, Fill::Accent);
            d.text_left(m, start + 8.0, band_y + 11.0, &section.name, true, 11.5);
        }
        for p in &section.periods {
            let cx = x + p.w / 2.0;
            d.line(vec![(cx, axis_y - 6.0), (cx, axis_y + 6.0)], false, 1.2);
            d.ellipse(cx, axis_y, 4.0, 4.0, Fill::Node);
            d.text(m, cx, label_y, &p.label, true, LABEL + 1.0);
            let mut ey = events_y;
            for e in &p.events {
                let w = p.w - COL_GAP;
                d.rect(x + COL_GAP / 2.0, ey, w, 24.0, 5.0, Fill::Node);
                d.text(m, cx, ey + 12.0, e, false, 11.0);
                ey += 30.0;
            }
            x += p.w;
        }
        if section.periods.is_empty() {
            x += 60.0;
        }
    }
    d.line(vec![(0.0, axis_y), (x, axis_y)], false, 1.4);
    Ok(d.finish(14.0))
}
