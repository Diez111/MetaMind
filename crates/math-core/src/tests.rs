use super::*;

fn lay(src: &str, display: bool) -> Layout {
    render(src, 16.0, display, &mut Approx)
}

fn texts(l: &Layout) -> Vec<&str> {
    l.glyphs.iter().map(|g| g.text.as_str()).collect()
}

#[test]
fn symbols_and_greek() {
    let l = lay(r"\alpha + \beta \leq \infty", false);
    let t = texts(&l);
    for s in ["α", "β", "≤", "∞", "+"] {
        assert!(t.contains(&s), "{t:?}");
    }
}

#[test]
fn scripts_are_smaller_and_shifted() {
    let l = lay("x^2_i", false);
    let x = &l.glyphs[0];
    let sup = l.glyphs.iter().find(|g| g.text == "2").unwrap();
    let sub = l.glyphs.iter().find(|g| g.text == "i").unwrap();
    assert!(sup.size < x.size && sub.size < x.size);
    assert!(sup.y < x.y && sub.y > x.y);
    assert!(sup.x > x.x && (sup.x - sub.x).abs() < 0.01);
}

#[test]
fn fraction_has_rule_and_stacks() {
    let l = lay(r"\frac{a}{b}", true);
    assert_eq!(l.segs.len(), 1);
    let rule_y = l.segs[0].y0;
    let a = l.glyphs.iter().find(|g| g.text == "a").unwrap();
    let b = l.glyphs.iter().find(|g| g.text == "b").unwrap();
    assert!(a.y < rule_y && b.y > rule_y);
    assert!(l.height() > 16.0 * 1.5);
}

#[test]
fn sqrt_draws_radical() {
    let l = lay(r"\sqrt{x+1}", false);
    assert!(l.segs.len() >= 4);
    assert!(texts(&l).contains(&"x"));
}

#[test]
fn limits_in_display_are_stacked() {
    let l = lay(r"\sum_{i=0}^{n} i", true);
    let sum = l.glyphs.iter().find(|g| g.text == "∑").unwrap();
    let n = l.glyphs.iter().find(|g| g.text == "n").unwrap();
    let i0 = l.glyphs.iter().find(|g| g.text == "0").unwrap();
    assert!(n.y < sum.y && i0.y > sum.y);
    assert!(sum.size > 16.0);
    let inline = lay(r"\sum_{i=0}^{n} i", false);
    let sum = inline.glyphs.iter().find(|g| g.text == "∑").unwrap();
    let n = inline.glyphs.iter().find(|g| g.text == "n").unwrap();
    assert!(n.x > sum.x);
}

#[test]
fn integral_keeps_side_limits() {
    let l = lay(r"\int_0^1 f(x)\,dx", true);
    let int = l.glyphs.iter().find(|g| g.text == "∫").unwrap();
    let one = l.glyphs.iter().find(|g| g.text == "1").unwrap();
    assert!(one.x > int.x);
}

#[test]
fn left_right_scale_with_content() {
    let small = lay(r"\left( x \right)", false);
    assert!(small.segs.is_empty());
    let big = lay(r"\left( \frac{a}{b} \right)", true);
    assert!(big.segs.len() > 5);
}

#[test]
fn text_and_fonts() {
    let l = lay(r"\text{if } x \mathbf{v} \mathrm{d} \mathbb{R}", false);
    let t = texts(&l);
    assert!(t.contains(&"if "));
    assert!(t.contains(&"ℝ"));
    assert!(l.glyphs.iter().any(|g| g.face == Face::Bold));
}

#[test]
fn matrices_and_aligned() {
    let m = lay(r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}", true);
    let (a, b, c) = (
        m.glyphs.iter().find(|g| g.text == "a").unwrap(),
        m.glyphs.iter().find(|g| g.text == "b").unwrap(),
        m.glyphs.iter().find(|g| g.text == "c").unwrap(),
    );
    assert!(b.x > a.x && c.y > a.y && (c.x - a.x).abs() < 0.01);
    assert!(!m.segs.is_empty());
    let al = lay(r"\begin{aligned} x &= 1 \\ yy &= 2 \end{aligned}", true);
    let eq: Vec<_> = al.glyphs.iter().filter(|g| g.text == "=").collect();
    assert_eq!(eq.len(), 2);
    assert!((eq[0].x - eq[1].x).abs() < 0.01);
}

#[test]
fn unknown_commands_fall_back_to_source() {
    let l = lay(r"\foo{x} + \begin{weird} a \end{weird}", false);
    let t = texts(&l);
    assert!(t.contains(&"\\foo"), "{t:?}");
    assert!(t.iter().any(|s| s.contains("\\begin{weird}")), "{t:?}");
}

#[test]
fn spacing_commands_and_garbage_do_not_panic() {
    let wide = lay(r"a\quad b", false).width;
    let narrow = lay("ab", false).width;
    assert!(wide > narrow + 10.0);
    for s in [
        "",
        "{",
        "}",
        "^",
        "_",
        r"\frac",
        r"\frac{a}",
        r"\left(",
        r"\begin{pmatrix}",
        "x^^2",
        r"\sqrt[3]",
        "&",
        r"\\",
        "{{{{{{{{{{",
        r"\underbrace{}",
        r"\underbrace{}_x",
        r"\overbrace{}^",
        r"\overset{}{}",
        r"\textcolor{}{x}",
        r"\color{}",
        r"\textcolor{#zzz}{x}",
    ] {
        let l = lay(s, true);
        assert!(l.width.is_finite() && l.height().is_finite(), "{s}");
    }
}

