//! The VBind expression language.
//!
//! Every mapping is one short expression, for example
//! `max(hosas_drive(l, r), 0)` or `left.b1`. Expressions are parsed once,
//! resolved against a profile's device roles and variables, and then
//! evaluated every tick.
//!
//! Values are `f64`. Booleans are `1` and `0`; anything `>= 0.5` is true.

use crate::math;
use crate::state::{DeviceState, MAX_BUTTONS, MAX_HATS};
use std::fmt;

/// Byte offset into the source expression, used in error messages.
pub type Pos = usize;

#[derive(Clone, Debug, PartialEq)]
pub struct ExprError {
    pub pos: Pos,
    pub message: String,
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at character {})", self.message, self.pos + 1)
    }
}

impl std::error::Error for ExprError {}

fn err<T>(pos: Pos, message: impl Into<String>) -> Result<T, ExprError> {
    Err(ExprError { pos, message: message.into() })
}

// ---------------------------------------------------------------- lexer

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Op(&'static str),
    LParen,
    RParen,
    Comma,
    Dot,
    End,
}

const OPERATORS: [&str; 16] = [
    "<=", ">=", "==", "!=", "&&", "||", "<", ">", "+", "-", "*", "/", "%", "!", "?", ":",
];

fn lex(src: &str) -> Result<Vec<(Tok, Pos)>, ExprError> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let next_is_digit = bytes.get(i + 1).is_some_and(|b| b.is_ascii_digit());
        if c.is_ascii_digit() || (c == '.' && next_is_digit && !matches!(out.last(), Some((Tok::Ident(_), _)))) {
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                let mut j = i + 1;
                if j < bytes.len() && (bytes[j] == b'+' || bytes[j] == b'-') {
                    j += 1;
                }
                if j < bytes.len() && bytes[j].is_ascii_digit() {
                    i = j;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let text = &src[start..i];
            match text.parse::<f64>() {
                Ok(v) => out.push((Tok::Num(v), start)),
                Err(_) => return err(start, format!("'{text}' is not a number")),
            }
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            out.push((Tok::Ident(src[start..i].to_string()), start));
            continue;
        }
        match c {
            '(' => out.push((Tok::LParen, start)),
            ')' => out.push((Tok::RParen, start)),
            ',' => out.push((Tok::Comma, start)),
            '.' => out.push((Tok::Dot, start)),
            _ => {
                let rest = &src[i..];
                match OPERATORS.iter().find(|op| rest.starts_with(**op)) {
                    Some(op) => {
                        out.push((Tok::Op(op), start));
                        i += op.len();
                        continue;
                    }
                    None => return err(start, format!("unexpected character '{c}'")),
                }
            }
        }
        i += 1;
    }
    out.push((Tok::End, src.len()));
    Ok(out)
}

// ---------------------------------------------------------------- syntax tree

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

/// Unresolved syntax tree, straight from the parser.
#[derive(Clone, Debug, PartialEq)]
pub enum Ast {
    Num(f64),
    /// A name or dotted path such as `drive` or `left.hat.up`.
    Path(Vec<String>, Pos),
    Call(String, Vec<Ast>, Pos),
    Neg(Box<Ast>),
    Not(Box<Ast>),
    Bin(BinOp, Box<Ast>, Box<Ast>),
    /// `cond ? a : b`
    Cond(Box<Ast>, Box<Ast>, Box<Ast>),
}

