//! `pie`: a disc cut into slices, with a legend that names each one and its share.

use crate::common::{self, Draw};
use crate::{Diagram, Error, Fill, Measure};

/// How many shades the slices cycle through.
const TONES: u8 = 6;

struct Slice {
    label: String,
    value: f32,
}

pub fn render(header: &str, lines: &[&str], m: &mut dyn Measure) -> Result<Diagram, Error> {
    let show_data = header.split_whitespace().any(|w| w == "showData");
    let mut title = String::new();
    let mut slices: Vec<Slice> = Vec::new();
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
        let Some((label, value)) = line.rsplit_once(':') else {
            return Err(Error::Syntax(format!("no entiendo el valor `{line}`")));
        };
        let label = common::clean(label.trim());
        let Ok(value) = value.trim().parse::<f32>() else {
            return Err(Error::Syntax(format!("`{}` no es un número", value.trim())));
        };
        if value < 0.0 {
            return Err(Error::Syntax("los valores de un gráfico de torta no pueden ser negativos".into()));
        }
        slices.push(Slice { label, value });
    }
    if slices.is_empty() {
        return Err(Error::Syntax("el gráfico de torta no tiene valores".into()));
    }
    let total: f32 = slices.iter().map(|s| s.value).sum();
    if total <= 0.0 {
        return Err(Error::Syntax("el gráfico de torta suma cero".into()));
    }

    // What the legend says, measured first: the disc sits to its left.
    let entries: Vec<(String, String)> = slices
        .iter()
        .map(|s| {
            let share = s.value / total * 100.0;
            let detail =
                if show_data { format!("{:.2} ({share:.1}%)", s.value) } else { format!("{share:.1}%") };
            (s.label.clone(), detail)
        })
        .collect();
    let name_w = entries.iter().map(|(l, _)| m.width(l, 12.0, false)).fold(0.0f32, f32::max);
    let detail_w = entries.iter().map(|(_, d)| m.width(d, 11.0, false)).fold(0.0f32, f32::max);
    let radius = 78.0f32;
    let disc = radius * 2.0 + 8.0;
    let legend_right = disc + 34.0 + name_w + detail_w;
    let title_h = if title.is_empty() { 0.0 } else { 26.0 };
    let legend_h = entries.len() as f32 * 22.0;
    let height = title_h + disc.max(legend_h);

    let mut d = Draw::new();
    let cx = disc / 2.0;
    let cy = title_h + (height - title_h) / 2.0;
    let mut from = -std::f32::consts::FRAC_PI_2;
    for (i, s) in slices.iter().enumerate() {
        let sweep = s.value / total * std::f32::consts::TAU;
        let to = from + sweep;
        d.sector(cx, cy, radius, from, to, Fill::Tone((i as u8) % TONES));
        from = to;
    }
    // The edges between the slices, so that two shades that fell close still read as two.
    let mut edge = -std::f32::consts::FRAC_PI_2;
    for s in &slices {
        let (x, y) = (cx + radius * edge.cos(), cy + radius * edge.sin());
        d.line(vec![(cx, cy), (x, y)], false, 1.0);
        edge += s.value / total * std::f32::consts::TAU;
    }
    d.ellipse(cx, cy, radius, radius, Fill::None);

    let lx = disc + 14.0;
    let mut ly = title_h + (height - title_h) / 2.0 - legend_h / 2.0 + 11.0;
    for (i, (name, detail)) in entries.iter().enumerate() {
        d.rect(lx, ly - 6.0, 12.0, 12.0, 3.0, Fill::Tone((i as u8) % TONES));
        d.text_left(m, lx + 20.0, ly, name, false, 12.0);
        d.text_right(m, legend_right, ly, detail, false, 11.0);
        ly += 22.0;
    }
    if !title.is_empty() {
        d.text(m, legend_right / 2.0, 13.0, &title, true, 14.0);
    }
    Ok(d.finish(14.0))
}