#[test]
fn deep_nesting_is_bounded() {
    let s = "{".repeat(5000) + "x" + &"}".repeat(5000);
    let l = lay(&s, false);
    assert!(l.width.is_finite());
}

fn glyph_x(l: &Layout, t: &str) -> f32 {
    l.glyphs.iter().find(|g| g.text == t).unwrap().x
}

fn colour(l: &Layout, t: &str) -> Option<Color> {
    l.glyphs.iter().find(|g| g.text == t).unwrap().color
}

fn top_of(l: &Layout) -> f32 {
    l.segs.iter().map(|s| s.y0.min(s.y1)).fold(f32::MAX, f32::min)
}

fn bottom_of(l: &Layout) -> f32 {
    l.segs.iter().map(|s| s.y0.max(s.y1)).fold(f32::MIN, f32::max)
}

#[test]
fn underbrace_puts_the_annotation_under_the_brace() {
    let l = lay(r"\underbrace{a+b}_{=3}", true);
    let plus = l.glyphs.iter().find(|g| g.text == "+").unwrap();
    let eq = l.glyphs.iter().find(|g| g.text == "=").unwrap();
    let three = l.glyphs.iter().find(|g| g.text == "3").unwrap();
    assert_eq!(l.segs.len(), 6, "one flat run and two curls");
    assert!(top_of(&l) > plus.y, "the brace hangs below the body");
    assert!(eq.y > bottom_of(&l), "the annotation hangs below the brace");
    assert!(eq.size < plus.size && (three.size - eq.size).abs() < 0.01);
    assert!(l.height() > lay("a+b", true).height() + 0.4 * 16.0);
}

#[test]
fn underbrace_is_as_wide_as_its_widest_part() {
    let label = lay(r"\underbrace{a}_{\text{a longer annotation}}", true);
    let empty = lay(r"\underbrace{a}_{}", true);
    assert!(label.width > empty.width + 20.0);
    // The body stays centred over the wide annotation.
    let a = glyph_x(&label, "a");
    assert!(a > 6.0 + 3.0, "body pushed in from the left edge: {a}");
}

#[test]
fn overbrace_puts_the_annotation_on_top() {
    let l = lay(r"\overbrace{a+b}^{n}", true);
    let plus = l.glyphs.iter().find(|g| g.text == "+").unwrap();
    let n = l.glyphs.iter().find(|g| g.text == "n").unwrap();
    assert_eq!(l.segs.len(), 6);
    assert!(bottom_of(&l) < plus.y, "the brace sits above the body");
    assert!(n.y < top_of(&l), "the annotation sits above the brace");
}

#[test]
fn overset_and_underset_centre_the_small_argument() {
    let o = lay(r"\overset{!}{=}", true);
    let bang = o.glyphs.iter().find(|g| g.text == "!").unwrap();
    let eq = o.glyphs.iter().find(|g| g.text == "=").unwrap();
    assert!(bang.y < eq.y && bang.size < eq.size);
    assert!(o.segs.is_empty());
    let mut m = Approx;
    let (wb, we) = (m.width("!", bang.size, bang.face), m.width("=", eq.size, eq.face));
    assert!((bang.x + wb / 2.0 - (eq.x + we / 2.0)).abs() < 0.01, "not centred");

    let u = lay(r"\underset{n}{\lim}", true);
    let lim = u.glyphs.iter().find(|g| g.text == "lim").unwrap();
    let n = u.glyphs.iter().find(|g| g.text == "n").unwrap();
    assert!(n.y > lim.y && n.size < lim.size);
}

#[test]
fn textcolor_sets_a_colour_per_run() {
    let l = lay(r"\textcolor{red}{x}+\color{#0a0}y+z", false);
    assert_eq!(colour(&l, "x"), Some(Color::rgb(255, 0, 0)));
    assert_eq!(colour(&l, "y"), Some(Color::rgb(0, 0xaa, 0)));
    assert_eq!(colour(&l, "z"), Some(Color::rgb(0, 0xaa, 0)), "\\color colours the rest of the group");
    assert_eq!(colour(&l, "+"), None);
}

#[test]
fn colour_reaches_rules_and_stays_inside_its_group() {
    let l = lay(r"\textcolor{blue}{\frac{a}{b}}", true);
    assert!(l.segs.iter().all(|s| s.color == Some(Color::rgb(0, 0, 255))));
    let l = lay(r"{\textcolor{red}x}+y", false);
    assert_eq!(colour(&l, "x"), Some(Color::rgb(255, 0, 0)));
    assert_eq!(colour(&l, "y"), None);
    let l = lay(r"\textcolor{chartreuse}{x}", false);
    assert_eq!(colour(&l, "x"), None, "unknown colour names are ignored, not fatal");
}

#[test]
fn colour_names_and_hex_literals() {
    assert_eq!(Color::parse("red"), Some(Color::rgb(255, 0, 0)));
    assert_eq!(Color::parse("DarkGray"), Some(Color::rgb(0xa9, 0xa9, 0xa9)));
    assert_eq!(Color::parse("#f00"), Some(Color::rgb(255, 0, 0)));
    assert_eq!(Color::parse("#1a2b3c"), Some(Color::rgb(0x1a, 0x2b, 0x3c)));
    assert_eq!(Color::parse("#12345"), None);
    assert_eq!(Color::parse("chartreuse"), None);
    assert_eq!(Color::parse(""), None);
}

