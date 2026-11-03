use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};
use walkdir::WalkDir;

use crate::installer::{self, InstallState};
use crate::util::{data_dir, read_json, write_json, ErrStr, Res};
use crate::vdf;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Features {
    pub dlss: bool,
    pub dlssg: bool,
    pub dlssd: bool,
    pub streamline: bool,
    pub re_engine: bool,
    pub unreal: bool,
    pub anti_cheat: Option<String>,
    pub existing_proxies: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Game {
    pub id: String,
    pub name: String,
    /// steam | epic | gog | custom
    pub source: String,
    pub install_dir: String,
    pub exe_dir: Option<String>,
    pub exe_name: Option<String>,
    pub exe_dir_overridden: bool,
    pub cover: Option<String>,
    pub app_id: Option<String>,
    pub features: Features,
    pub install: InstallState,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CustomGame {
    pub id: String,
    pub name: String,
    pub dir: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct LibraryConfig {
    #[serde(default)]
    pub custom: Vec<CustomGame>,
    /// game id -> user-chosen executable directory
    #[serde(default)]
    pub exe_overrides: HashMap<String, String>,
    #[serde(default)]
    pub hidden: Vec<String>,
}

#[derive(Serialize, Clone)]
struct ScanProgress {
    current: usize,
    total: usize,
    name: String,
}

/// A discovered game before deep detection.
struct Candidate {
    id: String,
    name: String,
    source: &'static str,
    install_dir: PathBuf,
    exe_hint: Option<PathBuf>,
    app_id: Option<String>,
    cover: Option<String>,
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

pub fn load_library(app: &AppHandle) -> LibraryConfig {
    data_dir(app).ok().and_then(|d| read_json(&d.join("library.json"))).unwrap_or_default()
}

pub fn save_library(app: &AppHandle, cfg: &LibraryConfig) -> Res<()> {
    write_json(&data_dir(app)?.join("library.json"), cfg)
}

fn cache_path(app: &AppHandle) -> Res<PathBuf> {
    Ok(data_dir(app)?.join("scan-cache.json"))
}

// ---------------------------------------------------------------------------
// Store discovery
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn steam_root() -> Option<PathBuf> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(k) = hkcu.open_subkey("Software\\Valve\\Steam") {
        if let Ok(p) = k.get_value::<String, _>("SteamPath") {
            let p = PathBuf::from(p.replace('/', "\\"));
            if p.exists() {
                return Some(p);
            }
        }
    }
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    if let Ok(k) = hklm.open_subkey("SOFTWARE\\WOW6432Node\\Valve\\Steam") {
        if let Ok(p) = k.get_value::<String, _>("InstallPath") {
            let p = PathBuf::from(p);
            if p.exists() {
                return Some(p);
            }
        }
    }
    let def = PathBuf::from("C:\\Program Files (x86)\\Steam");
    def.exists().then_some(def)
}

#[cfg(not(windows))]
fn steam_root() -> Option<PathBuf> {
    None
}

fn steam_games() -> Vec<Candidate> {
    let Some(root) = steam_root() else { return vec![] };
    let mut libs: Vec<PathBuf> = vec![root.clone()];
    if let Ok(text) = std::fs::read_to_string(root.join("steamapps").join("libraryfolders.vdf")) {
        let v = vdf::parse(&text);
        if let Some(lf) = v.get("libraryfolders") {
            for (_, entry) in lf.children() {
                if let Some(p) = entry.str("path") {
                    let pb = PathBuf::from(p);
                    if !libs.iter().any(|l| l.to_string_lossy().eq_ignore_ascii_case(&pb.to_string_lossy())) {
                        libs.push(pb);
                    }
                }
            }
        }
    }

    const SKIP_IDS: &[&str] = &["228980", "1070560", "1391110", "1628350", "1493710", "2180100"];
    let mut out = Vec::new();
    for lib in libs {
        let apps = lib.join("steamapps");
        let Ok(rd) = std::fs::read_dir(&apps) else { continue };
        for e in rd.flatten() {
            let fname = e.file_name().to_string_lossy().to_string();
            if !(fname.starts_with("appmanifest_") && fname.ends_with(".acf")) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
            let v = vdf::parse(&text);
            let Some(st) = v.get("AppState") else { continue };
            let appid = st.str("appid").unwrap_or("").to_string();
            let name = st.str("name").unwrap_or("").to_string();
            let installdir = st.str("installdir").unwrap_or("");
            if appid.is_empty() || installdir.is_empty() || SKIP_IDS.contains(&appid.as_str()) {
                continue;
            }
            let ln = name.to_lowercase();
            if ln.contains("proton") || ln.contains("steam linux runtime") || ln.contains("steamworks") || ln.contains("redistributable") {
                continue;
            }
            let dir = apps.join("common").join(installdir);
            if !dir.exists() {
                continue;
            }
            out.push(Candidate {
                id: format!("steam:{appid}"),
                cover: Some(format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{appid}/library_600x900.jpg")),
                app_id: Some(appid),
                name,
                source: "steam",
                install_dir: dir,
                exe_hint: None,
            });
        }
    }
    out
}

fn epic_games() -> Vec<Candidate> {
    let pd = std::env::var("PROGRAMDATA").unwrap_or_else(|_| "C:\\ProgramData".into());
    let dir = Path::new(&pd).join("Epic").join("EpicGamesLauncher").join("Data").join("Manifests");
    let Ok(rd) = std::fs::read_dir(&dir) else { return vec![] };
    let mut out = Vec::new();
    for e in rd.flatten() {
        if e.path().extension().and_then(|s| s.to_str()) != Some("item") {
            continue;
        }
        let Some(v) = read_json::<serde_json::Value>(&e.path()) else { continue };
        if v["bIsIncompleteInstall"].as_bool() == Some(true) {
            continue;
        }
        let name = v["DisplayName"].as_str().unwrap_or("").to_string();
        let loc = v["InstallLocation"].as_str().unwrap_or("");
        let app = v["AppName"].as_str().unwrap_or(&name).to_string();
        if name.is_empty() || loc.is_empty() || !Path::new(loc).exists() {
            continue;
        }
        let exe_hint = v["LaunchExecutable"].as_str().filter(|s| !s.is_empty()).map(|s| Path::new(loc).join(s));
        out.push(Candidate {
            id: format!("epic:{app}"),
            name,
            source: "epic",
            install_dir: PathBuf::from(loc),
            exe_hint,
            app_id: Some(app),
            cover: None,
        });
    }
    out
}

#[cfg(windows)]
fn gog_games() -> Vec<Candidate> {
    use winreg::enums::*;
    use winreg::RegKey;
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let Ok(games) = hklm.open_subkey("SOFTWARE\\WOW6432Node\\GOG.com\\Games") else { return vec![] };
    let mut out = Vec::new();
    for id in games.enum_keys().flatten() {
        let Ok(k) = games.open_subkey(&id) else { continue };
        let name: String = k.get_value("gameName").unwrap_or_default();
        let path: String = k.get_value("path").unwrap_or_default();
        let exe: String = k.get_value("exe").unwrap_or_default();
        if name.is_empty() || path.is_empty() || !Path::new(&path).exists() {
            continue;
        }
        out.push(Candidate {
            id: format!("gog:{id}"),
            name,
            source: "gog",
            install_dir: PathBuf::from(&path),
            exe_hint: (!exe.is_empty()).then(|| PathBuf::from(exe)),
            app_id: Some(id),
            cover: None,
        });
    }
    out
}

#[cfg(not(windows))]
fn gog_games() -> Vec<Candidate> {
    vec![]
}

// ---------------------------------------------------------------------------
// Feature / executable detection
// ---------------------------------------------------------------------------

const EXE_SKIP_NAMES: &[&str] = &[
    "crash", "unins", "setup", "redist", "vc_redist", "vcredist", "dxsetup", "dotnet", "unitycrashhandler",
    "easyanticheat", "beservice", "battleye", "start_protected_game", "cefprocess", "quicksfv", "report",
    "helper", "updater", "touchup", "cleanup", "dxwebsetup", "physx", "oalinst", "ue4prereq", "prereq",
];
const DIR_SKIP: &[&str] = &[
    "_commonredist", "redist", "redistributables", "directx", "vcredist", "__installer", "installers", "prereqs",
    "dotnet", "movies", "videos", "paks", "shadercache",
];
const PROXY_NAMES: &[&str] = &["dxgi.dll", "version.dll", "winmm.dll", "dinput8.dll", "d3d12.dll", "d3d11.dll"];

pub struct Detection {
    pub features: Features,
    pub exe_dir: Option<PathBuf>,
    pub exe_name: Option<String>,
}

pub fn detect(install_dir: &Path, exe_hint: Option<&Path>) -> Detection {
    let mut f = Features::default();
    let mut manifest_dir: Option<PathBuf> = None;
    let mut best: Option<(i64, PathBuf)> = None;

    let walker = WalkDir::new(install_dir)
        .max_depth(7)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 || !e.file_type().is_dir() {
                return true;
            }
            let n = e.file_name().to_string_lossy().to_lowercase();
            if n == "easyanticheat" || n == "battleye" {
                return true; // inspected below, but not descended deeply
            }
            !DIR_SKIP.contains(&n.as_str()) && !n.starts_with('.') && n != "optiscaler"
        });

    for (count, entry) in walker.flatten().enumerate() {
        if count > 80_000 {
            break;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if entry.file_type().is_dir() {
            match name.as_str() {
                "easyanticheat" => f.anti_cheat = Some("EasyAntiCheat".into()),
                "battleye" => f.anti_cheat = Some("BattlEye".into()),
                _ => {}
            }
            continue;
        }
        let path = entry.path();
        match name.as_str() {
            "nvngx_dlss.dll" => f.dlss = true,
            "nvngx_dlssg.dll" | "sl.dlss_g.dll" => f.dlssg = true,
            "nvngx_dlssd.dll" => f.dlssd = true,
            "sl.interposer.dll" => f.streamline = true,
            "re_chunk_000.pak" => f.re_engine = true,
            "start_protected_game.exe" | "easyanticheat_eos_setup.exe" => f.anti_cheat = Some("EasyAntiCheat".into()),
            "beservice.exe" | "beservice_x64.exe" => f.anti_cheat = Some("BattlEye".into()),
            ".dlss-unlocked.manifest" => manifest_dir = path.parent().map(|p| p.to_path_buf()),
            _ => {}
        }
        if name.ends_with(".exe") {
            let lpath = path.to_string_lossy().to_lowercase();
            if lpath.contains("\\engine\\binaries\\") {
                f.unreal = true;
                continue;
            }
            if EXE_SKIP_NAMES.iter().any(|k| name.contains(k)) {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0) as i64;
            let mut score = size / (1024 * 1024);
            if name.contains("-win64-shipping") || name.contains("_shipping") {
                score += 10_000;
                f.unreal = true;
            }
            if lpath.contains("\\binaries\\win64\\") || lpath.contains("\\bin\\x64\\") {
                score += 500;
            }
            if name.contains("launcher") {
                score -= 2_000;
            }
            if best.as_ref().map_or(true, |(s, _)| score > *s) {
                best = Some((score, path.to_path_buf()));
            }
        }
    }

    // Priority: existing DLSS Unlocked install > store hint > heuristic.
    let exe_path = exe_hint.filter(|p| p.exists()).map(|p| p.to_path_buf()).or(best.map(|(_, p)| p));
    let (exe_dir, exe_name) = match (&manifest_dir, &exe_path) {
        (Some(m), _) => (Some(m.clone()), exe_path.as_ref().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().to_string())),
        (None, Some(p)) => (p.parent().map(|d| d.to_path_buf()), p.file_name().map(|s| s.to_string_lossy().to_string())),
        _ => (None, None),
    };

    if let Some(dir) = &exe_dir {
        f.existing_proxies = foreign_proxies(dir);
    }

    Detection { features: f, exe_dir, exe_name }
}

/// Proxy DLLs present in `dir` that DLSS Unlocked did not install (ReShade, other mods...).
pub fn foreign_proxies(dir: &Path) -> Vec<String> {
    let ours = installer::read_manifest(dir).unwrap_or_default();
    PROXY_NAMES
        .iter()
        .filter(|n| dir.join(n).exists() && !ours.iter().any(|o| o.eq_ignore_ascii_case(n)))
        .map(|s| s.to_string())
        .collect()
}

fn build_game(c: Candidate, cfg: &LibraryConfig) -> Game {
    let det = detect(&c.install_dir, c.exe_hint.as_deref());
    let override_dir = cfg.exe_overrides.get(&c.id).map(PathBuf::from).filter(|p| p.exists());
    let exe_dir_overridden = override_dir.is_some();
    let exe_dir = override_dir.or(det.exe_dir);
    let mut features = det.features;
    if exe_dir_overridden {
        if let Some(d) = &exe_dir {
            features.existing_proxies = foreign_proxies(d);
        }
    }
    let install = exe_dir.as_deref().map(installer::read_state).unwrap_or_default();
    Game {
        id: c.id,
        name: c.name,
        source: c.source.to_string(),
        install_dir: c.install_dir.to_string_lossy().to_string(),
        exe_dir: exe_dir.map(|p| p.to_string_lossy().to_string()),
        exe_name: det.exe_name,
        exe_dir_overridden,
        cover: c.cover,
        app_id: c.app_id,
        features,
        install,
    }
}

fn custom_candidates(cfg: &LibraryConfig) -> Vec<Candidate> {
    cfg.custom
        .iter()
        .filter(|g| Path::new(&g.dir).exists())
        .map(|g| Candidate {
            id: g.id.clone(),
            name: g.name.clone(),
            source: "custom",
            install_dir: PathBuf::from(&g.dir),
            exe_hint: None,
            app_id: None,
            cover: None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn scan_games(app: AppHandle) -> Res<Vec<Game>> {
    let app2 = app.clone();
    let games = tauri::async_runtime::spawn_blocking(move || {
        let cfg = load_library(&app2);
        let mut cands = steam_games();
        cands.extend(epic_games());
        cands.extend(gog_games());
        cands.extend(custom_candidates(&cfg));
        cands.retain(|c| !cfg.hidden.contains(&c.id));

        let total = cands.len();
        let mut games = Vec::with_capacity(total);
        for (i, c) in cands.into_iter().enumerate() {
            let _ = app2.emit("scan-progress", ScanProgress { current: i + 1, total, name: c.name.clone() });
            games.push(build_game(c, &cfg));
        }
        games.sort_by(|a, b| {
            let ka = (!a.features.dlss, a.name.to_lowercase());
            let kb = (!b.features.dlss, b.name.to_lowercase());
            ka.cmp(&kb)
        });
        games
    })
    .await
    .es()?;

    if let Ok(p) = cache_path(&app) {
        let _ = write_json(&p, &games);
    }
    Ok(games)
}

#[tauri::command]
pub fn get_cached_games(app: AppHandle) -> Res<Vec<Game>> {
    let p = cache_path(&app)?;
    let mut games: Vec<Game> = read_json(&p).unwrap_or_default();
    // Install state may have changed outside the app: refresh cheaply.
    for g in games.iter_mut() {
        if let Some(d) = &g.exe_dir {
            g.install = installer::read_state(Path::new(d));
        }
    }
    Ok(games)
}

/// Re-detect a single game (after install/uninstall or exe dir change).
#[tauri::command]
pub async fn refresh_game(app: AppHandle, game: Game) -> Res<Game> {
    tauri::async_runtime::spawn_blocking(move || {
        let cfg = load_library(&app);
        let c = Candidate {
            id: game.id.clone(),
            name: game.name.clone(),
            source: match game.source.as_str() {
                "steam" => "steam",
                "epic" => "epic",
                "gog" => "gog",
                _ => "custom",
            },
            install_dir: PathBuf::from(&game.install_dir),
            exe_hint: match (&game.exe_dir, &game.exe_name) {
                (Some(d), Some(n)) => Some(Path::new(d).join(n)),
                _ => None,
            },
            app_id: game.app_id.clone(),
            cover: game.cover.clone(),
        };
        let updated = build_game(c, &cfg);
        if let Ok(p) = cache_path(&app) {
            let mut all: Vec<Game> = read_json(&p).unwrap_or_default();
            if let Some(slot) = all.iter_mut().find(|g| g.id == updated.id) {
                *slot = updated.clone();
            } else {
                all.push(updated.clone());
            }
            let _ = write_json(&p, &all);
        }
        Ok(updated)
    })
    .await
    .es()?
}

#[tauri::command]
pub async fn add_custom_game(app: AppHandle, dir: String, name: Option<String>) -> Res<Game> {
    let p = PathBuf::from(&dir);
    if !p.is_dir() {
        return Err("Folder not found".into());
    }
    let mut cfg = load_library(&app);
    let id = format!("custom:{}", dir.to_lowercase().replace(['\\', '/', ':', ' '], "_"));
    let name = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| {
        p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| dir.clone())
    });
    if !cfg.custom.iter().any(|g| g.id == id) {
        cfg.custom.push(CustomGame { id: id.clone(), name: name.clone(), dir: dir.clone() });
    }
    cfg.hidden.retain(|h| h != &id);
    save_library(&app, &cfg)?;

    let c = Candidate { id, name, source: "custom", install_dir: p, exe_hint: None, app_id: None, cover: None };
    let app2 = app.clone();
    let game = tauri::async_runtime::spawn_blocking(move || {
        let cfg = load_library(&app2);
        build_game(c, &cfg)
    })
    .await
    .es()?;

    if let Ok(cp) = cache_path(&app) {
        let mut all: Vec<Game> = read_json(&cp).unwrap_or_default();
        all.retain(|g| g.id != game.id);
        all.push(game.clone());
        let _ = write_json(&cp, &all);
    }
    Ok(game)
}

/// Remove a custom game, or hide a store game from the library.
#[tauri::command]
pub fn remove_game(app: AppHandle, id: String) -> Res<()> {
    let mut cfg = load_library(&app);
    if id.starts_with("custom:") {
        cfg.custom.retain(|g| g.id != id);
    } else if !cfg.hidden.contains(&id) {
        cfg.hidden.push(id.clone());
    }
    save_library(&app, &cfg)?;
    if let Ok(cp) = cache_path(&app) {
        let mut all: Vec<Game> = read_json(&cp).unwrap_or_default();
        all.retain(|g| g.id != id);
        let _ = write_json(&cp, &all);
    }
    Ok(())
}

#[tauri::command]
pub fn unhide_all(app: AppHandle) -> Res<()> {
    let mut cfg = load_library(&app);
    cfg.hidden.clear();
    save_library(&app, &cfg)
}

#[tauri::command]
pub fn set_exe_dir(app: AppHandle, id: String, dir: Option<String>) -> Res<()> {
    let mut cfg = load_library(&app);
    match dir {
        Some(d) if Path::new(&d).is_dir() => {
            cfg.exe_overrides.insert(id, d);
        }
        Some(_) => return Err("Folder not found".into()),
        None => {
            cfg.exe_overrides.remove(&id);
        }
    }
    save_library(&app, &cfg)
}
