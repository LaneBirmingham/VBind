//! Platform-independent core of VBind: the device model, the expression
//! language, profiles, device layouts and pictures, and the mapping graph.

pub mod drawing;
pub mod expr;
pub mod graph;
pub mod integration;
pub mod layout;
pub mod math;
pub mod output;
pub mod path;
pub mod profile;
pub mod resolve;
pub mod state;

pub use profile::{CompiledProfile, Diagnostic, Profile};
