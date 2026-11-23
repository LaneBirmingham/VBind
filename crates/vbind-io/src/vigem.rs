//! Virtual Xbox 360 controllers through the ViGEmBus driver.

use std::sync::Arc;
use vbind_core::output::XboxReport;
use vigem_client::{Client, TargetId, XButtons, XGamepad, Xbox360Wired};

/// A connection to the ViGEmBus driver, shared by all virtual pads.
#[derive(Clone)]
pub struct Vigem {
    client: Arc<Client>,
}

impl Vigem {
    /// Connect to the driver. Fails when ViGEmBus is not installed.
    pub fn connect() -> Result<Vigem, String> {
        Client::connect()
            .map(|c| Vigem { client: Arc::new(c) })
            .map_err(|e| format!("ViGEmBus driver not reachable ({e:?}). Install ViGEmBus 1.22 from https://github.com/nefarius/ViGEmBus/releases"))
    }

    /// Plug in a new virtual pad and wait until Windows has it.
    pub fn plug_xbox360(&self) -> Result<XboxPad, String> {
        let mut target = Xbox360Wired::new(self.client.clone(), TargetId::XBOX360_WIRED);
        target.plugin().map_err(|e| format!("could not plug in the virtual Xbox pad: {e:?}"))?;
        target.wait_ready().map_err(|e| format!("virtual Xbox pad did not become ready: {e:?}"))?;
        let mut pad = XboxPad { target, last: None };
        pad.update(&XboxReport::default())?;
        Ok(pad)
    }
}

/// A plugged-in virtual Xbox 360 pad. Unplugged when dropped.
pub struct XboxPad {
    target: Xbox360Wired<Arc<Client>>,
    last: Option<XboxReport>,
}

impl XboxPad {
    /// Send a report. Identical consecutive reports are skipped.
    pub fn update(&mut self, r: &XboxReport) -> Result<(), String> {
        if self.last.as_ref() == Some(r) {
            return Ok(());
        }
        let g = XGamepad {
            buttons: XButtons { raw: r.buttons },
            left_trigger: r.left_trigger,
            right_trigger: r.right_trigger,
            thumb_lx: r.thumb_lx,
            thumb_ly: r.thumb_ly,
            thumb_rx: r.thumb_rx,
            thumb_ry: r.thumb_ry,
        };
        self.target.update(&g).map_err(|e| format!("virtual Xbox pad update failed: {e:?}"))?;
        self.last = Some(*r);
        Ok(())
    }

    /// XInput slot (0-3) Windows gave this pad, when known.
    pub fn user_index(&mut self) -> Option<u32> {
        self.target.get_user_index().ok()
    }
}

impl Drop for XboxPad {
    fn drop(&mut self) {
        let _ = self.update(&XboxReport::default());
        let _ = self.target.unplug();
    }
}