struct Parser {
    toks: Vec<(Tok, Pos)>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.i].0
    }
    fn pos(&self) -> Pos {
        self.toks[self.i].1
    }
    fn bump(&mut self) -> Tok {
        let t = self.toks[self.i].0.clone();
        if self.i + 1 < self.toks.len() {
            self.i += 1;
        }
        t
    }
    fn is_op(&self, op: &str) -> bool {
        matches!(self.peek(), Tok::Op(o) if *o == op)
    }
    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s) if s == w)
    }

    fn expr(&mut self) -> Result<Ast, ExprError> {
        let cond = self.or()?;
        if self.is_op("?") {
            self.bump();
            let a = self.expr()?;
            if !self.is_op(":") {
                return err(self.pos(), "expected ':' in 'condition ? a : b'");
            }
            self.bump();
            let b = self.expr()?;
            return Ok(Ast::Cond(Box::new(cond), Box::new(a), Box::new(b)));
        }
        Ok(cond)
    }

    fn or(&mut self) -> Result<Ast, ExprError> {
        let mut lhs = self.and()?;
        while self.is_op("||") || self.is_word("or") {
            self.bump();
            let rhs = self.and()?;
            lhs = Ast::Bin(BinOp::Or, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn and(&mut self) -> Result<Ast, ExprError> {
        let mut lhs = self.not()?;
        while self.is_op("&&") || self.is_word("and") {
            self.bump();
            let rhs = self.not()?;
            lhs = Ast::Bin(BinOp::And, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn not(&mut self) -> Result<Ast, ExprError> {
        if self.is_op("!") || self.is_word("not") {
            self.bump();
            let inner = self.not()?;
            return Ok(Ast::Not(Box::new(inner)));
        }
        self.cmp()
    }

    fn cmp(&mut self) -> Result<Ast, ExprError> {
        let lhs = self.add()?;
        let op = match self.peek() {
            Tok::Op("<") => BinOp::Lt,
            Tok::Op("<=") => BinOp::Le,
            Tok::Op(">") => BinOp::Gt,
            Tok::Op(">=") => BinOp::Ge,
            Tok::Op("==") => BinOp::Eq,
            Tok::Op("!=") => BinOp::Ne,
            _ => return Ok(lhs),
        };
        self.bump();
        let rhs = self.add()?;
        Ok(Ast::Bin(op, Box::new(lhs), Box::new(rhs)))
    }

    fn add(&mut self) -> Result<Ast, ExprError> {
        let mut lhs = self.mul()?;
        loop {
            let op = match self.peek() {
                Tok::Op("+") => BinOp::Add,
                Tok::Op("-") => BinOp::Sub,
                _ => return Ok(lhs),
            };
            self.bump();
            let rhs = self.mul()?;
            lhs = Ast::Bin(op, Box::new(lhs), Box::new(rhs));
        }
    }

    fn mul(&mut self) -> Result<Ast, ExprError> {
        let mut lhs = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Op("*") => BinOp::Mul,
                Tok::Op("/") => BinOp::Div,
                Tok::Op("%") => BinOp::Rem,
                _ => return Ok(lhs),
            };
            self.bump();
            let rhs = self.unary()?;
            lhs = Ast::Bin(op, Box::new(lhs), Box::new(rhs));
        }
    }

    fn unary(&mut self) -> Result<Ast, ExprError> {
        if self.is_op("-") {
            self.bump();
            let inner = self.unary()?;
            return Ok(match inner {
                Ast::Num(v) => Ast::Num(-v),
                other => Ast::Neg(Box::new(other)),
            });
        }
        if self.is_op("+") {
            self.bump();
            return self.unary();
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Ast, ExprError> {
        let pos = self.pos();
        match self.bump() {
            Tok::Num(v) => Ok(Ast::Num(v)),
            Tok::LParen => {
                let e = self.expr()?;
                if *self.peek() != Tok::RParen {
                    return err(self.pos(), "expected ')'");
                }
                self.bump();
                Ok(e)
            }
            Tok::Ident(name) => {
                if name == "true" {
                    return Ok(Ast::Num(1.0));
                }
                if name == "false" {
                    return Ok(Ast::Num(0.0));
                }
                if *self.peek() == Tok::LParen {
                    self.bump();
                    let mut args = Vec::new();
                    if *self.peek() != Tok::RParen {
                        loop {
                            args.push(self.expr()?);
                            if *self.peek() == Tok::Comma {
                                self.bump();
                                continue;
                            }
                            break;
                        }
                    }
                    if *self.peek() != Tok::RParen {
                        return err(self.pos(), format!("expected ',' or ')' in call to {name}()"));
                    }
                    self.bump();
                    return Ok(Ast::Call(name, args, pos));
                }
                let mut path = vec![name];
                while *self.peek() == Tok::Dot {
                    self.bump();
                    match self.bump() {
                        Tok::Ident(seg) => path.push(seg),
                        Tok::Num(v) if v.fract() == 0.0 => path.push(format!("{v}")),
                        _ => return err(self.pos(), "expected a name after '.'"),
                    }
                }
                Ok(Ast::Path(path, pos))
            }
            Tok::End => err(pos, "expression ends too early"),
            other => err(pos, format!("unexpected {}", describe(&other))),
        }
    }
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Num(v) => format!("number {v}"),
        Tok::Ident(s) => format!("name '{s}'"),
        Tok::Op(o) => format!("'{o}'"),
        Tok::LParen => "'('".into(),
        Tok::RParen => "')'".into(),
        Tok::Comma => "','".into(),
        Tok::Dot => "'.'".into(),
        Tok::End => "end of expression".into(),
    }
}

/// Parse an expression into an unresolved syntax tree.
pub fn parse(src: &str) -> Result<Ast, ExprError> {
    if src.trim().is_empty() {
        return err(0, "expression is empty");
    }
    let mut p = Parser { toks: lex(src)?, i: 0 };
    let ast = p.expr()?;
    if *p.peek() != Tok::End {
        return err(p.pos(), format!("unexpected {} after the expression", describe(p.peek())));
    }
    Ok(ast)
}

// ---------------------------------------------------------------- input controls

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HatDir {
    Up,
    UpRight,
    Right,
    DownRight,
    Down,
    DownLeft,
    Left,
    UpLeft,
}

impl HatDir {
    pub const ALL: [HatDir; 8] = [
        HatDir::Up,
        HatDir::UpRight,
        HatDir::Right,
        HatDir::DownRight,
        HatDir::Down,
        HatDir::DownLeft,
        HatDir::Left,
        HatDir::UpLeft,
    ];

    pub fn name(self) -> &'static str {
        match self {
            HatDir::Up => "up",
            HatDir::UpRight => "up_right",
            HatDir::Right => "right",
            HatDir::DownRight => "down_right",
            HatDir::Down => "down",
            HatDir::DownLeft => "down_left",
            HatDir::Left => "left",
            HatDir::UpLeft => "up_left",
        }
    }

    pub fn from_name(s: &str) -> Option<HatDir> {
        HatDir::ALL.into_iter().find(|d| d.name() == s)
    }

    /// Angle in hundredths of a degree, clockwise from up.
    pub fn angle(self) -> u32 {
        (self as u32) * 4500
    }
}

/// A control on a physical device that an expression can read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum InputControl {
    /// Axis slot `0..8` (`x, y, z, rx, ry, rz, slider1, slider2`), `-1..=1`.
    Axis(u8),
    /// Button index `0..128`; written `b1..b128`.
    Button(u8),
    /// Hat `0..4` pointing in a direction (within 23 degrees); written `hat1.up`.
    HatDir(u8, HatDir),
    /// Hat horizontal component, `-1`, `0` or `1`.
    HatX(u8),
    /// Hat vertical component, `-1` (down), `0` or `1` (up).
    HatY(u8),
    /// `1` while the device is connected.
    Connected,
}

impl InputControl {
    /// Canonical name, as used in expressions after the role (`y`, `b1`, `hat1.up`).
    pub fn id(&self) -> String {
        match *self {
            InputControl::Axis(a) => crate::state::AXIS_NAMES[a as usize].to_string(),
            InputControl::Button(b) => format!("b{}", b as u16 + 1),
            InputControl::HatDir(h, d) => format!("hat{}.{}", h + 1, d.name()),
            InputControl::HatX(h) => format!("hat{}.x", h + 1),
            InputControl::HatY(h) => format!("hat{}.y", h + 1),
            InputControl::Connected => "connected".to_string(),
        }
    }

    /// Parse the part after the role: `["y"]`, `["b1"]`, `["hat", "up"]`.
    pub fn parse(segments: &[&str]) -> Result<InputControl, String> {
        let first = segments.first().copied().unwrap_or("");
        let rest = &segments[1.min(segments.len())..];
        if let Some(i) = crate::state::AXIS_NAMES.iter().position(|a| *a == first) {
            if !rest.is_empty() {
                return Err(format!("axis '{first}' has no '.{}'", rest.join(".")));
            }
            return Ok(InputControl::Axis(i as u8));
        }
        if first == "slider" {
            return Ok(InputControl::Axis(6));
        }
        if first == "connected" && rest.is_empty() {
            return Ok(InputControl::Connected);
        }
        if let Some(n) = first.strip_prefix('b').and_then(|n| n.parse::<usize>().ok()) {
            if !(1..=MAX_BUTTONS).contains(&n) {
                return Err(format!("button numbers run from b1 to b{MAX_BUTTONS}"));
            }
            if !rest.is_empty() {
                return Err(format!("button '{first}' has no '.{}'", rest.join(".")));
            }
            return Ok(InputControl::Button((n - 1) as u8));
        }
        if let Some(n) = first.strip_prefix("hat") {
            let idx = if n.is_empty() {
                1
            } else {
                n.parse::<usize>().map_err(|_| format!("unknown control '{first}'"))?
            };
            if !(1..=MAX_HATS).contains(&idx) {
                return Err(format!("hat numbers run from hat1 to hat{MAX_HATS}"));
            }
            let h = (idx - 1) as u8;
            return match rest {
                [] => Err(format!(
                    "'{first}' needs a direction, for example {first}.up, {first}.left or {first}.x"
                )),
                ["x"] => Ok(InputControl::HatX(h)),
                ["y"] => Ok(InputControl::HatY(h)),
                [d] => HatDir::from_name(d).map(|d| InputControl::HatDir(h, d)).ok_or_else(|| {
                    format!("hat direction '{d}' is not one of up, right, down, left, up_right, down_right, down_left, up_left, x, y")
                }),
                _ => Err(format!("unknown control '{}'", segments.join("."))),
            };
        }
        Err(format!(
            "unknown control '{}'. Use an axis (x, y, z, rx, ry, rz, slider1, slider2), a button (b1..b128) or a hat (hat1.up)",
            segments.join(".")
        ))
    }

    /// Current value of this control in `state` (or 0 when disconnected).
    pub fn read(&self, state: Option<&DeviceState>) -> f64 {
        let Some(s) = state else { return 0.0 };
        match *self {
            InputControl::Axis(a) => s.axes[a as usize],
            InputControl::Button(b) => bool_f(s.buttons[b as usize]),
            InputControl::HatDir(h, d) => bool_f(math::hat_matches(
                s.hats[h as usize],
                d.angle(),
                math::hat::DEFAULT_TOLERANCE,
            )),
            InputControl::HatX(h) => hat_component(s.hats[h as usize]).0,
            InputControl::HatY(h) => hat_component(s.hats[h as usize]).1,
            InputControl::Connected => 1.0,
        }
    }

    pub fn is_digital(&self) -> bool {
        !matches!(self, InputControl::Axis(_) | InputControl::HatX(_) | InputControl::HatY(_))
    }
}

/// Split a hat angle into (x, y) components of -1/0/1, counting diagonals as both.
fn hat_component(reading: Option<u32>) -> (f64, f64) {
    let Some(angle) = reading else { return (0.0, 0.0) };
    let near = |target: u32| math::hat_angle_distance(angle, target) <= 6750;
    let x = if near(math::hat::RIGHT) {
        1.0
    } else if near(math::hat::LEFT) {
        -1.0
    } else {
        0.0
    };
    let y = if near(math::hat::UP) {
        1.0
    } else if near(math::hat::DOWN) {
        -1.0
    } else {
        0.0
    };
    (x, y)
}

fn bool_f(b: bool) -> f64 {
    if b { 1.0 } else { 0.0 }
}

/// Truthiness used by logic operators and digital outputs.
pub fn truthy(v: f64) -> bool {
    v >= 0.5
}

/// Reference to one control on one device role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InputRef {
    pub role: usize,
    pub control: InputControl,
}

// ---------------------------------------------------------------- functions

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Func {
    Abs,
    Sign,
    Min,
    Max,
    Clamp,
    Deadzone,
    Deadzone1,
    Curve,
    HosasDrive,
    Lever,
    Centered,
    If,
    Pow,
    Sqrt,
    Round,
    Scale,
    Step,
    Between,
    Toggle,
    Taps,
}

/// Name, minimum and maximum argument counts, and a one-line description.
pub const FUNCTIONS: &[(Func, &str, usize, usize, &str)] = &[
    (Func::Abs, "abs", 1, 1, "abs(v): absolute value"),
    (Func::Sign, "sign", 1, 1, "sign(v): -1, 0 or 1"),
    (Func::Min, "min", 2, 16, "min(a, b, ...): smallest argument"),
    (Func::Max, "max", 2, 16, "max(a, b, ...): largest argument"),
    (Func::Clamp, "clamp", 3, 3, "clamp(v, lo, hi): limit v to lo..hi"),
    (Func::Deadzone, "deadzone", 2, 2, "deadzone(v, dz): zero within dz of center, rescale the rest (dz 0.06 = 6%)"),
    (Func::Deadzone1, "deadzone1", 2, 2, "deadzone1(v, dz): one-way deadzone for 0..1 levers"),
    (Func::Curve, "curve", 2, 4, "curve(v, power, weight=0.5, full=1): response curve; power 1 is linear, >1 softer center"),
    (Func::HosasDrive, "hosas_drive", 2, 3, "hosas_drive(l, r, preserve=1): two-stick thrust; both forward = stronger stick, one back scrubs, both back = average"),
    (Func::Lever, "lever", 1, 1, "lever(v): map an axis -1..1 to 0..1 (for throttles and triggers)"),
    (Func::Centered, "centered", 1, 1, "centered(v): map 0..1 to -1..1"),
    (Func::If, "if", 3, 3, "if(cond, a, b): a when cond is true, else b"),
    (Func::Pow, "pow", 2, 2, "pow(a, b): a to the power b"),
    (Func::Sqrt, "sqrt", 1, 1, "sqrt(v): square root"),
    (Func::Round, "round", 1, 1, "round(v): nearest whole number"),
    (Func::Scale, "scale", 5, 5, "scale(v, in_lo, in_hi, out_lo, out_hi): linear remap"),
    (Func::Step, "step", 2, 2, "step(v, t): 1 when v >= t, else 0"),
    (Func::Between, "between", 3, 3, "between(v, lo, hi): 1 when lo <= v <= hi"),
    (Func::Toggle, "toggle", 1, 1, "toggle(x): flips between 0 and 1 each time x becomes true"),
    (Func::Taps, "taps", 1, 3, "taps(x, n=2, ms=60): each time x becomes true, n quick taps (ms on, ms off), for keys a game wants double-tapped"),
];

fn lookup_func(name: &str) -> Option<(Func, usize, usize)> {
    FUNCTIONS
        .iter()
        .find(|(_, n, ..)| *n == name)
        .map(|(f, _, lo, hi, _)| (*f, *lo, *hi))
}

/// Closest candidate by edit distance, for "did you mean" hints.
pub fn suggest<'a>(name: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut best: Option<(&str, usize)> = None;
    for c in candidates {
        let d = edit_distance(name, c);
        if d <= 2.max(name.len() / 3) && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((c, d));
        }
    }
    best.map(|(c, _)| c)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}

