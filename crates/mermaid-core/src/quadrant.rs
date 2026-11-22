//! `quadrantChart`: a square split in four, with the points placed by two scores.

use crate::common::{self, Draw};
use crate::{Diagram, Error, Fill, Measure};

/// The side of the square the points are placed in.
const SIDE: f32 = 250.0;

struct Point {
    label: String,
    x: f32,
    y: f32,
}

pub fn render(lines: &[&str], m: &mut dyn Measure) -> Result<Diagram, Error> {
    let mut title = String::new();
    let mut quadrant = [String::new(), String::new(), String::new(), String::new()];
    let mut x_axis = (String::new(), String::new());
    let mut y_axis = (String::new(), String::new());
    let mut points: Vec<Point> = Vec::new();
    for raw in lines {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("title") {
            title = common::clean(rest);
            continue;
        }
        if let Some((head, rest)) = line.split_once(char::is_whitespace) {
            let head_trim = head.trim_end_matches(':');
            if let Some(n) = head_trim.strip_prefix("quadrant-").and_then(|n| n.parse::<usize>().ok()) {
                if (1..=4).contains(&n) {
                    quadrant[n - 1] = common::clean(rest);
                }
                continue;
            }
            if head_trim == "x-axis" || head_trim == "y-axis" {
                let (low, high) = match rest.split_once("-->") {
                    Some((a, b)) => (common::clean(a), common::clean(b)),
                    None => (common::clean(rest), String::new()),
                };
                if head_trim == "x-axis" {
                    x_axis = (low, high);
                } else {
                    y_axis = (low, high);
                }
                continue;
            }
        }
        if matches!(
            line.split_whitespace().next().unwrap_or(""),
            "accTitle" | "accDescr" | "classDef" | "style"
        ) {
            continue;
        }
        let Some((label, rest)) = line.split_once(':') else {
            return Err(Error::Syntax(format!("no entiendo el punto `{line}`")));
        };
        let (Some(a), Some(b)) = (rest.find('['), rest.rfind(']')) else {
            return Err(Error::Syntax(format!("al punto `{label}` le faltan las coordenadas")));
        };
        let numbers: Vec<f32> =
            rest[a + 1..b].split(',').filter_map(|n| n.trim().parse::<f32>().ok()).collect();
        let [x, y] = numbers[..] else {
            return Err(Error::Syntax(format!("las coordenadas de `{label}` no son dos números")));
        };
        if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
            return Err(Error::Syntax(format!("las coordenadas de `{label}` van de 0 a 1")));
        }
        points.push(Point { label: common::clean(label), x, y });
    }
    if points.is_empty() {
        return Err(Error::Syntax("el gráfico de cuadrantes no tiene puntos".into()));
    }
    Ok(paint(&title, &quadrant, &x_axis, &y_axis, &points, m))
}

fn paint(
    title: &str,
    quadrant: &[String; 4],
    x_axis: &(String, String),
    y_axis: &(String, String),
    points: &[Point],
    m: &mut dyn Measure,
) -> Diagram {
    let mut d = Draw::new();
    let title_h = if title.is_empty() { 0.0 } else { 26.0 };
    let top = title_h;
    let bottom = top + SIDE;
    if !title.is_empty() {
        d.text(m, SIDE / 2.0, title_h / 2.0 + 2.0, title, true, 14.0);
    }
    d.rect(0.0, top, SIDE, SIDE, 6.0, Fill::Accent);
    d.line(vec![(SIDE / 2.0, top), (SIDE / 2.0, bottom)], true, 1.0);
    d.line(vec![(0.0, top + SIDE / 2.0), (SIDE, top + SIDE / 2.0)], true, 1.0);
    // `quadrant-1` is the top right one, `quadrant-4` the bottom right, as mermaid has it.
    let corners = [(SIDE, top), (0.0, top), (0.0, top + SIDE / 2.0), (SIDE / 2.0, top + SIDE / 2.0)];
    for (i, text) in quadrant.iter().enumerate() {
        if text.is_empty() {
            continue;
        }
        let (x, y) = corners[i];
        let half = m.width(text, 11.0, true) / 2.0;
        let inner_x = if i % 2 == 0 { x - 10.0 - half } else { x + 10.0 + half };
        d.text(m, inner_x, y + 13.0, text, true, 11.0);
    }
    for p in points {
        let x = p.x * SIDE;
        let y = top + (1.0 - p.y) * SIDE;
        d.ellipse(x, y, 5.0, 5.0, Fill::Node);
        d.text_left(m, x + 9.0, y, &p.label, false, 10.5);
    }
    if !x_axis.0.is_empty() {
        d.text_left(m, 0.0, bottom + 13.0, &x_axis.0, false, 10.5);
    }
    if !x_axis.1.is_empty() {
        d.text_right(m, SIDE, bottom + 13.0, &x_axis.1, false, 10.5);
    }
    if !y_axis.1.is_empty() {
        d.text_right(m, -10.0, top + 6.0, &y_axis.1, false, 10.5);
    }
    if !y_axis.0.is_empty() {
        d.text_right(m, -10.0, bottom - 6.0, &y_axis.0, false, 10.5);
    }
    d.finish(14.0)
}
