//! Install / upgrade / uninstall engine.
//!
//! Faithful port of the upstream `install.sh` so both tools can manage the same game folder:
//!  * `.dlss-unlocked.manifest`  – newline separated list of installed relative paths
//!  * `<file>.dlss-unlocked.bak` – backup of a foreign file that was overwritten
//! Extra GUI-only metadata (version, options) lives in `.dlss-unlocked.gui.json`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};
use walkdir::WalkDir;

use crate::ini;
use crate::release;
use crate::util::{is_safe_rel, now_iso, read_json, rel_slash, write_json, ErrStr, Res};

pub const MANIFEST: &str = ".dlss-unlocked.manifest";
pub const GUI_META: &str = ".dlss-unlocked.gui.json";
pub const BAK: &str = ".dlss-unlocked.bak";

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InstallOptions {
    /// dxgi | version | winmm | asi
    pub proxy: String,
    pub streamline: bool,
    pub repatch: bool,
    pub reset_config: bool,
    /// ampere | ada | none | keep
    pub preset: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct InstallState {
    pub installed: bool,
    pub via_gui: bool,
    pub version: Option<String>,
    pub proxy: Option<String>,
    pub streamline: Option<bool>,
    pub repatch: Option<bool>,
    pub preset: Option<String>,
    pub installed_at: Option<String>,
    pub file_count: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct GuiMeta {
    version: String,
    options: InstallOptions,
    installed_at: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Preflight {
    pub exists: bool,
    pub writable: bool,
    pub has_manifest: bool,
    pub running: Vec<String>,
    pub conflicts: Vec<String>,
    pub has_ini: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OpResult {
    pub dir: String,
    pub files: usize,
    pub backed_up: Vec<String>,
    pub kept_ini: bool,
    pub launch_options: String,
}

#[derive(Serialize, Clone)]
struct OpProgress {
    phase: String,
    done: usize,
    total: usize,
    file: String,
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub fn read_manifest(dir: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(dir.join(MANIFEST)).ok()?;
    Some(
        text.lines()
            .map(|l| l.trim_end_matches('\r').to_string())
            .filter(|l| !l.is_empty())
            .collect(),
    )
}

pub fn read_state(dir: &Path) -> InstallState {
    let Some(files) = read_manifest(dir) else { return InstallState::default() };
    let meta: Option<GuiMeta> = read_json(&dir.join(GUI_META));
    let mut st = InstallState { installed: true, file_count: files.len(), ..Default::default() };
    if let Some(m) = meta {
        st.via_gui = true;
        st.version = Some(m.version);
        st.proxy = Some(m.options.proxy);
        st.streamline = Some(m.options.streamline);
        st.repatch = Some(m.options.repatch);
        st.preset = Some(m.options.preset);
        st.installed_at = Some(m.installed_at);
    } else {
        // Installed by install.sh / manually: infer proxy from manifest.
        st.proxy = ["dxgi", "version", "winmm"]
            .iter()
            .find(|p| files.iter().any(|f| f.eq_ignore_ascii_case(&format!("{p}.dll"))))
            .map(|s| s.to_string())
            .or_else(|| files.iter().any(|f| f.ends_with(".asi")).then(|| "asi".to_string()));
        st.repatch = Some(files.iter().any(|f| f.eq_ignore_ascii_case("dinput8.dll")));
    }
    st
}

fn running_in(dir: &Path) -> Vec<String> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
    let root = dir.to_string_lossy().to_lowercase();
    let mut out: Vec<String> = sys
        .processes()
        .values()
        .filter_map(|p| p.exe())
        .filter(|exe| exe.to_string_lossy().to_lowercase().starts_with(&root))
        .filter_map(|exe| exe.file_name().map(|s| s.to_string_lossy().to_string()))
        .collect();
    out.sort();
    out.dedup();
    out
}

fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(".dlss-gui-write-test");
    let ok = std::fs::write(&probe, b"ok").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

fn proxy_dest(proxy: &str) -> String {
    match proxy {
        "asi" => "plugins/dlss-unlocked.asi".to_string(),
        p => format!("{p}.dll"),
    }
}

// ---------------------------------------------------------------------------
// Core operations (blocking)
// ---------------------------------------------------------------------------

#[derive(PartialEq, Clone, Copy)]
enum RemoveMode {
    KeepIni,
    All,
    Restore,
}

fn clear_readonly(p: &Path) {
    if let Ok(meta) = std::fs::metadata(p) {
        let mut perm = meta.permissions();
        if perm.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perm.set_readonly(false);
            let _ = std::fs::set_permissions(p, perm);
        }
    }
}

/// Delete files listed in the manifest; optionally restore backups. Mirrors `remove_old` in install.sh.
fn remove_old(game: &Path, mode: RemoveMode) -> Res<()> {
    let Some(files) = read_manifest(game) else { return Ok(()) };
    for f in files.iter().filter(|f| is_safe_rel(f)) {
        if f == "OptiScaler.ini" && mode == RemoveMode::KeepIni {
            continue;
        }
        let target = game.join(f);
        if target.exists() {
            clear_readonly(&target);
            std::fs::remove_file(&target).map_err(|e| format!("{f}: {e}"))?;
        }
        if mode == RemoveMode::Restore {
            let bak = game.join(format!("{f}{BAK}"));
            if bak.exists() {
                std::fs::rename(&bak, &target).map_err(|e| format!("restore {f}: {e}"))?;
            }
        }
    }
    // Drop directories left empty, deepest first.
    let mut dirs: Vec<PathBuf> = Vec::new();
    for f in files.iter().filter(|f| is_safe_rel(f)) {
        let mut d = Path::new(f).parent();
        while let Some(p) = d {
            if p.as_os_str().is_empty() {
                break;
            }
            dirs.push(p.to_path_buf());
            d = p.parent();
        }
    }
    dirs.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    dirs.dedup();
    for d in dirs {
        let _ = std::fs::remove_dir(game.join(d)); // only succeeds when empty
    }
    Ok(())
}

/// Map a package file to its destination inside the game folder, or None to skip it.
fn map_dest(rel: &str, opts: &InstallOptions) -> Option<String> {
    let lower = rel.to_lowercase();
    if lower == "install.sh" || lower.starts_with(".dlss-unlocked") {
        return None;
    }
    if lower == "dxgi.dll" {
        return Some(proxy_dest(&opts.proxy));
    }
    if lower == "repatch/dinput8.dll" {
        return opts.repatch.then(|| "dinput8.dll".to_string());
    }
    if lower.starts_with("repatch/") {
        return None;
    }
    if lower.starts_with("optiscaler/streamline/") {
        if !opts.streamline || lower.starts_with("optiscaler/streamline/development/") {
            return None;
        }
    }
    Some(rel.to_string())
}

fn launch_options(proxy: &str) -> String {
    let dll = if proxy == "asi" { "dxgi".to_string() } else { proxy.to_string() };
    format!("WINEDLLOVERRIDES=\"{dll}=n,b\" PROTON_ENABLE_NVAPI=1 PROTON_NVIDIA_NVCUDA=1 %command%")
}

fn apply_preset(game: &Path, preset: &str) -> Res<()> {
    let ini_path = game.join("OptiScaler.ini");
    if !ini_path.exists() || preset == "keep" {
        return Ok(());
    }
    let (ampere, ada) = match preset {
        "ampere" => ("true", "false"),
        "ada" => ("false", "true"),
        _ => ("false", "false"),
    };
    let text = std::fs::read_to_string(&ini_path).es()?;
    // Same post-install fix-up the official Inno Setup installer performs.
    let text = ini::remove_key_outside(&text, "AmpereMfgUnlock", "DLSSG");
    let changes = vec![
        ("FrameGen".to_string(), "External".to_string(), "true".to_string()),
        ("DLSSG".to_string(), "AmpereMfgUnlock".to_string(), ampere.to_string()),
        ("DLSSG".to_string(), "AdaMfgUnlock".to_string(), ada.to_string()),
    ];
    let out = ini::set_values(&text, &changes, false);
    std::fs::write(&ini_path, out).es()
}

pub fn install_blocking(
    app: &AppHandle,
    game: &Path,
    pkg_root: &Path,
    version: &str,
    opts: &InstallOptions,
) -> Res<OpResult> {
    if !game.is_dir() {
        return Err(format!("Game folder not found: {}", game.display()));
    }
    if !matches!(opts.proxy.as_str(), "dxgi" | "version" | "winmm" | "asi") {
        return Err("proxy must be dxgi, version, winmm or asi".into());
    }
    let running = running_in(game);
    if !running.is_empty() {
        return Err(format!("Close the game first: {}", running.join(", ")));
    }

    let emit = |phase: &str, done: usize, total: usize, file: &str| {
        let _ = app.emit(
            "op-progress",
            OpProgress { phase: phase.into(), done, total, file: file.into() },
        );
    };

    let had_manifest = game.join(MANIFEST).exists();
    emit("cleanup", 0, 0, "");
    remove_old(game, if opts.reset_config { RemoveMode::All } else { RemoveMode::KeepIni })?;

    // Collect payload
    let mut payload: Vec<(PathBuf, String)> = Vec::new();
    for e in WalkDir::new(pkg_root).follow_links(false).into_iter().flatten() {
        if !e.file_type().is_file() {
            continue;
        }
        let rel = rel_slash(e.path().strip_prefix(pkg_root).es()?);
        if let Some(dest) = map_dest(&rel, opts) {
            payload.push((e.path().to_path_buf(), dest));
        }
    }
    if payload.is_empty() {
        return Err("Package is empty – re-download it from the Packages page".into());
    }

    let total = payload.len();
    let mut manifest: Vec<String> = Vec::with_capacity(total);
    let mut backed_up = Vec::new();
    let mut kept_ini = false;

    for (i, (src, dest)) in payload.iter().enumerate() {
        emit("copy", i + 1, total, dest);
        manifest.push(dest.clone());
        let target = game.join(dest);
        if target.exists() {
            if dest == "OptiScaler.ini" && had_manifest && !opts.reset_config {
                kept_ini = true;
                continue;
            }
            let bak = game.join(format!("{dest}{BAK}"));
            if !bak.exists() {
                std::fs::rename(&target, &bak).map_err(|e| format!("backup {dest}: {e}"))?;
                backed_up.push(dest.clone());
            } else {
                clear_readonly(&target);
                let _ = std::fs::remove_file(&target);
            }
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).es()?;
        }
        std::fs::copy(src, &target).map_err(|e| format!("{dest}: {e}"))?;
    }

    let mut mtext = manifest.join("\n");
    mtext.push('\n');
    std::fs::write(game.join(MANIFEST), mtext).es()?;

    // Only configure the INI on a fresh copy, unless the user explicitly asked for a preset.
    if !kept_ini || opts.preset != "keep" {
        apply_preset(game, &opts.preset)?;
    }

    write_json(
        &game.join(GUI_META),
        &GuiMeta { version: version.to_string(), options: opts.clone(), installed_at: now_iso() },
    )?;
    emit("done", total, total, "");

    Ok(OpResult {
        dir: game.to_string_lossy().to_string(),
        files: total,
        backed_up,
        kept_ini,
        launch_options: launch_options(&opts.proxy),
    })
}

pub fn uninstall_blocking(game: &Path) -> Res<usize> {
    let Some(files) = read_manifest(game) else {
        return Err("Nothing to uninstall (no manifest found)".into());
    };
    let running = running_in(game);
    if !running.is_empty() {
        return Err(format!("Close the game first: {}", running.join(", ")));
    }
    remove_old(game, RemoveMode::Restore)?;
    let _ = std::fs::remove_file(game.join(MANIFEST));
    let _ = std::fs::remove_file(game.join(GUI_META));
    Ok(files.len())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn preflight(dir: String, proxy: String) -> Res<Preflight> {
    tauri::async_runtime::spawn_blocking(move || {
        let p = PathBuf::from(&dir);
        let exists = p.is_dir();
        let ours = read_manifest(&p).unwrap_or_default();
        let dest = proxy_dest(&proxy);
        let mut conflicts: Vec<String> = crate::scanner::foreign_proxies(&p);
        if exists && p.join(&dest).exists() && !ours.iter().any(|o| o.eq_ignore_ascii_case(&dest)) && !conflicts.contains(&dest) {
            conflicts.push(dest);
        }
        Preflight {
            exists,
            writable: exists && is_writable(&p),
            has_manifest: !ours.is_empty(),
            running: if exists { running_in(&p) } else { vec![] },
            conflicts,
            has_ini: p.join("OptiScaler.ini").exists(),
        }
    })
    .await
    .es()
}

#[tauri::command]
pub async fn install_game(app: AppHandle, dir: String, tag: String, options: InstallOptions) -> Res<OpResult> {
    let pkg = release::package_root(&app, &tag)?;
    tauri::async_runtime::spawn_blocking(move || install_blocking(&app, Path::new(&dir), &pkg, &tag, &options))
        .await
        .es()?
}

#[tauri::command]
pub async fn uninstall_game(dir: String) -> Res<usize> {
    tauri::async_runtime::spawn_blocking(move || uninstall_blocking(Path::new(&dir)))
        .await
        .es()?
}

#[tauri::command]
pub fn install_state(dir: String) -> InstallState {
    read_state(Path::new(&dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(proxy: &str) -> InstallOptions {
        InstallOptions { proxy: proxy.into(), streamline: false, repatch: false, reset_config: false, preset: "ampere".into() }
    }

    #[test]
    fn maps_destinations() {
        let o = opts("version");
        assert_eq!(map_dest("dxgi.dll", &o).as_deref(), Some("version.dll"));
        assert_eq!(map_dest("install.sh", &o), None);
        assert_eq!(map_dest("REPatch/dinput8.dll", &o), None);
        assert_eq!(map_dest("REPatch/README.txt", &o), None);
        assert_eq!(map_dest("OptiScaler/streamline/sl.interposer.dll", &o), None);
        assert_eq!(map_dest("OptiScaler/dlssg_sm86/dlssg_sm86.dll", &o).as_deref(), Some("OptiScaler/dlssg_sm86/dlssg_sm86.dll"));
        let mut o2 = opts("asi");
        o2.repatch = true;
        o2.streamline = true;
        assert_eq!(map_dest("dxgi.dll", &o2).as_deref(), Some("plugins/dlss-unlocked.asi"));
        assert_eq!(map_dest("REPatch/dinput8.dll", &o2).as_deref(), Some("dinput8.dll"));
        assert_eq!(map_dest("OptiScaler/streamline/sl.interposer.dll", &o2).as_deref(), Some("OptiScaler/streamline/sl.interposer.dll"));
        assert_eq!(map_dest("OptiScaler/streamline/development/x.dll", &o2), None);
    }

    #[test]
    fn remove_old_restores_backups() {
        let tmp = std::env::temp_dir().join(format!("dlssgui-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("OptiScaler")).unwrap();
        std::fs::write(tmp.join("dxgi.dll"), "ours").unwrap();
        std::fs::write(tmp.join(format!("dxgi.dll{BAK}")), "reshade").unwrap();
        std::fs::write(tmp.join("OptiScaler/a.dll"), "x").unwrap();
        std::fs::write(tmp.join("OptiScaler.ini"), "[A]").unwrap();
        std::fs::write(tmp.join(MANIFEST), "dxgi.dll\nOptiScaler/a.dll\nOptiScaler.ini\n").unwrap();

        remove_old(&tmp, RemoveMode::Restore).unwrap();
        assert_eq!(std::fs::read_to_string(tmp.join("dxgi.dll")).unwrap(), "reshade");
        assert!(!tmp.join("OptiScaler").exists());
        assert!(!tmp.join("OptiScaler.ini").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
