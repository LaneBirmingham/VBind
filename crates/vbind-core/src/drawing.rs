//! Device pictures: views of a device drawn from a kit of generic parts and
//! free SVG paths. An agent writes them into a layout card for any device;
//! VBind ships no device-specific art.

use crate::path::{Affine, Path};
use crate::profile::{Diagnostic, is_valid_id};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Most views a layout may have.
pub const MAX_VIEWS: usize = 8;
/// Most parts in one view.
pub const MAX_PARTS: usize = 300;

/// How a part is painted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PartStyle {
    /// The device's body: filled and outlined.
    #[default]
    Body,
    /// A raised piece on the body, such as a button cap or a hat: a darker fill.
    Detail,
    /// A recess or gap: filled with the background.
    Hole,
    /// Outline only, for grooves, seams and the cross on a hat.
    Line,
}

/// One piece of a view: a kit part placed in a box, or a free SVG path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Part {
    /// Kit part name, for example "grip.contoured" (see the layout kit list).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit: Option<String>,
    /// Where a kit part sits: `[x, y, width, height]` in view units. The part is stretched to fill it.
    #[serde(rename = "box", default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<[f64; 4]>,
    /// SVG path data in view units, for shapes the kit has no part for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Degrees clockwise, around the box centre (kit parts only).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rotate: f64,
    /// Mirror left to right inside the box (kit parts only), for example a left-hand grip.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip: bool,
    /// Paint style. Kit parts have their own styles; this overrides the main shape's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<PartStyle>,
    /// A note on what the part is, for whoever edits the file next.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// A picture of the device from one side.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct View {
    /// Lowercase id, for example "side", "head" or "base".
    pub id: String,
    /// Shown above the picture, for example "Side" or "Head, from above".
    pub label: String,
    /// Width and height of the view in view units. Parts and control positions use this space.
    pub size: [f64; 2],
    /// Drawn in order, so later parts sit on top.
    pub parts: Vec<Part>,
}

/// A shape ready to draw: absolute path data in view units and its style.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Shape {
    pub d: String,
    pub style: PartStyle,
}

/// A view with every part turned into plain shapes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ResolvedView {
    pub id: String,
    pub label: String,
    pub size: [f64; 2],
    pub shapes: Vec<Shape>,
}

/// A kit part: shapes drawn in a 100 x 100 box.
pub struct KitPart {
    pub name: &'static str,
    pub description: &'static str,
    pub shapes: &'static [(&'static str, PartStyle)],
}

use PartStyle::{Body, Detail, Hole, Line};

const CIRCLE: &str = "M50 0 C77.6 0 100 22.4 100 50 C100 77.6 77.6 100 50 100 C22.4 100 0 77.6 0 50 C0 22.4 22.4 0 50 0 Z";
const ROUNDED: &str = "M12 0 L88 0 C94.6 0 100 5.4 100 12 L100 88 C100 94.6 94.6 100 88 100 L12 100 C5.4 100 0 94.6 0 88 L0 12 C0 5.4 5.4 0 12 0 Z";

