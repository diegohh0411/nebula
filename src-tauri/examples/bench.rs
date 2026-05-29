//! End-to-end throughput benchmark.
//!
//! Decode only:
//!   NEBULA_BENCH_DIR=/path/to/images cargo run --release --example bench
//!
//! Decode + embed:
//!   NEBULA_BENCH_DIR=/path/to/images NEBULA_DATA_DIR=/path/to/app-data \
//!     cargo run --release --example bench

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

    // Embed stage is optional — enabled when NEBULA_DATA_DIR is set to the
    // directory that contains the `models/` subdirectory (the app's data dir).
    let embed_ctx = std::env::var("NEBULA_DATA_DIR").ok().map(|d| {
        let data_dir = PathBuf::from(d);
        let engine = nebula_lib::vision_engine::VisionEngine::new(
            data_dir.clone(),
            nebula_lib::pipeline::ComputePlacement::Cpu,
        );
        let manager = nebula_lib::models::ModelManager::new(data_dir);
        (engine, manager)
    });
    let spec = &nebula_lib::models::registry::SIGLIP_BASE;

    let mut decode = Stage::default();
    let mut embed = Stage::default();

    let wall = Instant::now();
    for path in &images {
        let t = Instant::now();
        let img = image::open(path).expect("decode");
        decode.add(t.elapsed().as_secs_f64() * 1000.0);

        if let Some((engine, manager)) = &embed_ctx {
            let t = Instant::now();
            match engine.embed_image(manager, &img, spec) {
                Ok(_) => embed.add(t.elapsed().as_secs_f64() * 1000.0),
                Err(e) => { eprintln!("embed failed (stopping embed stage): {e}"); break; }
            }
        }
    }
    let secs = wall.elapsed().as_secs_f64();

    println!("--- bench results ---");
    println!("images:        {}", images.len());
    println!("decode avg ms: {:.1}", decode.avg());
    if embed.count > 0 {
        println!("embed avg ms:  {:.1}", embed.avg());
    } else {
        println!("embed:         (set NEBULA_DATA_DIR to enable)");
    }
    println!("wall secs:     {:.2}", secs);
    println!("images/sec:    {:.2}", images.len() as f64 / secs);
}
