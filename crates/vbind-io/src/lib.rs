//! Windows device I/O for VBind: physical input through DirectInput, virtual
//! output through ViGEmBus (Xbox 360) and vJoy (joystick), and XInput reads
//! for verification.

#![cfg(windows)]

pub mod dinput;
pub mod vigem;
pub mod vjoy;
pub mod xinput;

/// USB IDs of vJoy's own virtual devices, which VBind never reads as input.
pub const VJOY_VID_PID: (u16, u16) = (0x1234, 0xBEAD);
