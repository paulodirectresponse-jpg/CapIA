use s4_wasm_core::*;
fn main() {
    let mut tl = Timeline::build(50, 200, 12345); // 10_000 clips
    let t = std::time::Instant::now();
    let (h, commits) = run_ops(&mut tl, 99, 200_000);
    let dt = t.elapsed();
    println!("native clips={} hash={} commits={} time_ms={:.1} us_per_op={:.3}", tl.clips.len(), h, commits, dt.as_secs_f64()*1e3, dt.as_secs_f64()*1e6/200_000.0);
}