// ---------------------------------------------------------------- resolved tree

/// Resolved expression, ready to evaluate.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Const(f64),
    Input(InputRef),
    Var(usize),
    Neg(Box<Node>),
    Not(Box<Node>),
    Bin(BinOp, Box<Node>, Box<Node>),
    Cond(Box<Node>, Box<Node>, Box<Node>),
    Call(Func, Vec<Node>),
    /// Stateful toggle with its state slot.
    Toggle(usize, Box<Node>),
    /// Timed taps with their state slot: input, tap count, tap length in ms.
    Taps(usize, Box<Node>, Box<Node>, Box<Node>),
}

/// Names an expression may refer to.
pub trait Scope {
    /// Index of a device role (`left`, `right`).
    fn role(&self, name: &str) -> Option<usize>;
    /// Index of a variable.
    fn var(&self, name: &str) -> Option<usize>;
    fn role_names(&self) -> Vec<&str>;
    fn var_names(&self) -> Vec<&str>;
}

/// Resolves syntax trees against a scope, allocating toggle state slots.
pub struct Resolver<'a, S: Scope> {
    pub scope: &'a S,
    pub toggle_slots: usize,
}

impl<'a, S: Scope> Resolver<'a, S> {
    pub fn new(scope: &'a S) -> Self {
        Self { scope, toggle_slots: 0 }
    }