/// The parts kit. Boxes stretch parts, so pick a box with the right proportions.
pub const KIT: &[KitPart] = &[
    KitPart { name: "grip.straight", description: "Straight stick grip, side view (tall box)", shapes: &[("M22 3 C22 1 25 0 28 0 L72 0 C75 0 78 1 78 3 L84 96 C84 98.5 81 100 77 100 L23 100 C19 100 16 98.5 16 96 Z", Body)] },
    KitPart {
        name: "grip.contoured",
        description: "Ergonomic stick grip with finger grooves on the front (left) edge, side view (tall box)",
        shapes: &[("M13 100 C9 94 5 88 6 82 C2 76 2 70 5 64 C1 58 1 52 4 46 C2 38 3 31 7 26 C10 14 13 5 17 0 L87 0 C97 13 100 27 97 43 C93 66 87 85 80 100 Z", Body)],
    },
    KitPart { name: "grip.pistol", description: "Pistol grip that leans back, side view", shapes: &[("M30 100 L36 30 C38 12 50 2 66 2 L84 6 C92 8 94 16 90 24 L80 48 L72 100 Z", Body)] },
    KitPart { name: "head.round", description: "Round grip head", shapes: &[(CIRCLE, Body)] },
    KitPart {
        name: "head.wide",
        description: "Wide head that overhangs the grip, flat underneath, side view",
        shapes: &[("M8.4 94.4 C1.2 52.8 15.7 8.3 49.4 2.8 C85.5 0 102.4 30.6 95.2 77.8 C92.8 91.7 85.5 100 75.9 100 L20.5 100 C14.5 100 9.6 98.6 8.4 94.4 Z", Body)],
    },
    KitPart { name: "head.angled", description: "Angular head that slopes forward, side view", shapes: &[("M4 100 L10 30 C12 12 24 2 42 2 L80 0 C92 0 100 8 100 20 L96 100 Z", Body)] },
    KitPart { name: "base.square", description: "Square-cornered base or panel", shapes: &[("M6 0 L94 0 C97.3 0 100 2.7 100 6 L100 94 C100 97.3 97.3 100 94 100 L6 100 C2.7 100 0 97.3 0 94 L0 6 C0 2.7 2.7 0 6 0 Z", Body)] },
    KitPart { name: "base.rounded", description: "Base with well rounded corners", shapes: &[("M18 0 L82 0 C92 0 100 12 100 30 L100 82 C100 92 96 100 88 100 L12 100 C4 100 0 92 0 82 L0 30 C0 12 8 0 18 0 Z", Body)] },
    KitPart { name: "base.round", description: "Round or oval base", shapes: &[(CIRCLE, Body)] },
    KitPart { name: "boot", description: "Rubber boot or collar where a stick meets its base", shapes: &[("M0 100 C15 70 30 0 50 0 C70 0 85 70 100 100 Z", Body)] },
    KitPart { name: "trigger", description: "Index-finger trigger curling forward (draw it at the grip's front edge; flip to curl right)", shapes: &[("M100 0 C40 8 18 50 36 100 L58 100 C46 62 60 28 100 20 Z", Detail)] },
    KitPart { name: "hat", description: "Hat switch with a cross", shapes: &[(CIRCLE, Detail), ("M50 20 L50 80 M20 50 L80 50", Line)] },
    KitPart { name: "button.round", description: "Round button", shapes: &[(CIRCLE, Detail)] },
    KitPart { name: "button.square", description: "Square button", shapes: &[(ROUNDED, Detail)] },
    KitPart { name: "ministick", description: "Small analog thumbstick", shapes: &[(CIRCLE, Detail), ("M50 22 C65.5 22 78 34.5 78 50 C78 65.5 65.5 78 50 78 C34.5 78 22 65.5 22 50 C22 34.5 34.5 22 50 22 Z", Line)] },
    KitPart { name: "wheel", description: "Thumb wheel or scroll wheel, seen edge on", shapes: &[(ROUNDED, Detail), ("M10 25 L90 25 M10 42 L90 42 M10 58 L90 58 M10 75 L90 75", Line)] },
    KitPart { name: "slider", description: "Slider track with its knob in the middle", shapes: &[("M0 35 L100 35 L100 65 L0 65 Z", Hole), ("M38 0 L62 0 L62 100 L38 100 Z", Detail)] },
    KitPart { name: "toggle", description: "Toggle switch: plate and bat", shapes: &[("M0 55 L100 55 L100 100 L0 100 Z", Detail), ("M42 60 L46 0 L54 0 L58 60 Z", Detail)] },
    KitPart { name: "lever", description: "Small lever or rocker", shapes: &[(ROUNDED, Detail)] },
    KitPart { name: "throttle.handle", description: "Throttle handle on a lever, side view", shapes: &[("M20 100 L28 18 C30 6 40 0 52 0 L72 0 C84 0 90 8 88 20 L76 100 Z", Body)] },
    KitPart { name: "pedal", description: "Foot pedal, from above", shapes: &[("M14 0 L86 0 C93 0 98 5 97 12 L90 92 C89.5 97 86 100 80 100 L20 100 C14 100 10.5 97 10 92 L3 12 C2 5 7 0 14 0 Z", Body)] },
    KitPart {
        name: "gamepad.body",
        description: "Game controller body, from the front",
        shapes: &[("M30 0 L70 0 C84 0 90 14.7 93 38.2 L97 76.5 C99 94.1 88 101 80 89.7 L70 73.5 L30 73.5 L20 89.7 C12 101 1 94.1 3 76.5 L7 38.2 C10 14.7 16 0 30 0 Z", Body)],
    },
    KitPart {
        name: "wheel.rim",
        description: "Steering wheel rim with a hub, from the front",
        shapes: &[
            ("M50 0 C77.6 0 100 22.4 100 50 C100 77.6 77.6 100 50 100 C22.4 100 0 77.6 0 50 C0 22.4 22.4 0 50 0 Z M50 12 C29 12 12 29 12 50 C12 71 29 88 50 88 C71 88 88 71 88 50 C88 29 71 12 50 12 Z", Body),
            ("M12 46 L88 46 L88 58 L12 58 Z M44 58 L56 58 L56 88 L44 88 Z", Body),
        ],
    },
    KitPart { name: "shape.rect", description: "Plain rectangle", shapes: &[("M0 0 L100 0 L100 100 L0 100 Z", Body)] },
    KitPart { name: "shape.rounded", description: "Rectangle with rounded corners", shapes: &[(ROUNDED, Body)] },
    KitPart { name: "shape.ellipse", description: "Ellipse or circle", shapes: &[(CIRCLE, Body)] },
    KitPart { name: "shape.pill", description: "Capsule with fully rounded ends", shapes: &[("M25 0 L75 0 C89 0 100 22 100 50 C100 78 89 100 75 100 L25 100 C11 100 0 78 0 50 C0 22 11 0 25 0 Z", Body)] },
];