#[test]
fn metrics_set_the_padding_and_the_gap() {
    let tight = render_full("x+y", 16.0, false, &Metrics::ZERO, &mut Approx).layout;
    let padded = render("x+y", 16.0, false, &mut Approx);
    assert!((padded.width - tight.width - 2.0 * Metrics::default().padding_x).abs() < 0.001);
    assert!((padded.height() - tight.height() - 2.0 * Metrics::default().padding_y).abs() < 0.001);
    let spaced = Metrics { run_gap: 4.0, ..Metrics::ZERO };
    let wide = render_full("x+y", 16.0, false, &spaced, &mut Approx).layout;
    assert!((wide.width - tight.width - 8.0).abs() < 0.001, "two gaps between three items");
    let zero = render_full("x+y", 16.0, false, &Metrics::ZERO, &mut Approx).layout;
    assert_eq!(zero.glyphs[0].x, tight.glyphs[0].x);
}

#[test]
fn unknown_commands_are_reported_with_their_byte_offset() {
    let src = r"α+\foo{x}";
    let p = parse_full(src);
    assert_eq!(p.diagnostics, vec![Diagnostic { offset: 3, command: r"\foo".to_string() }]);
    assert!(p.nodes.iter().any(|n| matches!(n, Node::Unknown(t) if t == r"\foo")));
    let r = render_full(src, 16.0, false, &Metrics::default(), &mut Approx);
    assert_eq!(r.diagnostics, p.diagnostics);
    assert!(r.layout.glyphs.iter().any(|g| g.text == r"\foo"), "still drawn as source text");
    assert!(parse(src).iter().any(|n| matches!(n, Node::Unknown(_))));
}

#[test]
fn unknown_environments_are_reported_once() {
    let p = parse_full(r"\begin{weird} a \end{weird}");
    assert_eq!(p.diagnostics.len(), 1, "{:?}", p.diagnostics);
    assert_eq!(p.diagnostics[0].offset, 0);
    assert_eq!(p.diagnostics[0].command, r"\begin{weird}");
}

#[test]
fn supported_commands_report_nothing() {
    for src in [
        r"\underbrace{a}_{b}",
        r"\overbrace{a}^{b}",
        r"\overset{!}{=}",
        r"\underset{n}{\lim}",
        r"\textcolor{red}{x}+\color{#0a0}y",
        r"\lvert x\rvert + \lVert y\rVert",
        r"\dfrac{1}{2}+\left(\frac{a}{b}\right)",
        r"\sum_{i=0}^{n} i \quad \mathbf{v} \in \mathbb{R}",
        r"\boxed{\frac{1}{2}=0{,}5}",
        r"\boxed{\int_0^1 x\,dx}",
        r"a \xrightarrow[n\to\infty]{} b",
        r"a \xleftarrow[back]{there} b",
        r"a \xrightarrow{_{n\to 0}} b",
        r"x \tag{1}",
        r"x \tag*{A}",
        r"a_n\not\to 0",
        r"\not\equiv",
        r"\not{x}",
        r"x\preccurlyeq y",
        r"a\nmid b",
        r"\blacksquare",
        r"p\veebar q",
        r"\Gamma\vdash\Delta",
        r"a\equiv b\pmod n",
        r"a\bmod b",
        r"\check v",
        r"\texttt{x y}",
        r"\left\{a\middle|b\right\}",
        r"\phantom{abc}",
        r"a\Bigm|b",
        r"a\bigm|b\biggm|c\Biggm|d",
        r"\begin{aligned}a&=1\\\hline b&=2\end{aligned}",
        r"\llbracket x\rrbracket",
        r"\checkmark\longmapsto\rightsquigarrow",
    ] {
        assert!(parse_full(src).diagnostics.is_empty(), "{src}: {:?}", parse_full(src).diagnostics);
    }
}

fn ink_bbox(l: &Layout) -> (f32, f32) {
    let mut m = Approx;
    let mut x0 = f32::MAX;
    let mut x1 = f32::MIN;
    for g in &l.glyphs {
        x0 = x0.min(g.x);
        x1 = x1.max(g.x + m.width(&g.text, g.size, g.face));
    }
    (x0, x1)
}

/// The horizontal shaft of an extensible arrow: the only segment that is flat and runs the width.
fn shaft(l: &Layout) -> &Seg {
    l.segs
        .iter()
        .filter(|s| (s.y0 - s.y1).abs() < 0.001)
        .max_by(|a, b| (a.x1 - a.x0).abs().partial_cmp(&(b.x1 - b.x0).abs()).unwrap())
        .expect("an arrow has a flat shaft")
}

#[test]
fn boxed_draws_a_frame_edge_to_edge_of_the_box() {
    let l = lay(r"\boxed{x}", false);
    assert_eq!(l.segs.len(), 4, "four sides, one segment each");
    let t = l.segs[0].width;
    assert!((t - 0.06 * 16.0).abs() < 0.001, "frame stroke: {t}");
    let left = l.segs.iter().map(|s| s.x0).fold(f32::MAX, f32::min);
    let right = l.segs.iter().map(|s| s.x1).fold(f32::MIN, f32::max);
    let top = top_of(&l);
    let bottom = bottom_of(&l);
    // The stroke is centred on the frame line, so half of it is the outermost ink.
    assert!((left - t / 2.0).abs() < 0.001, "frame flush with the left edge: {left}");
    assert!((right + t / 2.0 - l.width).abs() < 0.001, "frame flush with the right edge: {right}");
    assert!((top - t / 2.0).abs() < 0.001, "frame flush with the top: {top}");
    assert!((bottom + t / 2.0 - l.height()).abs() < 0.001, "frame flush with the bottom: {bottom}");
}

