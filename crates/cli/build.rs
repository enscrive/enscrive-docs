use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::path::Path::new(".git/HEAD").exists() {
        println!("cargo:rerun-if-changed=.git/HEAD");
        println!("cargo:rerun-if-changed=.git/refs/heads");
    }

    println!("cargo:rustc-env=ENSCRIVE_GIT_SHA={}", git_sha());
    println!("cargo:rustc-env=ENSCRIVE_BUILD_DATE={}", build_date());
}

fn git_sha() -> String {
    if let Ok(out) = Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
    {
        if out.status.success() {
            let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !sha.is_empty() {
                let dirty = Command::new("git")
                    .args(["status", "--porcelain"])
                    .output()
                    .map(|o| !o.stdout.is_empty())
                    .unwrap_or(false);
                return if dirty { format!("{sha}-dirty") } else { sha };
            }
        }
    }
    // CI fallback: no .git but GITHUB_SHA is set
    if let Ok(sha) = std::env::var("GITHUB_SHA") {
        let s: String = sha.chars().take(7).collect();
        if !s.is_empty() {
            return s;
        }
    }
    "unknown".to_string()
}

fn build_date() -> String {
    let secs = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        });
    secs_to_ymd(secs)
}

// Pure-Rust Gregorian calendar conversion (Howard Hinnant's civil_from_days).
// No external crate, no runtime syscall.
fn secs_to_ymd(secs: u64) -> String {
    let days = secs / 86400;
    let z = days as i64 + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}