pub fn kit_part(name: &str) -> Option<&'static KitPart> {
    KIT.iter().find(|k| k.name == name)
}

/// The kit as data, for agents and the schema.
pub fn kit_list() -> Vec<serde_json::Value> {
    KIT.iter().map(|k| serde_json::json!({ "kit": k.name, "description": k.description })).collect()
}

impl Part {
    /// The transform that takes a kit part's 100 x 100 box into view units.
    fn kit_transform(&self) -> Affine {
        let [x, y, w, h] = self.bbox.unwrap_or([0.0, 0.0, 100.0, 100.0]);
        let mut t = Affine::scale(w / 100.0, h / 100.0);
        if self.flip {
            t = Affine::scale(-1.0, 1.0).then(Affine::translate(100.0, 0.0)).then(t);
        }
        if self.rotate != 0.0 {
            let (cx, cy) = (w / 2.0, h / 2.0);
            t = t.then(Affine::translate(-cx, -cy)).then(Affine::rotate_deg(self.rotate)).then(Affine::translate(cx, cy));
        }
        t.then(Affine::translate(x, y))
    }

    /// Plain shapes for this part. Assumes the part has been validated.
    pub fn shapes(&self) -> Vec<Shape> {
        if let Some(name) = &self.kit {
            let Some(k) = kit_part(name) else { return Vec::new() };
            let t = self.kit_transform();
            return k
                .shapes
                .iter()
                .enumerate()
                .filter_map(|(i, (d, style))| {
                    let p = Path::parse(d).ok()?;
                    let style = if i == 0 { self.style.unwrap_or(*style) } else { *style };
                    Some(Shape { d: p.transform(t).to_svg(), style })
                })
                .collect();
        }
        match self.path.as_deref().map(Path::parse) {
            Some(Ok(p)) => vec![Shape { d: p.to_svg(), style: self.style.unwrap_or_default() }],
            _ => Vec::new(),
        }
    }
}

impl View {
    pub fn resolve(&self) -> ResolvedView {
        ResolvedView { id: self.id.clone(), label: self.label.clone(), size: self.size, shapes: self.parts.iter().flat_map(Part::shapes).collect() }
    }

