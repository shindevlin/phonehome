use anyhow::Result;
use dirs::home_dir;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    pub cwd: String,
    pub chat_id: i64,
    pub thread_id: Option<i64>,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub backend: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RoutesFile {
    #[serde(default)]
    pub route: Vec<Route>,
}

pub fn routes_path() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("/root"))
        .join(".phonehome")
        .join("routes.toml")
}

pub fn load() -> Vec<Route> {
    std::fs::read_to_string(routes_path())
        .ok()
        .and_then(|s| toml::from_str::<RoutesFile>(&s).ok())
        .map(|f| f.route)
        .unwrap_or_default()
}

pub fn save(routes: &[Route]) -> Result<()> {
    let content = routes.iter().fold(String::new(), |mut acc, r| {
        acc.push_str("[[route]]\n");
        acc.push_str(&format!("cwd = {:?}\n", r.cwd));
        acc.push_str(&format!("chat_id = {}\n", r.chat_id));
        if let Some(tid) = r.thread_id {
            acc.push_str(&format!("thread_id = {}\n", tid));
        }
        if !r.label.is_empty() {
            acc.push_str(&format!("label = {:?}\n", r.label));
        }
        if let Some(ref b) = r.backend {
            acc.push_str(&format!("backend = {:?}\n", b));
        }
        acc.push('\n');
        acc
    });
    let path = routes_path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, content)?;
    Ok(())
}

pub fn upsert(new: Route) -> Result<()> {
    let mut routes = load();
    if let Some(existing) = routes.iter_mut().find(|r| r.cwd == new.cwd) {
        *existing = new;
    } else {
        routes.push(new);
    }
    save(&routes)
}

/// Set the backend for the route matching (chat_id, thread_id). Returns false if no route found.
pub fn set_backend(chat_id: i64, thread_id: Option<i64>, backend: &str) -> Result<bool> {
    let mut routes = load();
    if let Some(r) = routes.iter_mut().find(|r| r.chat_id == chat_id && r.thread_id == thread_id) {
        r.backend = if backend == "default" { None } else { Some(backend.to_string()) };
        save(&routes)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Reverse lookup: given an incoming (chat_id, thread_id), find the mapped cwd.
pub fn cwd_for(routes: &[Route], chat_id: i64, thread_id: Option<i64>) -> Option<String> {
    routes.iter()
        .find(|r| r.chat_id == chat_id && r.thread_id == thread_id)
        .map(|r| r.cwd.clone())
}

/// Find the destination (chat_id, thread_id) for a given cwd,
/// falling back to the default chat when cwd matches cfg.cwd.
/// Resolves symlinks so /home/user/repos/X and /mnt/real/repos/X match the same route.
pub fn resolve(
    routes: &[Route],
    hook_cwd: &str,
    default_chat_id: i64,
    default_cwd: &str,
) -> Option<(i64, Option<i64>)> {
    // Resolve symlinks once; fall back to the raw string if canonicalize fails.
    let real_cwd: std::borrow::Cow<str> = std::fs::canonicalize(hook_cwd)
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
        .map(std::borrow::Cow::Owned)
        .unwrap_or(std::borrow::Cow::Borrowed(hook_cwd));

    let matches = |r: &&Route| r.cwd == hook_cwd || r.cwd == real_cwd.as_ref();
    if let Some(r) = routes.iter().find(matches) {
        return Some((r.chat_id, r.thread_id));
    }
    if hook_cwd == default_cwd || real_cwd == default_cwd {
        return Some((default_chat_id, None));
    }
    None
}

/// Parse https://t.me/c/XXXXXXXXXX/Y → (chat_id, thread_id)
/// Supergroup API IDs are "-100" prepended to the URL number.
pub fn parse_tme_url(url: &str) -> Option<(i64, Option<i64>)> {
    let path = url.strip_prefix("https://t.me/c/")?;
    let mut parts = path.splitn(2, '/');
    let chat_num = parts.next()?;
    let thread_id: Option<i64> = parts.next().and_then(|s| s.parse().ok());
    let chat_id: i64 = format!("-100{}", chat_num).parse().ok()?;
    Some((chat_id, thread_id))
}
