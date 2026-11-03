pub mod gpu;
pub mod ini;
pub mod installer;
pub mod release;
pub mod scanner;
pub mod util;
pub mod vdf;

use serde::Serialize;
use std::path::Path;
use tauri::AppHandle;
use util::{data_dir, packages_dir, ErrStr, Res};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub data_dir: String,
    pub packages_dir: String,
}

#[tauri::command]
async fn detect_gpus() -> Res<Vec<gpu::GpuInfo>> {
    tauri::async_runtime::spawn_blocking(gpu::detect).await.es()
}

#[tauri::command]
fn get_app_info(app: AppHandle) -> Res<AppInfo> {
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        data_dir: data_dir(&app)?.to_string_lossy().to_string(),
        packages_dir: packages_dir(&app)?.to_string_lossy().to_string(),
    })
}

#[tauri::command]
fn open_folder(path: String) -> Res<()> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("Path does not exist: {path}"));
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer").arg(p).spawn().es()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(p).spawn().es()?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(p).spawn().es()?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            detect_gpus,
            get_app_info,
            open_folder,
            // scanner
            scanner::scan_games,
            scanner::get_cached_games,
            scanner::refresh_game,
            scanner::add_custom_game,
            scanner::remove_game,
            scanner::unhide_all,
            scanner::set_exe_dir,
            // release / packages
            release::get_packages,
            release::fetch_releases,
            release::download_release,
            release::import_package,
            release::delete_package,
            // installer
            installer::preflight,
            installer::install_game,
            installer::uninstall_game,
            installer::install_state,
            // ini editor
            ini::read_ini,
            ini::save_ini_values,
            ini::save_ini_raw,
            ini::restore_ini_backup,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
