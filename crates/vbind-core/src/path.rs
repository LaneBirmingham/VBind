//! A strict SVG path parser for agent-drawn device pictures.
//!
//! Paths are parsed into absolute moves, lines, quadratic and cubic curves
//! (arcs become cubics), so they can be moved, scaled and mirrored exactly,
//! and every path the UI or the renderer sees is re-written by VBind rather
//! than passed through as text.

use std::fmt::Write;

/// Longest path data accepted, in characters.
pub const MAX_PATH_LEN: usize = 20_000;
/// Most segments accepted in one path.
pub const MAX_SEGMENTS: usize = 4_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Move(f64, f64),
    Line(f64, f64),
    Quad(f64, f64, f64, f64),
    Cubic(f64, f64, f64, f64, f64, f64),
    Close,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path(pub Vec<Seg>);

/// A parse problem and the character position it was found at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathError {
    pub pos: usize,
    pub message: String,
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (at character {})", self.message, self.pos + 1)
    }
}

/// 2D affine transform `[a, b, c, d, e, f]`: x' = a x + c y + e, y' = b x + d y + f.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine(pub [f64; 6]);

impl Affine {
    pub const IDENTITY: Affine = Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    pub fn translate(x: f64, y: f64) -> Affine {
        Affine([1.0, 0.0, 0.0, 1.0, x, y])
    }
    pub fn scale(sx: f64, sy: f64) -> Affine {
        Affine([sx, 0.0, 0.0, sy, 0.0, 0.0])
    }
    pub fn rotate_deg(deg: f64) -> Affine {
        let (s, c) = deg.to_radians().sin_cos();
        Affine([c, s, -s, c, 0.0, 0.0])
    }
    /// `self` applied after `first`.
    pub fn then(self, next: Affine) -> Affine {
        let [a1, b1, c1, d1, e1, f1] = self.0;
        let [a2, b2, c2, d2, e2, f2] = next.0;
        Affine([
            a2 * a1 + c2 * b1,
            b2 * a1 + d2 * b1,
            a2 * c1 + c2 * d1,
            b2 * c1 + d2 * d1,
            a2 * e1 + c2 * f1 + e2,
            b2 * e1 + d2 * f1 + f2,
        ])
    }
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d, e, f] = self.0;
        (a * x + c * y + e, b * x + d * y + f)
    }
}

struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
}

impl Lexer<'_> {
    fn skip_sep(&mut self) {
        while self.i < self.s.len() && (self.s[self.i].is_ascii_whitespace() || self.s[self.i] == b',') {
            self.i += 1;
        }
    }
    fn err(&self, message: impl Into<String>) -> PathError {
        PathError { pos: self.i.min(self.s.len()), message: message.into() }
    }
    fn number(&mut self) -> Result<f64, PathError> {
        self.skip_sep();
        let start = self.i;
        let s = self.s;
        let mut j = self.i;
        if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
            j += 1;
        }
        let mut digits = 0;
        while j < s.len() && s[j].is_ascii_digit() {
            j += 1;
            digits += 1;
        }
        if j < s.len() && s[j] == b'.' {
            j += 1;
            while j < s.len() && s[j].is_ascii_digit() {
                j += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            return Err(self.err("expected a number"));
        }
        if j < s.len() && (s[j] == b'e' || s[j] == b'E') {
            let mut k = j + 1;
            if k < s.len() && (s[k] == b'+' || s[k] == b'-') {
                k += 1;
            }
            let exp_start = k;
            while k < s.len() && s[k].is_ascii_digit() {
                k += 1;
            }
            if k > exp_start {
                j = k;
            }
        }
        self.i = j;
        let text = std::str::from_utf8(&s[start..j]).unwrap_or("");
        let v: f64 = text.parse().map_err(|_| PathError { pos: start, message: format!("'{text}' is not a number") })?;
        if !v.is_finite() || v.abs() > 1e6 {
            return Err(PathError { pos: start, message: "number out of range".into() });
        }
        Ok(v)
    }
    fn flag(&mut self) -> Result<bool, PathError> {
        self.skip_sep();
        match self.s.get(self.i) {
            Some(b'0') => {
                self.i += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.i += 1;
                Ok(true)
            }
            _ => Err(self.err("expected an arc flag, 0 or 1")),
        }
    }
}

