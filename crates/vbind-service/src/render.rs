//! Renders a layout view to PNG so an agent can look at what it drew: the
//! drawing (and photo, if asked), a coordinate grid in view units, and every
//! placed control as a numbered marker with a legend.

use crate::photos::PhotoInfo;
use base64::Engine as _;
use std::fmt::Write;
use std::sync::{Arc, OnceLock};
use vbind_core::drawing::{PartStyle, View};
use vbind_core::layout::DeviceLayout;

pub struct RenderOptions {
    /// Largest width or height of the drawing area in pixels.
    pub size_px: f64,
    pub grid: bool,
    /// Controls to fill in solid, for example the ones just moved.
    pub highlight: Vec<String>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions { size_px: 760.0, grid: true, highlight: Vec::new() }
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn fmt(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() { format!("{}", r as i64) } else { format!("{r}") }
}

/// Sort key that puts `b2` before `b10`: buttons, then hats, then axes.
fn natural_key(id: &str) -> (u8, String, u32) {
    let digits: String = id.chars().filter(char::is_ascii_digit).collect();
    let prefix: String = id.chars().filter(|c| !c.is_ascii_digit()).collect();
    let group = if id.starts_with('b') && !digits.is_empty() && prefix == "b" {
        0
    } else if id.starts_with("hat") {
        1
    } else {
        2
    };
    (group, prefix, digits.parse().unwrap_or(0))
}

/// Grid step in view units: a round number giving about ten lines.
fn grid_step(extent: f64) -> f64 {
    let raw = extent / 10.0;
    let mag = 10f64.powf(raw.log10().floor());
    [1.0, 2.0, 5.0, 10.0].iter().map(|m| m * mag).find(|s| *s >= raw).unwrap_or(10.0 * mag)
}

/// The SVG source for one view.
pub fn view_svg(layout: &DeviceLayout, view: &View, photo: Option<(&PhotoInfo, &[u8])>, opts: &RenderOptions) -> String {
    let [w, h] = view.size;
    let s = (opts.size_px / w.max(h)).max(0.01);
    let (left, top, legend_w, bottom) = (54.0, 64.0, 330.0, 30.0);
    let (dw, dh) = (w * s, h * s);
    let placed: Vec<(&String, &vbind_core::layout::ControlInfo, [f64; 2])> = layout
        .controls
        .iter()
        .filter_map(|(k, c)| {
            let at = c.at?;
            (layout.view_of(c).map(|v| v.id.as_str()) == Some(view.id.as_str())).then_some((k, c, at))
        })
        .collect();
    let mut placed = placed;
    placed.sort_by_key(|(k, _, _)| natural_key(k));
    let mut unplaced: Vec<&String> = layout.controls.iter().filter(|(_, c)| c.at.is_none()).map(|(k, _)| k).collect();
    unplaced.sort_by_key(|k| natural_key(k));
    let legend_rows = placed.len() + if unplaced.is_empty() { 0 } else { 3 + unplaced.len().div_ceil(6) };
    let total_h = (top + dh + bottom).max(top + 22.0 * legend_rows as f64 + 40.0);
    let total_w = left + dw + 40.0 + legend_w;
    let mut o = String::new();
    let _ = write!(
        o,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{tw}" height="{th}" viewBox="0 0 {tw} {th}" font-family="Segoe UI, Arial, sans-serif">"##,
        tw = fmt(total_w),
        th = fmt(total_h)
    );
    let _ = write!(o, r##"<rect width="100%" height="100%" fill="#1c1a19"/>"##);
    let _ = write!(
        o,
        r##"<text x="{left}" y="26" fill="#f5f1ee" font-size="17" font-weight="600">{} - {}</text><text x="{left}" y="46" fill="#a69e98" font-size="12">view '{}', {} x {} units. Grid lines are in view units; control positions use the same units.</text>"##,
        esc(&layout.name),
        esc(&view.label),
        esc(&view.id),
        fmt(w),
        fmt(h)
    );
    let _ = write!(o, r##"<rect x="{left}" y="{top}" width="{}" height="{}" fill="#141211" stroke="#3c3734"/>"##, fmt(dw), fmt(dh));
    if opts.grid {
        for (extent, horizontal) in [(w, true), (h, false)] {
            let step = grid_step(extent);
            let mut v = step;
            while v < extent - 1e-9 {
                let p = v * s;
                if horizontal {
                    let x = left + p;
                    let _ = write!(o, r##"<line x1="{x}" y1="{top}" x2="{x}" y2="{}" stroke="#2f2b29"/><text x="{x}" y="{}" fill="#8f8882" font-size="10" text-anchor="middle">{}</text>"##, fmt(top + dh), fmt(top - 5.0), fmt(v));
                } else {
                    let y = top + p;
                    let _ = write!(o, r##"<line x1="{left}" y1="{y}" x2="{}" y2="{y}" stroke="#2f2b29"/><text x="{}" y="{}" fill="#8f8882" font-size="10" text-anchor="end">{}</text>"##, fmt(left + dw), fmt(left - 6.0), fmt(y + 3.5), fmt(v));
                }
                v += step;
            }
        }
    }
    let _ = write!(o, r##"<g transform="translate({left} {top}) scale({})">"##, fmt(s));
    if let Some((info, bytes)) = photo {
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        let [x, y, pw, ph] = info.fit;
        let _ = write!(o, r##"<clipPath id="view"><rect width="{}" height="{}"/></clipPath>"##, fmt(w), fmt(h));
        let _ = write!(
            o,
            r##"<image x="{}" y="{}" width="{}" height="{}" preserveAspectRatio="none" clip-path="url(#view)" href="data:{};base64,{data}"/>"##,
            fmt(x),
            fmt(y),
            fmt(pw),
            fmt(ph),
            info.mime
        );
    }
    let sw = 1.4 / s;
    let photo_on = photo.is_some();
    for shape in view.resolve().shapes {
        // Over a photo, draw outlines only so the agent can compare the two.
        let (fill, stroke, width) = match (shape.style, photo_on) {
            (_, true) => ("none", "#e8917a", sw),
            (PartStyle::Body, _) => ("#2b2826", "#a69e98", sw),
            (PartStyle::Detail, _) => ("#1a1817", "#a69e98", sw),
            (PartStyle::Hole, _) => ("#141211", "#3c3734", sw),
            (PartStyle::Line, _) => ("none", "#a69e98", sw * 1.2),
        };
        let _ = write!(o, r##"<path d="{}" fill="{fill}" fill-rule="evenodd" stroke="{stroke}" stroke-width="{}" stroke-linejoin="round" stroke-linecap="round"/>"##, shape.d, fmt(width));
    }
    let _ = write!(o, "</g>");
    let default_r = (w.max(h) * 0.022).max(7.0 / s);
    for (i, (key, info, [x, y])) in placed.iter().enumerate() {
        let (cx, cy) = (left + x * s, top + y * s);
        let r = (info.size.unwrap_or(default_r) * s).clamp(6.0, 40.0);
        let hot = opts.highlight.iter().any(|h| h == *key);
        let fill = if hot { "#b5523b" } else { "rgba(181,82,59,0.35)" };
        let _ = write!(
            o,
            r##"<circle cx="{}" cy="{}" r="{}" fill="{fill}" stroke="#e8917a" stroke-width="2"/><text x="{}" y="{}" fill="#ffffff" font-size="11" font-weight="700" text-anchor="middle">{}</text>"##,
            fmt(cx),
            fmt(cy),
            fmt(r),
            fmt(cx),
            fmt(cy + 4.0),
            i + 1
        );
    }
    let lx = left + dw + 40.0;
    let mut ly = top + 4.0;
    let _ = write!(o, r##"<text x="{lx}" y="{}" fill="#f5f1ee" font-size="13" font-weight="600">Controls on this view</text>"##, fmt(ly));
    ly += 24.0;
    for (i, (key, info, at)) in placed.iter().enumerate() {
        let _ = write!(
            o,
            r##"<circle cx="{}" cy="{}" r="9" fill="rgba(181,82,59,0.35)" stroke="#e8917a" stroke-width="1.5"/><text x="{}" y="{}" fill="#ffffff" font-size="10" font-weight="700" text-anchor="middle">{}</text><text x="{}" y="{}" fill="#e8917a" font-size="12" font-family="Consolas, monospace">{}</text><text x="{}" y="{}" fill="#cec6c1" font-size="12">{} <tspan fill="#8f8882">[{}, {}]</tspan></text>"##,
            fmt(lx + 9.0),
            fmt(ly - 4.0),
            fmt(lx + 9.0),
            fmt(ly),
            i + 1,
            fmt(lx + 26.0),
            fmt(ly),
            esc(key),
            fmt(lx + 80.0),
            fmt(ly),
            esc(&info.label),
            fmt(at[0]),
            fmt(at[1])
        );
        ly += 22.0;
    }
    if placed.is_empty() {
        let _ = write!(o, r##"<text x="{lx}" y="{}" fill="#a69e98" font-size="12">None yet. Place controls with at: [x, y].</text>"##, fmt(ly));
        ly += 22.0;
    }
    if !unplaced.is_empty() {
        ly += 14.0;
        let _ = write!(o, r##"<text x="{lx}" y="{}" fill="#f5f1ee" font-size="13" font-weight="600">Not placed on any view yet</text>"##, fmt(ly));
        ly += 20.0;
        for chunk in unplaced.chunks(6) {
            let line = chunk.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(", ");
            let _ = write!(o, r##"<text x="{lx}" y="{}" fill="#a69e98" font-size="12" font-family="Consolas, monospace">{}</text>"##, fmt(ly), esc(&line));
            ly += 18.0;
        }
    }
    o.push_str("</svg>");
    o
}

fn fontdb() -> Arc<resvg::usvg::fontdb::Database> {
    static DB: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = resvg::usvg::fontdb::Database::new();
        db.load_system_fonts();
        db.set_sans_serif_family("Segoe UI");
        db.set_monospace_family("Consolas");
        Arc::new(db)
    })
    .clone()
}

/// Rasterise SVG source to PNG bytes.
pub fn png(svg: &str) -> Result<Vec<u8>, String> {
    let opt = resvg::usvg::Options { fontdb: fontdb(), font_family: "Segoe UI".into(), ..Default::default() };
    let tree = resvg::usvg::Tree::from_str(svg, &opt).map_err(|e| format!("could not draw the view: {e}"))?;
    let size = tree.size().to_int_size();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size.width(), size.height()).ok_or("the view is too big to draw")?;
    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pixmap.as_mut());
    pixmap.encode_png().map_err(|e| format!("could not encode the picture: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_view_to_png() {
        let l: DeviceLayout = serde_json::from_str(
            r#"{"vbind_layout":1,"id":"s","name":"Stick & co","match":[{"vid":"1234"}],
                "views":[{"id":"side","label":"Side","size":[140,300],"parts":[
                    {"kit":"base.rounded","box":[0,256,140,42]},{"kit":"grip.contoured","box":[42,88,60,160]},{"kit":"hat","box":[65,33,18,18]}]}],
                "controls":{"b1":{"label":"Trigger <1>","at":[34,130]},"hat1":{"label":"Hat","at":[74,42]},"b2":{"label":"Base"}}}"#,
        )
        .unwrap();
        assert!(l.validate().is_empty(), "{:?}", l.validate());
        let svg = view_svg(&l, &l.views[0], None, &RenderOptions { highlight: vec!["b1".into()], ..Default::default() });
        assert!(svg.contains("Stick &amp; co") && svg.contains("Trigger &lt;1&gt;") && svg.contains("Not placed"));
        let png = png(&svg).unwrap();
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']) && png.len() > 2000);
        assert_eq!(grid_step(300.0), 50.0);
        assert_eq!(grid_step(140.0), 20.0);
    }
}