    pub fn resolve(&mut self, ast: &Ast) -> Result<Node, ExprError> {
        Ok(match ast {
            Ast::Num(v) => Node::Const(*v),
            Ast::Neg(a) => Node::Neg(Box::new(self.resolve(a)?)),
            Ast::Not(a) => Node::Not(Box::new(self.resolve(a)?)),
            Ast::Bin(op, a, b) => Node::Bin(*op, Box::new(self.resolve(a)?), Box::new(self.resolve(b)?)),
            Ast::Cond(c, a, b) => Node::Cond(
                Box::new(self.resolve(c)?),
                Box::new(self.resolve(a)?),
                Box::new(self.resolve(b)?),
            ),
            Ast::Call(name, args, pos) => {
                let Some((func, lo, hi)) = lookup_func(name) else {
                    let hint = suggest(name, FUNCTIONS.iter().map(|f| f.1))
                        .map(|s| format!(". Did you mean {s}()?"))
                        .unwrap_or_default();
                    return err(*pos, format!("unknown function '{name}'{hint}"));
                };
                if args.len() < lo || args.len() > hi {
                    let want = if lo == hi { format!("{lo}") } else { format!("{lo} to {hi}") };
                    return err(*pos, format!("{name}() takes {want} arguments, got {}", args.len()));
                }
                let mut resolved = Vec::with_capacity(args.len());
                for a in args {
                    resolved.push(self.resolve(a)?);
                }
                if func == Func::Toggle {
                    let slot = self.toggle_slots;
                    self.toggle_slots += 1;
                    Node::Toggle(slot, Box::new(resolved.remove(0)))
                } else if func == Func::Taps {
                    let slot = self.toggle_slots;
                    self.toggle_slots += 1;
                    let mut it = resolved.into_iter();
                    let x = it.next().expect("one argument at least");
                    let n = it.next().unwrap_or(Node::Const(2.0));
                    let ms = it.next().unwrap_or(Node::Const(60.0));
                    Node::Taps(slot, Box::new(x), Box::new(n), Box::new(ms))
                } else {
                    Node::Call(func, resolved)
                }
            }
            Ast::Path(path, pos) => self.resolve_path(path, *pos)?,
        })
    }

