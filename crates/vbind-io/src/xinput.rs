//! Read XInput pads, used to confirm what games actually receive from a
//! virtual Xbox controller.

use serde::Serialize;
use windows::Win32::UI::Input::XboxController::{XINPUT_STATE, XInputGetState};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct XinputPad {
    pub slot: u32,
    pub packet: u32,
    pub buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub thumb_lx: i16,
    pub thumb_ly: i16,
    pub thumb_rx: i16,
    pub thumb_ry: i16,
}

/// State of XInput slot 0-3, or `None` when nothing is connected there.
pub fn read(slot: u32) -> Option<XinputPad> {
    let mut st = XINPUT_STATE::default();
    let rc = unsafe { XInputGetState(slot, &mut st) };
    if rc != 0 {
        return None;
    }
    let g = st.Gamepad;
    Some(XinputPad {
        slot,
        packet: st.dwPacketNumber,
        buttons: g.wButtons.0,
        left_trigger: g.bLeftTrigger,
        right_trigger: g.bRightTrigger,
        thumb_lx: g.sThumbLX,
        thumb_ly: g.sThumbLY,
        thumb_rx: g.sThumbRX,
        thumb_ry: g.sThumbRY,
    })
}

/// All connected XInput pads.
pub fn read_all() -> Vec<XinputPad> {
    (0..4).filter_map(read).collect()
}