impl Path {
    /// Parse SVG path data (`d`). All commands are supported: M L H V C S Q T A Z,
    /// upper and lower case.
    pub fn parse(d: &str) -> Result<Path, PathError> {
        if d.len() > MAX_PATH_LEN {
            return Err(PathError { pos: MAX_PATH_LEN, message: format!("path is longer than {MAX_PATH_LEN} characters") });
        }
        let mut lx = Lexer { s: d.as_bytes(), i: 0 };
        let mut out = Vec::new();
        let (mut cx, mut cy) = (0.0f64, 0.0f64);
        let (mut sx, mut sy) = (0.0f64, 0.0f64);
        // Last control point, for S and T reflections.
        let mut last_c: Option<(f64, f64)> = None;
        let mut last_q: Option<(f64, f64)> = None;
        let mut cmd: Option<u8> = None;
        loop {
            lx.skip_sep();
            if lx.i >= lx.s.len() {
                break;
            }
            let ch = lx.s[lx.i];
            let start = lx.i;
            if ch.is_ascii_alphabetic() {
                if !b"MmLlHhVvCcSsQqTtAaZz".contains(&ch) {
                    return Err(lx.err(format!("'{}' is not a path command", ch as char)));
                }
                lx.i += 1;
                cmd = Some(ch);
            } else if cmd.is_none() {
                return Err(lx.err("a path must start with M"));
            } else if matches!(cmd, Some(b'Z' | b'z')) {
                return Err(lx.err("numbers after Z need a command"));
            }
            let c = cmd.unwrap();
            if out.is_empty() && c != b'M' && c != b'm' {
                return Err(PathError { pos: start, message: "a path must start with M".into() });
            }
            let rel = c.is_ascii_lowercase();
            let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
            let mut q_next = None;
            let mut c_next = None;
            match c.to_ascii_uppercase() {
                b'M' => {
                    let x = lx.number()? + ox;
                    let y = lx.number()? + oy;
                    out.push(Seg::Move(x, y));
                    (cx, cy, sx, sy) = (x, y, x, y);
                    // Extra pairs after a move are lines.
                    cmd = Some(if rel { b'l' } else { b'L' });
                }
                b'L' => {
                    let x = lx.number()? + ox;
                    let y = lx.number()? + oy;
                    out.push(Seg::Line(x, y));
                    (cx, cy) = (x, y);
                }
                b'H' => {
                    let x = lx.number()? + ox;
                    out.push(Seg::Line(x, cy));
                    cx = x;
                }
                b'V' => {
                    let y = lx.number()? + oy;
                    out.push(Seg::Line(cx, y));
                    cy = y;
                }
                b'C' => {
                    let v = [lx.number()?, lx.number()?, lx.number()?, lx.number()?, lx.number()?, lx.number()?];
                    let s = Seg::Cubic(v[0] + ox, v[1] + oy, v[2] + ox, v[3] + oy, v[4] + ox, v[5] + oy);
                    out.push(s);
                    c_next = Some((v[2] + ox, v[3] + oy));
                    (cx, cy) = (v[4] + ox, v[5] + oy);
                }
                b'S' => {
                    let v = [lx.number()?, lx.number()?, lx.number()?, lx.number()?];
                    let (x1, y1) = last_c.map(|(px, py)| (2.0 * cx - px, 2.0 * cy - py)).unwrap_or((cx, cy));
                    out.push(Seg::Cubic(x1, y1, v[0] + ox, v[1] + oy, v[2] + ox, v[3] + oy));
                    c_next = Some((v[0] + ox, v[1] + oy));
                    (cx, cy) = (v[2] + ox, v[3] + oy);
                }
                b'Q' => {
                    let v = [lx.number()?, lx.number()?, lx.number()?, lx.number()?];
                    out.push(Seg::Quad(v[0] + ox, v[1] + oy, v[2] + ox, v[3] + oy));
                    q_next = Some((v[0] + ox, v[1] + oy));
                    (cx, cy) = (v[2] + ox, v[3] + oy);
                }
                b'T' => {
                    let v = [lx.number()?, lx.number()?];
                    let (x1, y1) = last_q.map(|(px, py)| (2.0 * cx - px, 2.0 * cy - py)).unwrap_or((cx, cy));
                    out.push(Seg::Quad(x1, y1, v[0] + ox, v[1] + oy));
                    q_next = Some((x1, y1));
                    (cx, cy) = (v[0] + ox, v[1] + oy);
                }
                b'A' => {
                    let rx = lx.number()?;
                    let ry = lx.number()?;
                    let rot = lx.number()?;
                    let large = lx.flag()?;
                    let sweep = lx.flag()?;
                    let x = lx.number()? + ox;
                    let y = lx.number()? + oy;
                    arc_to_cubics(&mut out, (cx, cy), rx, ry, rot, large, sweep, (x, y));
                    (cx, cy) = (x, y);
                }
                b'Z' => {
                    out.push(Seg::Close);
                    (cx, cy) = (sx, sy);
                }
                _ => unreachable!(),
            }
            last_c = c_next;
            last_q = q_next;
            if out.len() > MAX_SEGMENTS {
                return Err(lx.err(format!("path has more than {MAX_SEGMENTS} segments")));
            }
        }
        if out.is_empty() {
            return Err(PathError { pos: 0, message: "path is empty".into() });
        }
        Ok(Path(out))
    }