    fn resolve_path(&self, path: &[String], pos: Pos) -> Result<Node, ExprError> {
        let head = path[0].as_str();
        if path.len() == 1 {
            if let Some(v) = self.scope.var(head) {
                return Ok(Node::Var(v));
            }
            if head == "pi" {
                return Ok(Node::Const(std::f64::consts::PI));
            }
            if self.scope.role(head).is_some() {
                return err(pos, format!("'{head}' is a device; name a control, for example {head}.y or {head}.b1"));
            }
            let mut names = self.scope.var_names();
            names.extend(self.scope.role_names());
            let hint = suggest(head, names).map(|s| format!(". Did you mean '{s}'?")).unwrap_or_default();
            return err(pos, format!("unknown name '{head}'{hint}"));
        }
        let Some(role) = self.scope.role(head) else {
            let hint = suggest(head, self.scope.role_names())
                .map(|s| format!(". Did you mean '{s}'?"))
                .unwrap_or_default();
            return err(pos, format!("unknown device '{head}'{hint}"));
        };
        let segs: Vec<&str> = path[1..].iter().map(String::as_str).collect();
        let control = InputControl::parse(&segs).map_err(|m| ExprError { pos, message: m })?;
        Ok(Node::Input(InputRef { role, control }))
    }
}

impl Node {
    /// Every input control and variable this node reads directly.
    pub fn collect_refs(&self, inputs: &mut Vec<InputRef>, vars: &mut Vec<usize>) {
        match self {
            Node::Const(_) => {}
            Node::Input(r) => {
                if !inputs.contains(r) {
                    inputs.push(*r);
                }
            }
            Node::Var(v) => {
                if !vars.contains(v) {
                    vars.push(*v);
                }
            }
            Node::Neg(a) | Node::Not(a) | Node::Toggle(_, a) => a.collect_refs(inputs, vars),
            Node::Taps(_, a, b, c) => {
                a.collect_refs(inputs, vars);
                b.collect_refs(inputs, vars);
                c.collect_refs(inputs, vars);
            }
            Node::Bin(_, a, b) => {
                a.collect_refs(inputs, vars);
                b.collect_refs(inputs, vars);
            }
            Node::Cond(c, a, b) => {
                c.collect_refs(inputs, vars);
                a.collect_refs(inputs, vars);
                b.collect_refs(inputs, vars);
            }
            Node::Call(_, args) => args.iter().for_each(|a| a.collect_refs(inputs, vars)),
        }
    }
}

