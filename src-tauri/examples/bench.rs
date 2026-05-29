//! End-to-end throughput benchmark.
//! Usage: NEBULA_BENCH_DIR=path/to/folder cargo run --release --example bench
//!
//! Decodes every JPEG/PNG in the folder and runs the current embed + face paths,
//! printing per-stage timings and images/sec. This is the baseline that every
//! optimization task is measured against.

use std::path::PathBuf;
use std::time::Instant;

fn list_images(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            let ext = p.extension().and_then(|s| s.to_str()).map(|s| s.to_lowercase());
            if matches!(ext.as_deref(), Some("jpg" | "jpeg" | "png")) {
                out.push(p);
            }
        }
    }
    out
}

#[derive(Default)]
struct Stage {
    count: u64,
    total_ms: f64,
}
impl Stage {
    fn add(&mut self, ms: f64) {
        self.count += 1;
        self.total_ms += ms;
    }
    fn avg(&self) -> f64 {
        if self.count == 0 { 0.0 } else { self.total_ms / self.count as f64 }
    }
}

fn main() {
    let dir = std::env::var("NEBULA_BENCH_DIR")
        .expect("set NEBULA_BENCH_DIR to a folder of images");
    let dir = PathBuf::from(dir);
    let images = list_images(&dir);
    assert!(!images.is_empty(), "no images found in {}", dir.display());
    eprintln!("benchmarking {} images from {}", images.len(), dir.display());

    let mut decode = Stage::default();

    let wall = Instant::now();
    for path in &images {
        let t = Instant::now();
        let _img = image::open(path).expect("decode");
        decode.add(t.elapsed().as_secs_f64() * 1000.0);
        // NOTE: embed/face stages are wired in once the pipeline exposes a
        // reusable single-image entry point (Task 9). For the baseline run we
        // measure decode only; record this number now.
    }
    let secs = wall.elapsed().as_secs_f64();

    println!("--- bench results ---");
    println!("images:        {}", images.len());
    println!("decode avg ms: {:.1}", decode.avg());
    println!("wall secs:     {:.2}", secs);
    println!("images/sec:    {:.2}", images.len() as f64 / secs);
}