    pub fn transform(&self, t: Affine) -> Path {
        let p = |x: f64, y: f64| t.apply(x, y);
        Path(
            self.0
                .iter()
                .map(|s| match *s {
                    Seg::Move(x, y) => {
                        let (x, y) = p(x, y);
                        Seg::Move(x, y)
                    }
                    Seg::Line(x, y) => {
                        let (x, y) = p(x, y);
                        Seg::Line(x, y)
                    }
                    Seg::Quad(x1, y1, x, y) => {
                        let ((x1, y1), (x, y)) = (p(x1, y1), p(x, y));
                        Seg::Quad(x1, y1, x, y)
                    }
                    Seg::Cubic(x1, y1, x2, y2, x, y) => {
                        let ((x1, y1), (x2, y2), (x, y)) = (p(x1, y1), p(x2, y2), p(x, y));
                        Seg::Cubic(x1, y1, x2, y2, x, y)
                    }
                    Seg::Close => Seg::Close,
                })
                .collect(),
        )
    }

    /// Bounding box of all points, control points included: `[min_x, min_y, max_x, max_y]`.
    pub fn bounds(&self) -> Option<[f64; 4]> {
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        let mut add = |x: f64, y: f64| {
            b[0] = b[0].min(x);
            b[1] = b[1].min(y);
            b[2] = b[2].max(x);
            b[3] = b[3].max(y);
        };
        for s in &self.0 {
            match *s {
                Seg::Move(x, y) | Seg::Line(x, y) => add(x, y),
                Seg::Quad(x1, y1, x, y) => {
                    add(x1, y1);
                    add(x, y);
                }
                Seg::Cubic(x1, y1, x2, y2, x, y) => {
                    add(x1, y1);
                    add(x2, y2);
                    add(x, y);
                }
                Seg::Close => {}
            }
        }
        b[0].is_finite().then_some(b)
    }

    /// Path data with absolute commands and numbers rounded to 0.01.
    pub fn to_svg(&self) -> String {
        let mut s = String::new();
        let n = |s: &mut String, v: f64| {
            let r = (v * 100.0).round() / 100.0;
            let r = if r == 0.0 { 0.0 } else { r };
            let _ = write!(s, " {r}");
        };
        for seg in &self.0 {
            match *seg {
                Seg::Move(x, y) => {
                    s.push('M');
                    n(&mut s, x);
                    n(&mut s, y);
                }
                Seg::Line(x, y) => {
                    s.push('L');
                    n(&mut s, x);
                    n(&mut s, y);
                }
                Seg::Quad(x1, y1, x, y) => {
                    s.push('Q');
                    for v in [x1, y1, x, y] {
                        n(&mut s, v);
                    }
                }
                Seg::Cubic(x1, y1, x2, y2, x, y) => {
                    s.push('C');
                    for v in [x1, y1, x2, y2, x, y] {
                        n(&mut s, v);
                    }
                }
                Seg::Close => s.push('Z'),
            }
            s.push(' ');
        }
        s.trim_end().to_string()
    }
}

