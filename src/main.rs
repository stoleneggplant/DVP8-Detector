//! p8detector: list Dolby Vision Profile 8.1 files in a directory tree.
//!
//! Detection relies on `ffprobe`, which exposes the Dolby Vision decoder
//! configuration record (dvcC / dvvC) as stream side data. Profile 8.1 is
//! `dv_profile == 8` with `dv_bl_signal_compatibility_id == 1` (HDR10-compatible
//! base layer). 8.2 (SDR) and 8.4 (HLG) use compatibility IDs 2 and 4.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use clap::Parser;
use serde_json::Value;
use walkdir::WalkDir;

const VIDEO_EXTENSIONS: &[&str] = &["mkv", "mp4", "m4v", "mov", "ts", "m2ts", "mts"];

#[derive(Parser)]
#[command(version, about = "List Dolby Vision Profile 8.1 files in a directory")]
struct Args {
    /// Directories (or files) to scan
    #[arg(default_value = ".")]
    paths: Vec<PathBuf>,

    /// Only scan the top level of each directory
    #[arg(long)]
    no_recursive: bool,

    /// Match any Profile 8 variant (8.1, 8.2, 8.4), not just 8.1
    #[arg(long)]
    any_p8: bool,

    /// Print the detected Dolby Vision profile next to each file
    #[arg(short, long)]
    verbose: bool,

    /// Number of parallel ffprobe processes (default: CPU count)
    #[arg(short, long)]
    jobs: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DoviConfig {
    profile: u64,
    compatibility_id: u64,
}

impl DoviConfig {
    fn is_p8(&self) -> bool {
        self.profile == 8
    }

    fn is_p81(&self) -> bool {
        self.profile == 8 && self.compatibility_id == 1
    }
}

/// Extract the Dolby Vision configuration from `ffprobe -show_streams -of json` output.
fn parse_dovi(ffprobe_json: &str) -> Option<DoviConfig> {
    let root: Value = serde_json::from_str(ffprobe_json).ok()?;
    root.get("streams")?
        .as_array()?
        .iter()
        .filter_map(|s| s.get("side_data_list")?.as_array())
        .flatten()
        .find(|sd| {
            sd.get("side_data_type").and_then(Value::as_str) == Some("DOVI configuration record")
        })
        .and_then(|sd| {
            Some(DoviConfig {
                profile: sd.get("dv_profile")?.as_u64()?,
                compatibility_id: sd.get("dv_bl_signal_compatibility_id")?.as_u64()?,
            })
        })
}

fn probe(path: &Path) -> Result<Option<DoviConfig>, String> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v",
            "-show_streams",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .map_err(|e| format!("failed to run ffprobe: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr
            .lines()
            .next()
            .unwrap_or("ffprobe failed")
            .to_string());
    }
    Ok(parse_dovi(&String::from_utf8_lossy(&output.stdout)))
}

fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

fn collect_files(paths: &[PathBuf], recursive: bool) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for root in paths {
        let walker = WalkDir::new(root).follow_links(true);
        let walker = if recursive {
            walker
        } else {
            walker.max_depth(1)
        };
        for entry in walker {
            match entry {
                Ok(e) if e.file_type().is_file() && is_video(e.path()) => files.push(e.into_path()),
                Ok(_) => {}
                Err(e) => eprintln!("p8detector: {e}"),
            }
        }
    }
    files.sort();
    files.dedup();
    files
}

fn main() -> ExitCode {
    let args = Args::parse();

    if Command::new("ffprobe").arg("-version").output().is_err() {
        eprintln!("p8detector: ffprobe not found in PATH (install ffmpeg)");
        return ExitCode::from(2);
    }

    let files = collect_files(&args.paths, !args.no_recursive);
    let jobs = args
        .jobs
        .unwrap_or_else(|| thread::available_parallelism().map_or(4, |n| n.get()))
        .clamp(1, files.len().max(1));

    let next = AtomicUsize::new(0);
    let matches = Mutex::new(Vec::new());
    thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = files.get(i) else { break };
                    match probe(path) {
                        Ok(Some(cfg)) if cfg.is_p81() || (args.any_p8 && cfg.is_p8()) => {
                            matches.lock().unwrap().push((path, cfg));
                        }
                        Ok(_) => {}
                        Err(e) => eprintln!("p8detector: {}: {e}", path.display()),
                    }
                }
            });
        }
    });

    let mut matches = matches.into_inner().unwrap();
    matches.sort_by(|a, b| a.0.cmp(b.0));
    for (path, cfg) in &matches {
        if args.verbose {
            println!(
                "P{}.{}\t{}",
                cfg.profile,
                cfg.compatibility_id,
                path.display()
            );
        } else {
            println!("{}", path.display());
        }
    }

    if matches.is_empty() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream_json(profile: u64, compat: u64) -> String {
        format!(
            r#"{{"streams": [{{"index": 0, "codec_name": "hevc", "side_data_list": [
                {{"side_data_type": "DOVI configuration record", "dv_version_major": 1,
                  "dv_version_minor": 0, "dv_profile": {profile}, "dv_level": 6,
                  "rpu_present_flag": 1, "el_present_flag": 0, "bl_present_flag": 1,
                  "dv_bl_signal_compatibility_id": {compat}}}]}}]}}"#
        )
    }

    #[test]
    fn detects_profile_8_1() {
        let cfg = parse_dovi(&stream_json(8, 1)).unwrap();
        assert!(cfg.is_p81());
    }

    #[test]
    fn profile_8_4_is_p8_but_not_8_1() {
        let cfg = parse_dovi(&stream_json(8, 4)).unwrap();
        assert!(cfg.is_p8());
        assert!(!cfg.is_p81());
    }

    #[test]
    fn profile_7_is_not_p8() {
        let cfg = parse_dovi(&stream_json(7, 6)).unwrap();
        assert!(!cfg.is_p8());
    }

    #[test]
    fn no_dovi_side_data() {
        let json = r#"{"streams": [{"index": 0, "codec_name": "hevc",
            "side_data_list": [{"side_data_type": "Mastering display metadata"}]}]}"#;
        assert_eq!(parse_dovi(json), None);
        assert_eq!(parse_dovi(r#"{"streams": [{"index": 0}]}"#), None);
        assert_eq!(parse_dovi("not json"), None);
    }

    #[test]
    fn video_extension_filter() {
        assert!(is_video(Path::new("a/Movie.MKV")));
        assert!(is_video(Path::new("clip.mp4")));
        assert!(!is_video(Path::new("subs.srt")));
        assert!(!is_video(Path::new("noext")));
    }
}