#[test]
fn boxed_uses_the_metrics_padding_for_the_frame() {
    let frame = 0.06 * 16.0;
    let ink = render_full("x", 16.0, false, &Metrics::ZERO, &mut Approx).layout;
    let tight = render_full(r"\boxed{x}", 16.0, false, &Metrics::ZERO, &mut Approx).layout;
    // A boxed root takes the frame breathing room from the metrics, so the outer padding of the
    // formula is not added a second time: the growth is the frame padding plus the stroke.
    assert!((tight.width - ink.width - frame).abs() < 0.001, "no padding, just the stroke");
    let pad = Metrics::default();
    let padded = lay(r"\boxed{x}", false);
    assert!((padded.width - tight.width - 2.0 * pad.padding_x).abs() < 0.001, "frame padding_x");
    // Vertical growth needs a body taller than the 0.7 em floor, or the floor hides the padding.
    let tall = render_full(r"\boxed{\dfrac{x}{y}}", 16.0, true, &Metrics::ZERO, &mut Approx).layout;
    let tall_pad = lay(r"\boxed{\dfrac{x}{y}}", true);
    assert!((tall_pad.height() - tall.height() - 2.0 * pad.padding_y).abs() < 0.001, "frame padding_y");
    let wide = Metrics { padding_x: 20.0, ..Metrics::ZERO };
    let roomy = render_full(r"\boxed{x}", 16.0, false, &wide, &mut Approx).layout;
    assert!((roomy.width - tight.width - 40.0).abs() < 0.001, "the frame follows the knob");
    // The clearance around the body is the frame padding, measured from the ink, not a constant.
    let x = padded.glyphs.iter().find(|g| g.text == "x").unwrap();
    let left = padded.segs.iter().map(|s| s.x0).fold(f32::MAX, f32::min);
    let top = padded.segs.iter().map(|s| s.y0).fold(f32::MAX, f32::min);
    let bottom = padded.segs.iter().map(|s| s.y1).fold(f32::MIN, f32::max);
    assert!((x.x - left - pad.padding_x).abs() < 0.01, "padding to the left of the ink");
    assert!((x.y - 0.46 * 16.0 - top - pad.padding_y).abs() < 0.01, "padding above the ink");
    assert!((bottom - x.y - pad.padding_y).abs() < 0.01, "and below the baseline");
}

#[test]
fn boxed_keeps_the_body_inside_the_frame() {
    let l = lay(r"\boxed{a+b}", false);
    let (x0, x1) = ink_bbox(&l);
    let frame_left = l.segs.iter().map(|s| s.x0).fold(f32::MAX, f32::min);
    let frame_right = l.segs.iter().map(|s| s.x1).fold(f32::MIN, f32::max);
    assert!(x0 > frame_left && x1 < frame_right, "body {x0}..{x1} inside {frame_left}..{frame_right}");
    // A boxed formula in the middle of a line still gets its frame, without moving the line box
    // beyond what the frame itself needs.
    let mid = lay(r"x=\boxed{2}", false);
    assert_eq!(mid.segs.len(), 4);
    assert!(mid.glyphs.iter().any(|g| g.text == "2"));
}

#[test]
fn xrightarrow_is_as_wide_as_its_label_and_centres_it() {
    let pad = 2.0 * Metrics::default().padding_x;
    let empty = lay(r"\xrightarrow{}", true);
    let min = empty.width - pad;
    assert!((min - 1.5 * 16.0).abs() < 0.001, "1.5 em minimum shaft, got {min}");
    // The shaft is the label plus a fixed gap: two labels differ by exactly their ink difference.
    let short = lay(r"\xrightarrow[a\to b]{}", true);
    let long = lay(r"\xrightarrow[a\to b\to c]{}", true);
    let ink = |l: &Layout| {
        let (x0, x1) = ink_bbox(l);
        x1 - x0
    };
    assert!(long.width > empty.width, "a long label stretches the shaft");
    assert!(
        (long.width - short.width - (ink(&long) - ink(&short))).abs() < 0.01,
        "width = label + gap: {} vs {}",
        long.width - short.width,
        ink(&long) - ink(&short)
    );
    let (x0, x1) = ink_bbox(&long);
    let centre = (x0 + x1) / 2.0;
    assert!((centre - long.width / 2.0).abs() < 0.01, "label not centred: {centre} vs {}", long.width / 2.0);
}

#[test]
fn xrightarrow_draws_a_head_at_the_end_and_puts_the_label_above() {
    let l = lay(r"\xrightarrow[\text{under}]{\text{over}}", true);
    let flat = *shaft(&l);
    assert_eq!(l.segs.len(), 3, "the shaft plus the two strokes of the head");
    let pad = Metrics::default().padding_x;
    assert!((flat.x0 - pad).abs() < 0.01 && (flat.x1 - (l.width - pad)).abs() < 0.01, "{flat:?}");
    let over = l.glyphs.iter().find(|g| g.text == "over").unwrap();
    let under = l.glyphs.iter().find(|g| g.text == "under").unwrap();
    assert!(over.y < flat.y0, "the brace label sits above the shaft");
    assert!(under.y > flat.y0, "the bracket label sits below the shaft");
    assert!(over.size < 16.0 && under.size < 16.0);
}

