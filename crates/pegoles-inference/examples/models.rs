//! Model store CLI (the same code paths the app uses).
//!
//! ```sh
//! cargo run -p pegoles-inference --example models -- list
//! cargo run -p pegoles-inference --example models -- install mai-ui-2b-6bit
//! cargo run -p pegoles-inference --example models -- verify mai-ui-2b-6bit
//! cargo run -p pegoles-inference --example models -- remove mai-ui-2b-6bit
//! cargo run -p pegoles-inference --example models -- hw
//! ```
//! `PEGOLES_DATA_DIR` overrides the data directory (default: the app's own,
//! `~/Library/Application Support/Pegoles` or `%LOCALAPPDATA%\Pegoles`).

use std::path::PathBuf;
use std::time::Instant;

use pegoles_inference::download::HttpsFetcher;
use pegoles_inference::{hardware, models_dir, Catalog, ModelStore};

fn data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("PEGOLES_DATA_DIR") {
        return PathBuf::from(p);
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(local).join("Pegoles");
    }
    PathBuf::from(std::env::var("HOME").expect("HOME")).join("Library/Application Support/Pegoles")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let catalog = Catalog::builtin();
    let store = ModelStore::new(models_dir(&data_dir()));
    let spec = |id: &str| {
        catalog.get(id).cloned().unwrap_or_else(|| {
            eprintln!("unknown model {id}");
            std::process::exit(2)
        })
    };
    match args.first().map(String::as_str) {
        Some("list") => {
            for m in &catalog.models {
                println!(
                    "{:<20} {:>6} {:>8.2} GB  {:?}{}",
                    m.id,
                    m.quantization,
                    m.total_bytes() as f64 / 1e9,
                    store.state(m),
                    if m.id == catalog.default_model {
                        "  (default)"
                    } else {
                        ""
                    }
                );
            }
        }
        Some("install") => {
            let s = spec(&args[1]);
            let t = Instant::now();
            let mut last = Instant::now();
            let res = store.install(
                &s,
                &HttpsFetcher::default(),
                &mut |p| {
                    if last.elapsed().as_secs() >= 5 {
                        last = Instant::now();
                        eprintln!(
                            "{:?} {:.1}% ({} / {})",
                            p.phase,
                            p.done_bytes as f64 * 100.0 / p.total_bytes.max(1) as f64,
                            p.done_bytes,
                            p.total_bytes
                        );
                    }
                },
                &|| false,
            );
            match res {
                Ok(v) => println!(
                    "installed {} in {:.1}s (verify {} ms) at {}",
                    s.id,
                    t.elapsed().as_secs_f64(),
                    v.verify_ms,
                    v.dir.display()
                ),
                Err(e) => {
                    eprintln!("install failed: {e}");
                    std::process::exit(1)
                }
            }
        }
        Some("verify") => match store.verify(&spec(&args[1])) {
            Ok(v) => println!("ok ({} ms)", v.verify_ms),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1)
            }
        },
        Some("import") => {
            let s = spec(&args[1]);
            match store.import_local(&s, &PathBuf::from(&args[2])) {
                Ok(v) => println!("imported {} ({} ms verify)", s.id, v.verify_ms),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1)
                }
            }
        }
        Some("remove") => {
            store.remove(&args[1]).expect("remove");
            println!("removed {}", args[1]);
        }
        Some("hw") => {
            println!("{:#?}", hardware::detect());
            println!("{:#?}", hardware::system_memory());
            #[cfg(windows)]
            println!("{:#?}", hardware::windows_acceleration());
        }
        _ => eprintln!(
            "usage: models list|install <id>|verify <id>|import <id> <dir>|remove <id>|hw"
        ),
    }
}
