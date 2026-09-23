#[cfg(not(target_arch = "wasm32"))]
use std::env;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

#[cfg(not(target_arch = "wasm32"))]
use valheim_backup_cheat_scanner::{
    scan_archives, scan_character_timeline, validate_character_oracle, validate_oracle,
    validate_oracle_fixture_file, write_reports_with_characters,
};

#[cfg(not(target_arch = "wasm32"))]
fn usage() -> ! {
    eprintln!("usage: valheim-backup-cheat-scanner [--archive-dir DIR] [--output-dir DIR] [--prefab-names FILE] [--prefab-biomes FILE] [--zstd PROGRAM] [--character FILE] [--character-history-dir DIR] [--oracle FILE] [--validate]");
    std::process::exit(2);
}

#[cfg(not(target_arch = "wasm32"))]
fn default_character(archive_dir: &std::path::Path) -> Option<PathBuf> {
    let mut paths = std::fs::read_dir(archive_dir.join("character-saves"))
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("fch"))
        .collect::<Vec<_>>();
    paths.sort_by(|left, right| left.to_string_lossy().cmp(&right.to_string_lossy()));
    paths.into_iter().next()
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let mut archive_dir = PathBuf::from(".");
    let mut output_dir = PathBuf::from("reports-rust");
    let mut prefab_names = PathBuf::from("prefab_names.txt");
    let mut prefab_biomes = PathBuf::from("prefab_biomes.txt");
    let mut prefab_biomes_given = false;
    let mut zstd = String::from("zstd");
    let mut character: Option<PathBuf> = None;
    let mut character_history_dir: Option<PathBuf> = None;
    let mut oracle: Option<PathBuf> = None;
    let mut validate = false;
    let args: Vec<String> = env::args().skip(1).collect();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--archive-dir" => {
                index += 1;
                archive_dir = args
                    .get(index)
                    .map(PathBuf::from)
                    .unwrap_or_else(|| usage());
            }
            "--output-dir" => {
                index += 1;
                output_dir = args
                    .get(index)
                    .map(PathBuf::from)
                    .unwrap_or_else(|| usage());
            }
            "--prefab-names" => {
                index += 1;
                prefab_names = args
                    .get(index)
                    .map(PathBuf::from)
                    .unwrap_or_else(|| usage());
            }
            "--prefab-biomes" => {
                index += 1;
                prefab_biomes = args
                    .get(index)
                    .map(PathBuf::from)
                    .unwrap_or_else(|| usage());
                prefab_biomes_given = true;
            }
            "--zstd" => {
                index += 1;
                zstd = args.get(index).cloned().unwrap_or_else(|| usage());
            }
            "--character" => {
                index += 1;
                character = args.get(index).map(PathBuf::from).or_else(|| usage());
            }
            "--character-history-dir" => {
                index += 1;
                character_history_dir = args.get(index).map(PathBuf::from).or_else(|| usage());
            }
            "--oracle" => {
                index += 1;
                oracle = args.get(index).map(PathBuf::from).or_else(|| usage());
            }
            "--validate" => validate = true,
            "--help" | "-h" => usage(),
            _ => usage(),
        }
        index += 1;
    }

    let started = Instant::now();
    // The default biome table is optional; an explicitly requested one must exist.
    let biome_path = if prefab_biomes_given || prefab_biomes.is_file() {
        Some(prefab_biomes.as_path())
    } else {
        None
    };
    let archives = match scan_archives(&archive_dir, &zstd, &prefab_names, biome_path) {
        Ok(archives) => archives,
        Err(error) => {
            eprintln!("scan failed: {error}");
            std::process::exit(1);
        }
    };
    if validate {
        if let Err(error) = validate_oracle(&archives) {
            eprintln!("oracle validation failed: {error}");
            std::process::exit(1);
        }
    }

    let canonical = character.or_else(|| default_character(&archive_dir));
    if let Some(path) = &canonical {
        if !path.is_file() {
            eprintln!(
                "character scan failed: canonical character does not exist: {}",
                path.display()
            );
            std::process::exit(1);
        }
    }
    let characters = match canonical {
        Some(path) => {
            let history_dir =
                character_history_dir.unwrap_or_else(|| archive_dir.join("character-saves"));
            match scan_character_timeline(&path, &history_dir) {
                Ok(characters) => characters,
                Err(error) => {
                    eprintln!("character scan failed: {error}");
                    std::process::exit(1);
                }
            }
        }
        None => Vec::new(),
    };
    if validate {
        if let Err(error) = validate_character_oracle(&characters) {
            eprintln!("oracle validation failed: {error}");
            std::process::exit(1);
        }
        // World-specific expectations are opt-in: `--oracle FILE`, or `oracle.local.txt`
        // beside the archives when that local file exists. Neither is in the repository.
        let fixture = oracle.or_else(|| {
            let candidate = archive_dir.join("oracle.local.txt");
            candidate.is_file().then_some(candidate)
        });
        if let Some(path) = fixture {
            if let Err(error) = validate_oracle_fixture_file(&archives, &characters, &path) {
                eprintln!("oracle fixture failed: {error}");
                std::process::exit(1);
            }
        }
    }
    if let Err(error) = write_reports_with_characters(&output_dir, &archives, &characters) {
        eprintln!("report failed: {error}");
        std::process::exit(1);
    }
    println!(
        "scanned {} archives and {} character files in {:.3}s; reports written to {}",
        archives.len(),
        characters.len(),
        started.elapsed().as_secs_f64(),
        output_dir.display()
    );
}

#[cfg(target_arch = "wasm32")]
fn main() {}