#[test]
fn xleftarrow_points_left_and_accepts_the_underscore_label() {
    let left = lay(r"\xleftarrow{x}", false);
    let right = lay(r"\xrightarrow{x}", false);
    // Same geometry, mirrored: the shaft runs the whole width and the barbs meet at the tip.
    let pad = Metrics::default().padding_x;
    let (l, r) = (shaft(&left), shaft(&right));
    assert!((r.x0 - pad).abs() < 0.01, "the right arrow starts at the left edge: {r:?}");
    assert!((l.x1 - (left.width - pad)).abs() < 0.01, "the left arrow ends at the right edge: {l:?}");
    fn barbs(l: &Layout) -> Vec<&Seg> {
        l.segs.iter().filter(|s| (s.y0 - s.y1).abs() > 0.001).collect()
    }
    let touches = |l: &Layout, edge: f32| {
        barbs(l).iter().all(|s| (s.x0.min(s.x1) - edge).abs() < 0.01 || (s.x0.max(s.x1) - edge).abs() < 0.01)
    };
    assert_eq!(barbs(&right).len(), 2, "two strokes make the head");
    assert!(touches(&right, right.width - pad), "the head of a right arrow is at the right end");
    assert!(touches(&left, pad), "and mirrored for a left arrow");
    // The vault writes the label as a lone `_{..}`; it belongs below the shaft, like `[..]`.
    let bracket = lay(r"\xrightarrow[n\to 0]{}", true);
    let underscore = lay(r"\xrightarrow{_{n\to 0}}", true);
    assert_eq!(underscore.segs.len(), bracket.segs.len());
    let y = |l: &Layout| l.glyphs.iter().find(|g| g.text == "n").map(|g| (g.x, g.y));
    let (bx, by) = y(&bracket).unwrap();
    let (ux, uy) = y(&underscore).unwrap();
    assert!(by > shaft(&bracket).y0 && (by - uy).abs() < 0.01, "same below-label: {by} vs {uy}");
    assert!((bx - ux).abs() < 0.01);
}

#[test]
fn not_is_a_slash_over_the_base_inside_its_width() {
    let ink = render_full("=", 16.0, false, &Metrics::ZERO, &mut Approx).layout;
    let not = lay(r"\not=", false);
    assert_eq!(not.glyphs.len(), 1, "the base is what is drawn");
    let w = ink.width;
    let px = Metrics::default().padding_x;
    assert!((not.width - w - 2.0 * px).abs() < 0.001, "the slash does not advance");
    assert_eq!(not.segs.len(), 1, "one diagonal stroke");
    let s = not.segs[0];
    assert!(s.x0 < s.x1 && s.x0 >= px && s.x1 <= px + w, "stays inside the advance: {s:?}");
    assert!(s.y0 > s.y1, "rising slash, as \\not draws it");
    assert!(not.ascent > ink.ascent && not.descent > ink.descent, "the box grows to hold the slash");
}

#[test]
fn not_does_not_change_the_spacing_of_the_symbol() {
    let plain = lay(r"a=b", false).width;
    let slashed = lay(r"a\not=b", false).width;
    assert!((plain - slashed).abs() < 0.001, "a \\not is still a relation: {plain} vs {slashed}");
    let arrow = lay(r"a_n\not\to 0", false);
    assert!(arrow.glyphs.iter().any(|g| g.text == "→"));
    assert_eq!(arrow.segs.len(), 1);
    assert!(arrow.glyphs.iter().all(|g| !g.text.contains(r"\not")), "not kept as source");
}

#[test]
fn tag_is_exposed_in_the_parse_and_the_layout() {
    let p = parse_full(r"x=1 \tag{7}");
    let t = p.tag.expect("the tag is parsed out");
    assert_eq!(t.text, "7");
    assert!(!t.star);
    assert_eq!(t.label(), "(7)");

    let r = render_full(r"x=1 \tag{7}", 16.0, true, &Metrics::default(), &mut Approx);
    assert!(r.diagnostics.is_empty());
    let t = r.tag.expect("the layout reports the tag");
    assert_eq!(t.label(), "(7)");
    let l = r.layout.tag.as_ref().unwrap();
    assert_eq!((l.x, l.width), (t.x, t.width));
    assert!(t.x > 0.0 && t.width > 0.0);
    // The fallback run is drawn, past the end of the formula, so nothing is lost on screen.
    assert!(r.layout.glyphs.iter().any(|g| g.text == "(7)"), "{:?}", texts(&r.layout));
    let one = lay("x=1", true);
    assert!(t.x >= one.width, "the run sits after the formula");
    let mut m = Approx;
    assert!((t.width - m.width("(7)", 16.0, Face::Roman)).abs() < 0.001);
    assert_eq!(r.layout.glyphs.last().unwrap().text, "(7)", "and it is the last run of the line");
}

#[test]
fn tag_star_drops_the_parentheses_and_nested_tags_stay_in_place() {
    assert_eq!(parse_full(r"\tag*{2.1}").tag.unwrap().label(), "2.1");
    // Inside a group it is not an equation number, so the layout leaves the run where it stands.
    let p = parse_full(r"x{\tag{3}}");
    assert_eq!(p.tag.unwrap().text, "3");
    let r = render_full(r"x{\tag{3}}", 16.0, true, &Metrics::default(), &mut Approx);
    assert!(r.layout.tag.is_none());
    assert_eq!(r.tag.unwrap().text, "3", "the parse still reports it");
    assert!(r.layout.glyphs.iter().any(|g| g.text == "(3)"));
}