/// Convert an SVG arc to cubic curves (SVG spec F.6 endpoint to center form).
#[allow(clippy::too_many_arguments)]
fn arc_to_cubics(out: &mut Vec<Seg>, from: (f64, f64), rx: f64, ry: f64, rot_deg: f64, large: bool, sweep: bool, to: (f64, f64)) {
    let (x1, y1) = from;
    let (x2, y2) = to;
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx < 1e-9 || ry < 1e-9 || (x1 - x2).abs() + (y1 - y2).abs() < 1e-12 {
        if (x1 - x2).abs() + (y1 - y2).abs() >= 1e-12 {
            out.push(Seg::Line(x2, y2));
        }
        return;
    }
    let phi = rot_deg.to_radians();
    let (sp, cp) = phi.sin_cos();
    let dx = (x1 - x2) / 2.0;
    let dy = (y1 - y2) / 2.0;
    let x1p = cp * dx + sp * dy;
    let y1p = -sp * dx + cp * dy;
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }
    let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut coef = if den == 0.0 { 0.0 } else { (num / den).max(0.0).sqrt() };
    if large == sweep {
        coef = -coef;
    }
    let cxp = coef * rx * y1p / ry;
    let cyp = -coef * ry * x1p / rx;
    let cx = cp * cxp - sp * cyp + (x1 + x2) / 2.0;
    let cy = sp * cxp + cp * cyp + (y1 + y2) / 2.0;
    let ang = |ux: f64, uy: f64, vx: f64, vy: f64| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let th1 = ang(1.0, 0.0, (x1p - cxp) / rx, (y1p - cyp) / ry);
    let mut dth = ang((x1p - cxp) / rx, (y1p - cyp) / ry, (-x1p - cxp) / rx, (-y1p - cyp) / ry);
    if !sweep && dth > 0.0 {
        dth -= std::f64::consts::TAU;
    } else if sweep && dth < 0.0 {
        dth += std::f64::consts::TAU;
    }
    let n = (dth.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = dth / n as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let pt = |t: f64| {
        let (st, ct) = t.sin_cos();
        let x = rx * ct;
        let y = ry * st;
        (cp * x - sp * y + cx, sp * x + cp * y + cy)
    };
    let deriv = |t: f64| {
        let (st, ct) = t.sin_cos();
        let x = -rx * st;
        let y = ry * ct;
        (cp * x - sp * y, sp * x + cp * y)
    };
    let mut t = th1;
    for i in 0..n {
        let t2 = t + step;
        let (p0x, p0y) = pt(t);
        let (p3x, p3y) = if i + 1 == n { (x2, y2) } else { pt(t2) };
        let (d0x, d0y) = deriv(t);
        let (d3x, d3y) = deriv(t2);
        out.push(Seg::Cubic(p0x + k * d0x, p0y + k * d0y, p3x - k * d3x, p3y - k * d3y, p3x, p3y));
        t = t2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_command_to_absolute() {
        let p = Path::parse("m10 10 h5 v5 l-5 0 z M0 0 C1 1 2 2 3 3 s4 4 5 5 Q1 1 2 2 t3 3 A5 5 0 0 1 20 20").unwrap();
        let svg = p.to_svg();
        assert!(svg.starts_with("M 10 10 L 15 10 L 15 15 L 10 15 Z M 0 0 C 1 1 2 2 3 3 C 4 4 7 7 8 8"), "{svg}");
        assert!(!svg.contains('A'));
        assert_eq!(p.bounds().map(|b| b[2] >= 20.0), Some(true));
    }

    #[test]
    fn packed_numbers_and_implicit_lines() {
        let p = Path::parse("M1.5.5L2-3 4,5").unwrap();
        assert_eq!(p.0, vec![Seg::Move(1.5, 0.5), Seg::Line(2.0, -3.0), Seg::Line(4.0, 5.0)]);
        let p = Path::parse("M0 0 1 1 2 2").unwrap();
        assert_eq!(p.0.len(), 3);
        let p = Path::parse("M0 0a1 1 0 1110 0").unwrap();
        assert!(matches!(p.0.last(), Some(Seg::Cubic(.., x, y)) if (*x - 10.0).abs() < 1e-9 && y.abs() < 1e-9));
    }

    #[test]
    fn rejects_bad_input_with_a_position() {
        assert_eq!(Path::parse("L1 1").unwrap_err().pos, 0);
        let e = Path::parse("M0 0 L1 x").unwrap_err();
        assert_eq!(e.pos, 8);
        assert!(Path::parse("M0 0 <script>").is_err());
        assert!(Path::parse("M0 0 L1e400 1").is_err());
        assert!(Path::parse("").is_err());
    }

    #[test]
    fn transforms_exactly() {
        let p = Path::parse("M0 0 L100 100").unwrap();
        let t = Affine::scale(0.5, 2.0).then(Affine::translate(10.0, 20.0));
        assert_eq!(p.transform(t).to_svg(), "M 10 20 L 60 220");
        let r = Path::parse("M10 0").unwrap().transform(Affine::rotate_deg(90.0));
        assert_eq!(r.to_svg(), "M 0 10");
    }
}