/// Per-profile mutable evaluation state.
#[derive(Clone, Debug, Default)]
pub struct EvalState {
    /// Per stateful-function slot: (previous input was true, current toggle output)
    pub toggles: Vec<(bool, bool)>,
    /// Per slot: when `taps` last started, in seconds on the `now_s` clock.
    pub started: Vec<Option<f64>>,
    /// The current time in seconds, set by whoever runs the profile; timed
    /// functions measure from it.
    pub now_s: f64,
}

impl EvalState {
    pub fn new(toggle_slots: usize) -> Self {
        Self { toggles: vec![(false, false); toggle_slots], started: vec![None; toggle_slots], now_s: 0.0 }
    }
}

/// Evaluation inputs: device states by role, and already-computed variables.
pub struct Env<'a> {
    pub devices: &'a [Option<&'a DeviceState>],
    pub vars: &'a [f64],
}

pub fn eval(node: &Node, env: &Env, st: &mut EvalState) -> f64 {
    let v = match node {
        Node::Const(v) => *v,
        Node::Input(r) => r.control.read(env.devices.get(r.role).copied().flatten()),
        Node::Var(i) => env.vars.get(*i).copied().unwrap_or(0.0),
        Node::Neg(a) => -eval(a, env, st),
        Node::Not(a) => bool_f(!truthy(eval(a, env, st))),
        Node::Bin(op, a, b) => {
            let x = eval(a, env, st);
            match op {
                BinOp::And => return bool_f(truthy(x) && truthy(eval(b, env, st))),
                BinOp::Or => return bool_f(truthy(x) || truthy(eval(b, env, st))),
                _ => {}
            }
            let y = eval(b, env, st);
            match op {
                BinOp::Add => x + y,
                BinOp::Sub => x - y,
                BinOp::Mul => x * y,
                BinOp::Div => {
                    if y == 0.0 {
                        0.0
                    } else {
                        x / y
                    }
                }
                BinOp::Rem => {
                    if y == 0.0 {
                        0.0
                    } else {
                        x % y
                    }
                }
                BinOp::Lt => bool_f(x < y),
                BinOp::Le => bool_f(x <= y),
                BinOp::Gt => bool_f(x > y),
                BinOp::Ge => bool_f(x >= y),
                BinOp::Eq => bool_f((x - y).abs() < 1e-9),
                BinOp::Ne => bool_f((x - y).abs() >= 1e-9),
                BinOp::And | BinOp::Or => unreachable!(),
            }
        }
        Node::Cond(c, a, b) => {
            if truthy(eval(c, env, st)) {
                eval(a, env, st)
            } else {
                eval(b, env, st)
            }
        }
        Node::Toggle(slot, a) => {
            let now = truthy(eval(a, env, st));
            let entry = &mut st.toggles[*slot];
            if now && !entry.0 {
                entry.1 = !entry.1;
            }
            entry.0 = now;
            bool_f(entry.1)
        }
        Node::Taps(slot, a, n, ms) => {
            let pressed = truthy(eval(a, env, st));
            let n = eval(n, env, st).round().clamp(1.0, 10.0);
            let len = eval(ms, env, st).clamp(10.0, 1000.0) / 1000.0;
            if pressed && !st.toggles[*slot].0 {
                st.started[*slot] = Some(st.now_s);
            }
            st.toggles[*slot].0 = pressed;
            let Some(start) = st.started[*slot] else { return 0.0 };
            let t = st.now_s - start;
            // on for `len`, off for `len`, n times
            let k = (t / len).floor();
            bool_f(t >= 0.0 && k < n * 2.0 && (k as i64) % 2 == 0)
        }
        Node::Call(f, args) => {
            let mut a = [0.0f64; 16];
            let n = args.len().min(16);
            if *f == Func::If {
                return if truthy(eval(&args[0], env, st)) {
                    eval(&args[1], env, st)
                } else {
                    eval(&args[2], env, st)
                };
            }
            for (slot, arg) in a.iter_mut().zip(args.iter()) {
                *slot = eval(arg, env, st);
            }
            call(*f, &a[..n])
        }
    };
    if v.is_finite() { v } else { 0.0 }
}