/// The symbol table sweep that took the whole test vault (all subjects, not just Analisis I) from
/// 508 unknown-command hits to zero. Every pair here is a command the vault writes and the table
/// did not carry; the expected glyph follows the Unicode shape of the analogous entry already in
/// the table (`\preccurlyeq` is `\prec` plus a curvy underline, `\blacksquare` the filled square).
#[test]
fn vault_symbol_sweep_has_a_glyph_for_every_missing_name() {
    let cases: &[(&str, &str)] = &[
        (r"\preccurlyeq", "≼"),
        (r"\preceq", "≼"),
        (r"\prec", "≺"),
        (r"\succ", "≻"),
        (r"\succeq", "≽"),
        (r"\succcurlyeq", "≽"),
        (r"\nmid", "∤"),
        (r"\nparallel", "∦"),
        (r"\blacksquare", "■"),
        (r"\square", "□"),
        (r"\checkmark", "✓"),
        (r"\veebar", "⊻"),
        (r"\vdash", "⊢"),
        (r"\dashv", "⊣"),
        (r"\models", "⊨"),
        (r"\longmapsto", "⟼"),
        (r"\longleftrightarrow", "⟷"),
        (r"\rightleftarrows", "⇄"),
        (r"\rightsquigarrow", "⇝"),
        (r"\sqcap", "⊓"),
        (r"\sqcup", "⊔"),
        (r"\leqslant", "⩽"),
        (r"\geqslant", "⩾"),
        (r"\subsetneq", "⊊"),
        (r"\nsubseteq", "⊈"),
        (r"\llbracket", "⟦"),
        (r"\rrbracket", "⟧"),
        (r"\backslash", "∖"),
    ];
    for (src, glyph) in cases {
        let p = parse_full(src);
        assert!(p.diagnostics.is_empty(), "{src}: {:?}", p.diagnostics);
        let l = lay(src, false);
        assert!(texts(&l).contains(glyph), "{src} -> {glyph}: {:?}", texts(&l));
    }
}

#[test]
fn pmod_wraps_its_argument_and_spaces_like_a_relation() {
    let l = lay(r"a\equiv b\pmod n", false);
    let t = texts(&l);
    for s in ["(", "mod", "n", ")"] {
        assert!(t.contains(&s), "{t:?}");
    }
    let word = l.glyphs.iter().find(|g| g.text == "mod").unwrap();
    assert_eq!(word.face, Face::Mono, "the mod word is the monospace run");
    let b = l.glyphs.iter().find(|g| g.text == "b").unwrap();
    let open = l.glyphs.iter().find(|g| g.text == "(").unwrap();
    let mut m = Approx;
    let expected = b.x + m.width("b", b.size, b.face) + 0.278 * 16.0;
    assert!((open.x - expected).abs() < 0.01, "relation gap before (mod: {} vs {expected}", open.x);
    assert!(parse_full(r"a\equiv b\pmod n").diagnostics.is_empty());
}

#[test]
fn bmod_is_a_binary_operator_in_the_monospace_face() {
    let l = lay(r"a\bmod n", false);
    let word = l.glyphs.iter().find(|g| g.text == "mod").unwrap();
    assert_eq!(word.face, Face::Mono);
    let a = l.glyphs.iter().find(|g| g.text == "a").unwrap();
    let mut m = Approx;
    let expected = a.x + m.width("a", a.size, a.face) + 0.222 * 16.0;
    assert!((word.x - expected).abs() < 0.01, "binary gap before mod: {} vs {expected}", word.x);
}

#[test]
fn check_is_a_caron_over_the_base() {
    let plain = lay("v", false);
    let checked = lay(r"\check v", false);
    assert_eq!(texts(&checked), vec!["v"], "the accent adds no glyph");
    assert_eq!(checked.segs.len(), 2, "two strokes make the caron");
    let v = &checked.glyphs[0];
    assert!(checked.segs.iter().all(|s| s.y0 < v.y && s.y1 < v.y), "the caron sits above the baseline");
    assert!(checked.ascent > plain.ascent, "the box grows for the accent");
    assert!(checked.glyphs.iter().all(|g| g.text != "ˇ"), "the caron is drawn, not typed");
}

#[test]
fn texttt_is_a_monospace_run_with_its_spaces() {
    let l = lay(r"\texttt{if x}", false);
    assert_eq!(texts(&l), vec!["if x"], "spaces survive the raw group");
    assert_eq!(l.glyphs[0].face, Face::Mono);
    assert!(parse_full(r"\texttt{a_b}").diagnostics.is_empty());
}

#[test]
fn hline_draws_a_rule_over_the_row_and_takes_no_width() {
    let plain = lay(r"\begin{aligned}a&=1\\b&=2\end{aligned}", true);
    let ruled = lay(r"\begin{aligned}a&=1\\\hline b&=2\end{aligned}", true);
    assert_eq!(ruled.width, plain.width, "the rule does not advance the row");
    let rule = ruled
        .segs
        .iter()
        .find(|s| (s.y0 - s.y1).abs() < 0.001 && (s.x1 - s.x0) > 0.9 * (ruled.width - 12.0))
        .expect("a full-width horizontal rule");
    let one = ruled.glyphs.iter().find(|g| g.text == "1").unwrap();
    let two = ruled.glyphs.iter().find(|g| g.text == "2").unwrap();
    assert!(rule.y0 > one.y && rule.y0 < two.y, "the rule sits between the two rows");
    assert!(ruled.segs.len() > plain.segs.len(), "and is drawn");
}

#[test]
fn middle_stretches_with_the_fence() {
    let l = lay(r"\left\{\frac{a}{b}\middle|c\right\}", true);
    assert!(l.glyphs.iter().all(|g| g.text != "|"), "the middle is stretched, not a natural glyph");
    let top = l.segs.iter().map(|s| s.y0.min(s.y1)).fold(f32::MAX, f32::min);
    let bot = l.segs.iter().map(|s| s.y0.max(s.y1)).fold(f32::MIN, f32::max);
    let bar =
        l.segs.iter().find(|s| s.x0 == s.x1 && s.x0 > 20.0 && s.x0 < 35.0).expect("a vertical middle bar");
    assert!((bar.y0.min(bar.y1) - top).abs() < 0.01 && (bar.y0.max(bar.y1) - bot).abs() < 0.01, "{bar:?}");
    // The left brace strokes end at x <= 11.22 and the right ones start at x >= 39.65; the bar
    // is between the `a/b` chunk and `c`, so it has to fall in that gap.
    let a_chunk_right = l.glyphs.iter().find(|g| g.text == "a").unwrap().x + 8.8;
    let c_chunk_left = l.glyphs.iter().find(|g| g.text == "c").unwrap().x;
    assert!(bar.x0 > a_chunk_right && bar.x0 < c_chunk_left, "bar between chunks: {bar:?}");
}

