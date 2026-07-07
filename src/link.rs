/// Manages ~/.phonehome/watch_dirs — one canonical path per line.
/// The bot reads this to know which repos to route Telegram messages to.
use anyhow::{Context, Result};
use dirs::home_dir;
use std::path::{Path, PathBuf};

fn watch_dirs_path() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("/root"))
        .join(".phonehome")
        .join("watch_dirs")
}

pub fn read_all() -> Vec<String> {
    std::fs::read_to_string(watch_dirs_path())
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn write_all(dirs: &[String]) -> Result<()> {
    let path = watch_dirs_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, dirs.join("\n") + "\n").context("failed to write watch_dirs")
}

fn resolve(path: Option<PathBuf>) -> Result<String> {
    let p = match path {
        Some(p) => p,
        None => std::env::current_dir().context("cannot determine current directory")?,
    };
    // canonicalize only if the path exists
    let canonical = if p.exists() {
        p.canonicalize().unwrap_or(p)
    } else {
        p
    };
    Ok(canonical.to_string_lossy().to_string())
}

/// Path of the per-repo marker file.
pub fn marker_path(repo: &Path) -> PathBuf {
    repo.join(".phonehome")
}

/// True if this directory (or any parent up to 4 levels) has a .phonehome marker FILE.
pub fn has_marker(dir: &str) -> bool {
    let mut p = Path::new(dir).to_path_buf();
    for _ in 0..5 {
        if p.join(".phonehome").is_file() {
            return true;
        }
        if !p.pop() { break; }
    }
    false
}

pub fn link(path: Option<PathBuf>) -> Result<()> {
    let target = resolve(path)?;

    // Write .phonehome marker in the repo
    let marker = marker_path(Path::new(&target));
    if !marker.exists() {
        std::fs::write(&marker, format!("# phonehome repo marker\n# linked: {}\n", chrono::Local::now().format("%Y-%m-%d")))
            .with_context(|| format!("cannot write {}", marker.display()))?;
        println!("created {}", marker.display());
    }

    // Also register in the global watch_dirs list
    let mut dirs = read_all();
    if !dirs.contains(&target) {
        dirs.push(target.clone());
        write_all(&dirs)?;
    }

    println!("linked: {target}");
    println!("phonehome will now watch this repo. No restart needed.");
    Ok(())
}

pub fn unlink(path: Option<PathBuf>) -> Result<()> {
    let target = resolve(path)?;

    // Remove .phonehome marker from the repo
    let marker = marker_path(Path::new(&target));
    if marker.exists() {
        std::fs::remove_file(&marker).ok();
        println!("removed {}", marker.display());
    }

    // Remove from global list
    let before = read_all();
    let after: Vec<String> = before.iter().filter(|s| s.as_str() != target).cloned().collect();
    if after.len() == before.len() {
        println!("not in watch list: {target}");
    } else {
        write_all(&after)?;
        println!("unlinked: {target}");
    }
    Ok(())
}

pub fn list() -> Result<()> {
    let dirs = read_all();
    if dirs.is_empty() {
        let cfg = crate::config::Config::load()?;
        println!("no dirs linked — defaulting to cwd: {}", cfg.cwd);
    } else {
        println!("linked directories:");
        for d in &dirs { println!("  {d}"); }
    }
    Ok(())
}

/// Returns true if hook_cwd is in the watch list (or watch list is empty and matches cfg.cwd).
pub fn is_watched(hook_cwd: &str, cfg_cwd: &str) -> bool {
    let dirs = read_all();
    if dirs.is_empty() {
        hook_cwd == cfg_cwd
    } else {
        dirs.iter().any(|d| d == hook_cwd)
            || dirs.iter().any(|d| Path::new(hook_cwd).starts_with(d))
    }
}