fn call(f: Func, a: &[f64]) -> f64 {
    match f {
        Func::Abs => a[0].abs(),
        Func::Sign => {
            if a[0] > 0.0 {
                1.0
            } else if a[0] < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        Func::Min => a.iter().copied().fold(f64::INFINITY, f64::min),
        Func::Max => a.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        Func::Clamp => math::clamp(a[0], a[1].min(a[2]), a[1].max(a[2])),
        Func::Deadzone => math::deadzone(a[0], a[1]),
        Func::Deadzone1 => math::deadzone_one_way(a[0], a[1]),
        Func::Curve => math::curve(
            a[0],
            a[1],
            a.get(2).copied().unwrap_or(0.5),
            a.get(3).copied().unwrap_or(1.0),
        ),
        Func::HosasDrive => math::hosas_drive(a[0], a[1], a.get(2).is_none_or(|p| truthy(*p))),
        Func::Lever => math::clamp((a[0] + 1.0) / 2.0, 0.0, 1.0),
        Func::Centered => math::clamp(a[0] * 2.0 - 1.0, -1.0, 1.0),
        Func::Pow => a[0].powf(a[1]),
        Func::Sqrt => a[0].max(0.0).sqrt(),
        Func::Round => a[0].round(),
        Func::Scale => {
            let span = a[2] - a[1];
            if span == 0.0 {
                a[3]
            } else {
                a[3] + (a[0] - a[1]) / span * (a[4] - a[3])
            }
        }
        Func::Step => bool_f(a[0] >= a[1]),
        Func::Between => bool_f(a[0] >= a[1] && a[0] <= a[2]),
        Func::If | Func::Toggle | Func::Taps => unreachable!("handled in eval"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestScope;
    impl Scope for TestScope {
        fn role(&self, name: &str) -> Option<usize> {
            ["left", "right"].iter().position(|r| *r == name)
        }
        fn var(&self, name: &str) -> Option<usize> {
            ["l", "r"].iter().position(|r| *r == name)
        }
        fn role_names(&self) -> Vec<&str> {
            vec!["left", "right"]
        }
        fn var_names(&self) -> Vec<&str> {
            vec!["l", "r"]
        }
    }

    fn compile(src: &str) -> Result<Node, ExprError> {
        let ast = parse(src)?;
        Resolver::new(&TestScope).resolve(&ast)
    }

    fn run(src: &str, left: &DeviceState, vars: &[f64]) -> f64 {
        let node = compile(src).unwrap();
        let devices = [Some(left), None];
        let env = Env { devices: &devices, vars };
        eval(&node, &env, &mut EvalState::new(4))
    }

    #[test]
    fn arithmetic_and_precedence() {
        let s = DeviceState::default();
        assert_eq!(run("1 + 2 * 3", &s, &[]), 7.0);
        assert_eq!(run("(1 + 2) * 3", &s, &[]), 9.0);
        assert_eq!(run("-2 * -3", &s, &[]), 6.0);
        assert_eq!(run("10 % 4", &s, &[]), 2.0);
        assert_eq!(run("1 / 0", &s, &[]), 0.0);
        assert_eq!(run(".5 + 0.25", &s, &[]), 0.75);
        assert_eq!(run("1 < 2 and 3 >= 3", &s, &[]), 1.0);
        assert_eq!(run("not (1 > 2) && true", &s, &[]), 1.0);
        assert_eq!(run("0 or false", &s, &[]), 0.0);
        assert_eq!(run("1 > 0 ? 5 : 6", &s, &[]), 5.0);
    }

    #[test]
    fn reads_inputs_and_vars() {
        let mut s = DeviceState::default();
        s.axes[1] = -0.5;
        s.buttons[0] = true;
        s.hats[0] = Some(9000);
        assert_eq!(run("-left.y", &s, &[]), 0.5);
        assert_eq!(run("left.b1", &s, &[]), 1.0);
        assert_eq!(run("left.b2", &s, &[]), 0.0);
        assert_eq!(run("left.hat.right", &s, &[]), 1.0);
        assert_eq!(run("left.hat1.up", &s, &[]), 0.0);
        assert_eq!(run("left.hat.x", &s, &[]), 1.0);
        assert_eq!(run("right.b1", &s, &[]), 0.0);
        assert_eq!(run("right.connected", &s, &[]), 0.0);
        assert_eq!(run("left.connected", &s, &[]), 1.0);
        assert_eq!(run("l + r", &s, &[0.25, 0.5]), 0.75);
    }

    #[test]
    fn functions() {
        let s = DeviceState::default();
        assert_eq!(run("max(-0.25, 0)", &s, &[]), 0.0);
        assert_eq!(run("max(1, 4, 2)", &s, &[]), 4.0);
        assert_eq!(run("clamp(3, -1, 1)", &s, &[]), 1.0);
        assert_eq!(run("hosas_drive(-0.25, 1)", &s, &[]), 0.75);
        assert_eq!(run("hosas_drive(1, 0, 0)", &s, &[]), 0.5);
        assert_eq!(run("lever(0)", &s, &[]), 0.5);
        assert_eq!(run("if(1, 2, 3)", &s, &[]), 2.0);
        assert_eq!(run("scale(0.5, 0, 1, 10, 20)", &s, &[]), 15.0);
    }

    #[test]
    fn toggle_flips_on_rising_edge() {
        let node = compile("toggle(left.b1)").unwrap();
        let mut st = EvalState::new(1);
        let mut s = DeviceState::default();
        let mut step = |pressed: bool, st: &mut EvalState| {
            s.buttons[0] = pressed;
            let devices = [Some(&s), None];
            eval(&node, &Env { devices: &devices, vars: &[] }, st)
        };
        assert_eq!(step(false, &mut st), 0.0);
        assert_eq!(step(true, &mut st), 1.0);
        assert_eq!(step(true, &mut st), 1.0);
        assert_eq!(step(false, &mut st), 1.0);
        assert_eq!(step(true, &mut st), 0.0);
    }

    #[test]
    fn taps_play_out_after_a_press() {
        let node = compile("taps(left.b1, 2, 50)").unwrap();
        let mut st = EvalState::new(1);
        let mut s = DeviceState::default();
        let mut at = |ms: f64, pressed: bool, st: &mut EvalState| {
            s.buttons[0] = pressed;
            st.now_s = ms / 1000.0;
            let devices = [Some(&s), None];
            eval(&node, &Env { devices: &devices, vars: &[] }, st)
        };
        assert_eq!(at(0.0, false, &mut st), 0.0);
        // pressed at 100 ms and released quickly: on, off, on, then done
        let seen: Vec<f64> = [(100.0, true), (120.0, false), (160.0, false), (210.0, false), (260.0, false), (400.0, false)]
            .iter()
            .map(|&(t, p)| at(t, p, &mut st))
            .collect();
        assert_eq!(seen, vec![1.0, 1.0, 0.0, 1.0, 0.0, 0.0]);
        // holding the button does not repeat
        assert_eq!(at(500.0, true, &mut st), 1.0);
        assert_eq!(at(900.0, true, &mut st), 0.0);
    }

    #[test]
    fn helpful_errors() {
        let e = compile("hosas_drv(l, r)").unwrap_err();
        assert!(e.message.contains("Did you mean hosas_drive()"), "{}", e.message);
        let e = compile("lfet.y").unwrap_err();
        assert!(e.message.contains("Did you mean 'left'"), "{}", e.message);
        let e = compile("left.hat").unwrap_err();
        assert!(e.message.contains("needs a direction"), "{}", e.message);
        let e = compile("left.b200").unwrap_err();
        assert!(e.message.contains("b1 to b128"), "{}", e.message);
        let e = compile("max(1)").unwrap_err();
        assert!(e.message.contains("takes 2 to 16"), "{}", e.message);
        let e = compile("1 +").unwrap_err();
        assert!(e.message.contains("ends too early"), "{}", e.message);
        let e = compile("left").unwrap_err();
        assert!(e.message.contains("is a device"), "{}", e.message);
        assert!(parse("1 2").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn collects_dependencies() {
        let node = compile("max(hosas_drive(l, -right.y), left.b3)").unwrap();
        let (mut inputs, mut vars) = (vec![], vec![]);
        node.collect_refs(&mut inputs, &mut vars);
        assert_eq!(vars, vec![0]);
        assert_eq!(inputs.len(), 2);
        assert_eq!(inputs[0], InputRef { role: 1, control: InputControl::Axis(1) });
        assert_eq!(inputs[1].control.id(), "b3");
    }

    #[test]
    fn control_ids_round_trip() {
        for id in ["x", "slider2", "b1", "b128", "hat1.up", "hat4.down_left", "hat2.x", "connected"] {
            let segs: Vec<&str> = id.split('.').collect();
            assert_eq!(InputControl::parse(&segs).unwrap().id(), id);
        }
        assert_eq!(InputControl::parse(&["hat", "left"]).unwrap().id(), "hat1.left");
    }
}
