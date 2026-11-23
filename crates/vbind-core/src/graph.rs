//! The mapping graph the UI draws: input controls on the left, variables
//! (mixers) in the middle, output controls on the right.

use crate::expr::{InputControl, InputRef};
use crate::output::{ControlKind, OutputKind};
use crate::profile::{CompiledProfile, Profile};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GraphInput {
    /// Control id after the role, for example `y`, `b1` or `hat1.up`.
    pub control: String,
    pub label: String,
    pub digital: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GraphRole {
    pub role: String,
    pub label: String,
    /// Controls the profile reads, in device order.
    pub controls: Vec<GraphInput>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GraphRef {
    pub role: String,
    pub control: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GraphVar {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub expr: String,
    /// Column: 1 reads only inputs, 2 reads a column-1 variable, and so on.
    pub depth: u32,
    pub inputs: Vec<GraphRef>,
    pub vars: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GraphOutputControl {
    /// `pad.rt`
    pub key: String,
    /// `rt`
    pub control: String,
    /// Default control name, for example "Right trigger".
    pub label: String,
    /// What it does in the game, from the profile's labels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    pub kind: ControlKind,
    pub expr: String,
    pub inputs: Vec<GraphRef>,
    pub vars: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GraphOutput {
    pub name: String,
    pub kind: OutputKind,
    pub label: String,
    pub controls: Vec<GraphOutputControl>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Graph {
    pub profile: String,
    pub roles: Vec<GraphRole>,
    pub vars: Vec<GraphVar>,
    pub outputs: Vec<GraphOutput>,
}

/// Default name for an input control, used when no layout card labels it.
pub fn default_input_label(c: &InputControl) -> String {
    match *c {
        InputControl::Axis(a) => match a {
            0 => "Stick X (left/right)".into(),
            1 => "Stick Y (forward/back)".into(),
            2 => "Z axis".into(),
            3 => "X rotation".into(),
            4 => "Y rotation".into(),
            5 => "Twist (Z rotation)".into(),
            6 => "Slider 1".into(),
            _ => "Slider 2".into(),
        },
        InputControl::Button(b) => format!("Button {}", b as u16 + 1),
        InputControl::HatDir(h, d) => format!("Hat {} {}", h + 1, d.name().replace('_', "-")),
        InputControl::HatX(h) => format!("Hat {} left/right", h + 1),
        InputControl::HatY(h) => format!("Hat {} up/down", h + 1),
        InputControl::Connected => "Connected".into(),
    }
}

/// Build the graph. `label` may name controls from a device layout card;
/// returning `None` falls back to generic names.
pub fn build_graph(
    profile: &Profile,
    compiled: &CompiledProfile,
    label: &dyn Fn(usize, &InputControl) -> Option<String>,
) -> Graph {
    let gref = |r: &InputRef| GraphRef { role: compiled.roles[r.role].clone(), control: r.control.id() };

    let mut roles: Vec<GraphRole> = compiled
        .roles
        .iter()
        .map(|name| GraphRole {
            role: name.clone(),
            label: profile.devices.get(name).and_then(|d| d.label.clone()).unwrap_or_else(|| name.clone()),
            controls: Vec::new(),
        })
        .collect();
    for r in compiled.used_inputs() {
        let text = label(r.role, &r.control).unwrap_or_else(|| default_input_label(&r.control));
        let text = match r.control {
            // a layout names the hat; add the direction
            InputControl::HatDir(_, d) if !text.to_lowercase().contains(d.name()) => format!("{text} {}", d.name().replace('_', "-")),
            _ => text,
        };
        roles[r.role].controls.push(GraphInput { control: r.control.id(), label: text, digital: r.control.is_digital() });
    }

    let mut depth = vec![0u32; compiled.vars.len()];
    for &i in &compiled.var_order {
        depth[i] = 1 + compiled.vars[i].vars.iter().map(|d| depth[*d]).max().unwrap_or(0);
    }
    let vars = compiled
        .vars
        .iter()
        .enumerate()
        .map(|(i, v)| GraphVar {
            name: v.name.clone(),
            label: v.label.clone(),
            expr: v.expr.clone(),
            depth: depth[i],
            inputs: v.inputs.iter().map(gref).collect(),
            vars: v.vars.iter().map(|d| compiled.vars[*d].name.clone()).collect(),
        })
        .collect();

    let outputs = compiled
        .outputs
        .iter()
        .enumerate()
        .map(|(oi, o)| {
            let mut controls: Vec<(u32, GraphOutputControl)> = compiled
                .mappings
                .iter()
                .filter(|m| m.output == oi)
                .map(|m| {
                    (
                        m.control.order(),
                        GraphOutputControl {
                            key: m.key.clone(),
                            control: m.control.id(),
                            label: m.control.default_label(),
                            action: profile.label_for(&m.key).map(str::to_string),
                            kind: m.control.kind(),
                            expr: m.expr.clone(),
                            inputs: m.inputs.iter().map(gref).collect(),
                            vars: m.vars.iter().map(|d| compiled.vars[*d].name.clone()).collect(),
                        },
                    )
                })
                .collect();
            controls.sort_by_key(|c| c.0);
            GraphOutput {
                name: o.name.clone(),
                kind: o.kind,
                label: o.label.clone().unwrap_or_else(|| match o.kind {
                    OutputKind::Xbox360 => "Virtual Xbox 360".into(),
                    OutputKind::Vjoy => format!("Virtual stick (vJoy {})", o.vjoy_id),
                }),
                controls: controls.into_iter().map(|c| c.1).collect(),
            }
        })
        .collect();

    Graph { profile: compiled.id.clone(), roles, vars, outputs }
}