#[test]
fn phantom_keeps_the_box_and_drops_the_ink() {
    let full = lay("abcd", false);
    let phantom = lay(r"a\phantom{bc}d", false);
    assert_eq!(texts(&phantom), vec!["a", "d"]);
    assert!((phantom.width - full.width).abs() < 0.001, "same advance as the measured body");
    assert!(phantom.segs.is_empty());
}

#[test]
fn bigm_variants_are_relations_and_the_plain_ones_ordinary() {
    // Same glyph, same size: the only difference between `\Bigm|` and `\Big|` is the spacing
    // class the suffix asks for, so the two thick spaces around a relation are the whole delta.
    for (m, plain) in [(r"\bigm", r"\big"), (r"\Bigm", r"\Big"), (r"\biggm", r"\bigg"), (r"\Biggm", r"\Bigg")]
    {
        let rel = lay(&format!("a{m}|b"), false).width;
        let ord = lay(&format!("a{plain}|b"), false).width;
        assert!((rel - ord - 2.0 * THICK * 16.0).abs() < 0.01, "{m}: {rel} vs {ord}");
        assert!(parse_full(&format!("a{m}|b")).diagnostics.is_empty());
    }
}

#[test]
fn sized_delimiters_are_drawn_at_the_texbook_sizes_on_the_axis() {
    for (cmd, scale) in [(r"\big", 1.2), (r"\Big", 1.8), (r"\bigg", 2.4), (r"\Bigg", 3.0)] {
        let l = lay(&format!(r"{cmd}(x"), false);
        let paren = l.glyphs.iter().find(|g| g.text == "(").unwrap();
        assert!((paren.size - 16.0 * scale).abs() < 0.01, "{cmd}: {} at {}", paren.text, paren.size);
        // Centred on the math axis, which is a quarter of an em above the baseline.
        let tight = render_full(&format!(r"{cmd}("), 16.0, false, &Metrics::ZERO, &mut Approx).layout;
        let axis = (tight.ascent - tight.descent) / 2.0;
        assert!((axis - 0.25 * 16.0).abs() < 0.01, "{cmd} is on the axis: {axis}");
        assert!(parse_full(&format!(r"{cmd}(")).diagnostics.is_empty());
    }
}

#[test]
fn the_other_subjects_have_no_unknown_commands_left() {
    // One real line per subject that used to fall back to source text.
    for src in [
        r"x\cdot y=0_A\iff x\preccurlyeq\bar y",
        r"28-5k\equiv0\pmod 3",
        r"\check v\in\mathbb{R}^{2}",
        r"\begin{aligned}001110&=\ \ 14\\ 110010&=-14\\ \hline 000000&=\ \ \ 0\end{aligned}",
        r"(\text{IDENTIFICADOR},\ \texttt{contador})",
        r"\left\{\operatorname{cl}(x)\ \middle|\ x\in\left[\dfrac{5}{2},+\infty\right)\right\}",
        r"\mathbb{R}^{2}/R=\Bigl\{\operatorname{cl}(u;0),\ \operatorname{cl}(u;1)\ \Bigm|\ u\in[0,+\infty)\Bigr\}",
    ] {
        assert!(parse_full(src).diagnostics.is_empty(), "{src}: {:?}", parse_full(src).diagnostics);
    }
}

/// A measure where every character is exactly half an em, so a test can read the space the layout
/// put between two atoms off their positions without knowing any font.
struct Fixed;

impl Measure for Fixed {
    fn width(&mut self, text: &str, size: f32, _face: Face) -> f32 {
        0.5 * size * text.chars().count() as f32
    }
}

/// Distance in pixels from the start of the first item of `src` to the start of the second, laid
/// out tight at 16px with [`Fixed`]: half an em (8px) for the first item, the rest is spacing.
fn item_gap(src: &str) -> f32 {
    let l = layout_nodes_with(&parse(src), 16.0, false, &Metrics::ZERO, &mut Fixed);
    l.glyphs[1].x - l.glyphs[0].x
}

const THIN: f32 = 3.0 / 18.0;
const MEDIUM: f32 = 4.0 / 18.0;
const THICK: f32 = 5.0 / 18.0;

#[test]
fn inter_atom_spaces_follow_the_texbook_table() {
    // TeXbook, Chapter 18: thin 3mu, medium 4mu, thick 5mu, and nothing where the table says so.
    for (src, want) in [
        ("ab", 0.0),     // Ord + Ord
        ("a(", 0.0),     // Ord + Open
        ("(a", 0.0),     // Open + Ord
        (")+a", MEDIUM), // Close + Bin; the `a` keeps the `+` a Bin
        (",a", THIN),    // Punct + Ord
        (",,", THIN),    // Punct + Punct (was dropped as if it were Punct + Close)
        ("a+b", MEDIUM), // Ord + Bin
        ("a=b", THICK),  // Ord + Rel
        ("=a", THICK),   // Rel + Ord
        ("+a", 0.0),     // a leading Bin is an Ord, and Ord + Ord is no space
        (")+", 0.0),     // a trailing Bin is an Ord as well
    ] {
        let got = item_gap(src) / 16.0 - 0.5;
        assert!((got - want).abs() < 0.001, "{src}: {got} em, want {want}");
    }
}

