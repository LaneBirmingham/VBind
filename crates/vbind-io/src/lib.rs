//! Windows device I/O for VBind: physical input through DirectInput, virtual
//! output through ViGEmBus (Xbox 360), vJoy (joystick) and shared memory (game
//! integrations), Steam game lookup, and XInput reads for verification.

#![cfg(windows)]

pub mod dinput;
pub mod sendinput;
pub mod shm;
pub mod steam;
pub mod vigem;
pub mod vjoy;
pub mod xinput;

/// USB IDs of vJoy's own virtual devices, which VBind never reads as input.
pub const VJOY_VID_PID: (u16, u16) = (0x1234, 0xBEAD);