    pub fn validate(&self, at: &str, errs: &mut Vec<Diagnostic>) {
        let d = |errs: &mut Vec<Diagnostic>, path: String, message: String| errs.push(Diagnostic { path, message, pos: None });
        if !is_valid_id(&self.id) {
            d(errs, format!("{at}.id"), "use lowercase letters, digits and dashes".into());
        }
        let [w, h] = self.size;
        if !(10.0..=4000.0).contains(&w) || !(10.0..=4000.0).contains(&h) {
            d(errs, format!("{at}.size"), "width and height must be between 10 and 4000".into());
        }
        if self.parts.len() > MAX_PARTS {
            d(errs, format!("{at}.parts"), format!("a view can have at most {MAX_PARTS} parts"));
        }
        // Shapes may poke out a little, but not wander off the picture.
        let (mx, my) = (w * 0.25, h * 0.25);
        for (i, p) in self.parts.iter().enumerate() {
            let here = format!("{at}.parts[{i}]");
            match (&p.kit, &p.path) {
                (Some(_), Some(_)) => d(errs, here.clone(), "give either kit or path, not both".into()),
                (None, None) => d(errs, here.clone(), "give a kit part name or an SVG path".into()),
                (Some(name), None) => {
                    if kit_part(name).is_none() {
                        let names = KIT.iter().map(|k| k.name).collect::<Vec<_>>().join(", ");
                        d(errs, format!("{here}.kit"), format!("unknown kit part '{name}'; parts are: {names}"));
                    }
                    match p.bbox {
                        None => d(errs, format!("{here}.box"), "kit parts need a box: [x, y, width, height]".into()),
                        Some(b) if b.iter().any(|v| !v.is_finite()) || b[2] <= 0.0 || b[3] <= 0.0 => {
                            d(errs, format!("{here}.box"), "width and height must be above 0".into())
                        }
                        _ => {}
                    }
                }
                (None, Some(path)) => {
                    if p.bbox.is_some() || p.rotate != 0.0 || p.flip {
                        d(errs, here.clone(), "box, rotate and flip apply to kit parts; write a path in view units instead".into());
                    }
                    if let Err(e) = Path::parse(path) {
                        errs.push(Diagnostic { path: format!("{here}.path"), message: e.message, pos: Some(e.pos) });
                    }
                }
            }
            if !p.rotate.is_finite() {
                d(errs, format!("{here}.rotate"), "rotate must be a number of degrees".into());
            }
            let bounds = p.shapes().iter().filter_map(|s| Path::parse(&s.d).ok()?.bounds()).reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]);
            if let Some(b) = bounds
                && (b[0] < -mx || b[1] < -my || b[2] > w + mx || b[3] > h + my)
            {
                d(errs, here, format!("this part reaches far outside the {w} x {h} view"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kit_part_parses() {
        for k in KIT {
            for (d, _) in k.shapes {
                let p = Path::parse(d).unwrap_or_else(|e| panic!("{}: {e}", k.name));
                let b = p.bounds().unwrap();
                assert!(b[0] >= -5.0 && b[1] >= -5.0 && b[2] <= 105.0 && b[3] <= 105.0, "{} leaves its box: {b:?}", k.name);
            }
        }
    }

    #[test]
    fn kit_parts_fill_their_box_and_flip() {
        let p = Part { kit: Some("shape.rect".into()), bbox: Some([10.0, 20.0, 30.0, 40.0]), path: None, rotate: 0.0, flip: false, style: None, note: None };
        assert_eq!(p.shapes()[0].d, "M 10 20 L 40 20 L 40 60 L 10 60 Z");
        let f = Part { kit: Some("trigger".into()), flip: true, ..p.clone() };
        let b = Path::parse(&f.shapes()[0].d).unwrap().bounds().unwrap();
        assert!(b[0] >= 9.9 && b[2] <= 40.1, "{b:?}");
        let r = Part { rotate: 90.0, ..p };
        let b = Path::parse(&r.shapes()[0].d).unwrap().bounds().unwrap();
        assert_eq!(b.map(|v| v.round()), [5.0, 25.0, 45.0, 55.0]);
    }

    #[test]
    fn validation_names_the_part() {
        let v: View = serde_json::from_str(
            r#"{"id":"side","label":"Side","size":[100,200],"parts":[
                {"kit":"grip.contoured","box":[20,40,50,120]},
                {"kit":"grip.nope","box":[0,0,10,10]},
                {"path":"M0 0 L10 x"},
                {"kit":"hat"},
                {"path":"M0 0 L900 900"}
            ]}"#,
        )
        .unwrap();
        let mut errs = Vec::new();
        v.validate("views[0]", &mut errs);
        let paths: Vec<_> = errs.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["views[0].parts[1].kit", "views[0].parts[2].path", "views[0].parts[3].box", "views[0].parts[4]"]);
        assert_eq!(errs[1].pos, Some(9));
    }
}
