use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

pub type Res<T> = Result<T, String>;

/// Convert any displayable error into a `String` so it can cross the Tauri IPC boundary.
pub trait ErrStr<T> {
    fn es(self) -> Res<T>;
}

impl<T, E: std::fmt::Display> ErrStr<T> for Result<T, E> {
    fn es(self) -> Res<T> {
        self.map_err(|e| e.to_string())
    }
}

pub fn data_dir(app: &AppHandle) -> Res<PathBuf> {
    let d = app.path().app_data_dir().es()?;
    std::fs::create_dir_all(&d).es()?;
    Ok(d)
}

pub fn packages_dir(app: &AppHandle) -> Res<PathBuf> {
    let d = data_dir(app)?.join("packages");
    std::fs::create_dir_all(&d).es()?;
    Ok(d)
}

/// A `Command` that never flashes a console window on Windows.
pub fn hidden_command(program: &str) -> std::process::Command {
    #[allow(unused_mut)]
    let mut c = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// Relative path rendered with forward slashes (same format as install.sh manifests).
pub fn rel_slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Reject manifest entries that could escape the game folder.
pub fn is_safe_rel(rel: &str) -> bool {
    let p = Path::new(rel);
    !rel.is_empty()
        && !p.is_absolute()
        && !rel.contains(':')
        && p.components().all(|c| matches!(c, std::path::Component::Normal(_)))
}

pub fn now_iso() -> String {
    chrono::Local::now().to_rfc3339()
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Res<()> {
    let text = serde_json::to_string_pretty(value).es()?;
    std::fs::write(path, text).es()
}
