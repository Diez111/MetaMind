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
    ] {
        assert!(parse_full(src).diagnostics.is_empty(), "{src}: {:?}", parse_full(src).diagnostics);
    }
}
