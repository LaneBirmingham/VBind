use std::time::{Duration, Instant};
fn main() {
    let mut di = vbind_io::dinput::DirectInput::new().unwrap();
    // 1. poll cost
    let t = Instant::now();
    for _ in 0..200 { di.poll(|_, _| {}); }
    println!("poll all devices: {:.3} ms each", t.elapsed().as_secs_f64() * 1000.0 / 200.0);
    // 2. sleep accuracy
    for us in [500u64, 1000, 2000, 3700] {
        let t = Instant::now();
        for _ in 0..100 { std::thread::sleep(Duration::from_micros(us)); }
        println!("sleep {us}us: {:.3} ms actual", t.elapsed().as_secs_f64() * 1000.0 / 100.0);
    }
    // 3. deadline loop at 500 Hz with polling
    let period = Duration::from_secs_f64(1.0 / 500.0);
    let mut next = Instant::now();
    let t = Instant::now();
    let mut n = 0;
    while t.elapsed() < Duration::from_secs(1) {
        di.poll(|_, _| {});
        n += 1;
        next += period;
        let now = Instant::now();
        if next <= now { next = now; } else { std::thread::sleep(next - now); }
    }
    println!("deadline loop: {n} Hz");
}