#[test]
fn fractions_fences_and_matrices_space_like_inner_atoms() {
    // Table 18.4's Inner row and column: a fraction beside an ordinary atom takes a thin space,
    // which Ord + Ord would not have. The 0.1em is the fraction's own numerator centring.
    assert!((item_gap(r"a\frac{1}{2}") / 16.0 - 0.5 - (THIN + 0.1)).abs() < 0.001);
    assert!((item_gap(r")\frac{1}{2}") / 16.0 - 0.5 - (THIN + 0.1)).abs() < 0.001);
    // A `\left..\right` group and a matrix are Inner too: Ord + Inner is thin, the extra 0.025em
    // being the air the fence puts around its own delimiter.
    assert!((item_gap(r"a\left(1\right)") / 16.0 - 0.5 - (THIN + 0.025)).abs() < 0.001);
    assert!((item_gap(r"a\begin{pmatrix}1\end{pmatrix}") / 16.0 - 0.5 - (THIN + 0.025)).abs() < 0.001);
}

#[test]
fn script_style_takes_no_inter_atom_space() {
    let l = layout_nodes_with(&parse("a^{b+c}"), 16.0, false, &Metrics::ZERO, &mut Fixed);
    let b = l.glyphs.iter().find(|g| g.text == "b").unwrap();
    let plus = l.glyphs.iter().find(|g| g.text == "+").unwrap();
    // The scripts are laid out at 0.7 of the size, so a half em there is 5.6px, not 8: the two
    // atoms are adjacent, with no medium space between them.
    assert!((plus.x - b.x - 5.6).abs() < 0.001, "no medium space in a superscript: {}", plus.x - b.x);
}

#[test]
fn sqrt_index_stands_on_the_vinculum_inside_the_box() {
    let l = lay(r"\sqrt[3]{x^2+1}", false);
    let idx = l.glyphs.iter().find(|g| g.text == "3").unwrap();
    let body = l.glyphs.iter().find(|g| g.text == "x").unwrap();
    let bar = l.segs.iter().min_by(|a, b| a.y0.total_cmp(&b.y0)).unwrap();
    assert!(idx.size < body.size, "scriptscript size: {} vs {}", idx.size, body.size);
    assert!((idx.y - bar.y0).abs() < 0.01, "the index stands on the bar: {} vs {}", idx.y, bar.y0);
    assert!(idx.x < body.x, "the index is left of the body: {} vs {}", idx.x, body.x);
    assert!(idx.y - idx.size * 0.72 > 0.0, "the index's ink is inside the box, not above it");
    // The box grows for the index: without it there is nothing above the vinculum.
    let without = render_full(r"\sqrt{x^2+1}", 16.0, false, &Metrics::ZERO, &mut Approx).layout;
    assert!(l.ascent > without.ascent, "{} vs {}", l.ascent, without.ascent);
}

/// A measure that reports ink the way a real font's metrics do: 1.4em up, 0.6em down.
struct Tall;

impl Measure for Tall {
    fn width(&mut self, text: &str, size: f32, face: Face) -> f32 {
        Approx.width(text, size, face)
    }

    fn extents(&mut self, _text: &str, size: f32, _face: Face) -> (f32, f32) {
        (1.4 * size, 0.6 * size)
    }
}

#[test]
fn extents_come_from_the_measure_when_it_has_them() {
    let est = render_full("x", 16.0, false, &Metrics::ZERO, &mut Approx).layout;
    let real = render_full("x", 16.0, false, &Metrics::ZERO, &mut Tall).layout;
    assert!((real.ascent - 1.4 * 16.0).abs() < 0.01, "{}", real.ascent);
    assert!((real.descent - 0.6 * 16.0).abs() < 0.01, "{}", real.descent);
    assert!(real.ascent > est.ascent && real.descent > est.descent);
    // Scripts stack off the measured extents, so the box follows the font and not the table.
    let tall = render_full("x^2", 16.0, false, &Metrics::ZERO, &mut Tall).layout;
    let approx = render_full("x^2", 16.0, false, &Metrics::ZERO, &mut Approx).layout;
    assert!(tall.height() > approx.height(), "{} vs {}", tall.height(), approx.height());
}

/// The display formulas of the note that reported them shown raw: a decimal comma `{,}` and the
/// commands around it (`\lvert`, `\rvert`, `\le`, `\cdot`) are read, not drawn as their source.
#[test]
fn the_reported_display_formulas_have_nothing_unread() {
    let snippets = [
        (r"P_4(0{,}3)=1-0{,}045+\frac{0{,}0081}{24}=1-0{,}045+0{,}0003375=0{,}9553375", '='),
        (r"\lvert R_4(0{,}3)\rvert\le\frac{0{,}00243}{120}=2{,}03\cdot 10^{-5}", '≤'),
    ];
    for (s, mark) in snippets {
        let r = render_full(s, 20.0, true, &Metrics::default(), &mut Approx);
        assert!(r.diagnostics.is_empty(), "{s}: {:?}", r.diagnostics);
        // A command the engine cannot read is drawn as its own text: a backslash never survives.
        let drawn: String = r.layout.glyphs.iter().map(|g| g.text.as_str()).collect();
        assert!(!drawn.contains('\\'), "{s} drew its source: {drawn}");
        // The decimal comma is punctuation of the number, at the size of the text around it.
        assert!(drawn.contains(','), "{s}: {drawn}");
        assert!(drawn.contains(mark), "{s}: {drawn}");
        // `\frac` is a rule between its parts, never the word `frac`.
        assert!(!r.layout.segs.is_empty(), "{s}: no fraction rule");
        assert!(r.layout.width > 0.0 && r.layout.height() > 0.0);
    }
}
