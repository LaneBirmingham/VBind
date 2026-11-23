//! Hardware probe: list devices, print a few polls, and optionally plug in a
//! virtual Xbox pad and read it back through XInput.
//!
//! cargo run -p vbind-io --example probe [-- --xbox]

use std::time::{Duration, Instant};
use vbind_io::{dinput::DirectInput, vigem::Vigem, xinput};

fn main() {
    let mut di = DirectInput::new().expect("DirectInput");
    for d in di.devices() {
        println!(
            "{}  {:04X}:{:04X}  {:<32} axes={:?} buttons={} hats={}",
            d.instance,
            d.vid.unwrap_or(0),
            d.pid.unwrap_or(0),
            d.name,
            vbind_core::state::AXIS_NAMES
                .iter()
                .zip(d.caps.axes)
                .filter(|(_, p)| *p)
                .map(|(n, _)| *n)
                .collect::<Vec<_>>(),
            d.caps.button_count,
            d.caps.hat_count
        );
    }
    let start = Instant::now();
    let mut polls = 0u32;
    while start.elapsed() < Duration::from_millis(500) {
        di.poll(|_, _| {});
        polls += 1;
        std::thread::sleep(Duration::from_millis(2));
    }
    println!("polled all devices {polls} times in 500 ms");
    di.poll(|info, st| {
        if let Some(s) = st {
            let pressed: Vec<usize> = s.buttons.iter().enumerate().filter(|(_, b)| **b).map(|(i, _)| i + 1).collect();
            println!("{:<32} axes={:?} hats={:?} pressed={:?}", info.name, s.axes.map(|a| (a * 100.0).round() / 100.0), s.hats, pressed);
        }
    });

    if std::env::args().any(|a| a == "--xbox") {
        let before = xinput::read_all();
        let bus = Vigem::connect().expect("ViGEm");
        let mut pad = bus.plug_xbox360().expect("plug");
        let report = vbind_core::output::XboxReport { buttons: 0x1000, right_trigger: 200, thumb_lx: -12000, ..Default::default() };
        pad.update(&report).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        println!("user index: {:?}", pad.user_index());
        println!("xinput before: {before:?}");
        println!("xinput now:    {:?}", xinput::read_all());
        di.refresh();
        println!("DirectInput devices with the virtual pad:");
        for d in di.devices() {
            println!("  {} {:04X}:{:04X} {}", d.instance, d.vid.unwrap_or(0), d.pid.unwrap_or(0), d.name);
        }
    }
}
