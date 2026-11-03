//! GitHub release fetcher, downloader, and local package manager for `ShyVortex/dlss-unlocked`.

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;
use walkdir::WalkDir;

use crate::util::{data_dir, is_safe_rel, now_iso, packages_dir, read_json, rel_slash, write_json, ErrStr, Res};

const REPO_API: &str = "https://api.github.com/repos/ShyVortex/dlss-unlocked/releases?per_page=15";
const META_FILE: &str = ".package-meta.json";
const CACHE_FILE: &str = "releases-cache.json";

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseInfo {
    pub tag: String,
    pub name: String,
    pub published_at: String,
    pub body: String,
    pub html_url: String,
    pub asset_name: Option<String>,
    pub download_url: Option<String>,
    pub size: u64,
    pub prerelease: bool,
    pub downloaded: bool,
    pub local_size: u64,
    pub file_count: usize,
    pub imported: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
struct LocalMeta {
    tag: String,
    name: String,
    installed_at: String,
    size: u64,
    file_count: usize,
    imported: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    tag: String,
    /// download | extract | done
    phase: String,
    downloaded: u64,
    total: u64,
    percent: f64,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    published_at: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

pub fn sanitize_tag(tag: &str) -> String {
    let cleaned: String = tag
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' })
        .collect();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "custom-package".to_string()
    } else {
        cleaned
    }
}

/// Locate the root directory inside `<packages>/<tag>` that contains `dxgi.dll`.
pub fn package_root(app: &AppHandle, tag: &str) -> Res<PathBuf> {
    let base = packages_dir(app)?.join(sanitize_tag(tag));
    resolve_payload_root(&base).ok_or_else(|| {
        format!("Package '{tag}' is not downloaded or is missing dxgi.dll. Download or import it on the Packages page.")
    })
}

fn resolve_payload_root(base: &Path) -> Option<PathBuf> {
    if !base.is_dir() {
        return None;
    }
    if base.join("dxgi.dll").is_file() {
        return Some(base.to_path_buf());
    }
    // Check one level deep in case the archive had a top-level wrapper folder.
    if let Ok(rd) = std::fs::read_dir(base) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && p.join("dxgi.dll").is_file() {
                return Some(p);
            }
        }
    }
    None
}

fn dir_stats(dir: &Path) -> (u64, usize) {
    let mut bytes = 0u64;
    let mut files = 0usize;
    for e in WalkDir::new(dir).follow_links(false).into_iter().flatten() {
        if e.file_type().is_file() {
            if e.file_name().to_string_lossy() == META_FILE {
                continue;
            }
            files += 1;
            bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    (bytes, files)
}

fn inspect_local(pkg_dir: &Path, fallback_tag: &str) -> Option<LocalMeta> {
    resolve_payload_root(pkg_dir)?;
    if let Some(m) = read_json::<LocalMeta>(&pkg_dir.join(META_FILE)) {
        if m.file_count > 0 {
            return Some(m);
        }
    }
    let (size, file_count) = dir_stats(pkg_dir);
    Some(LocalMeta {
        tag: fallback_tag.to_string(),
        name: fallback_tag.to_string(),
        installed_at: String::new(),
        size,
        file_count,
        imported: false,
    })
}

fn merge_with_local(app: &AppHandle, mut remote: Vec<ReleaseInfo>) -> Res<Vec<ReleaseInfo>> {
    let pdir = packages_dir(app)?;
    for r in remote.iter_mut() {
        let sub = pdir.join(sanitize_tag(&r.tag));
        if let Some(meta) = inspect_local(&sub, &r.tag) {
            r.downloaded = true;
            r.local_size = meta.size;
            r.file_count = meta.file_count;
            r.imported = meta.imported;
            if r.size == 0 {
                r.size = meta.size;
            }
        }
    }

    // Add any locally present packages (imported or older tags) not in the remote list.
    if let Ok(rd) = std::fs::read_dir(&pdir) {
        for e in rd.flatten() {
            let path = e.path();
            if !path.is_dir() {
                continue;
            }
            let folder_tag = e.file_name().to_string_lossy().to_string();
            let Some(meta) = inspect_local(&path, &folder_tag) else { continue };
            let tag = if meta.tag.is_empty() { folder_tag.clone() } else { meta.tag.clone() };
            if remote.iter().any(|r| sanitize_tag(&r.tag) == sanitize_tag(&tag)) {
                continue;
            }
            remote.insert(
                0,
                ReleaseInfo {
                    tag: tag.clone(),
                    name: if meta.name.is_empty() { format!("Local Package ({tag})") } else { meta.name },
                    published_at: meta.installed_at,
                    body: "Locally imported DLSS Unlocked standalone package.".to_string(),
                    html_url: "https://github.com/ShyVortex/dlss-unlocked/releases".to_string(),
                    asset_name: None,
                    download_url: None,
                    size: meta.size,
                    prerelease: false,
                    downloaded: true,
                    local_size: meta.size,
                    file_count: meta.file_count,
                    imported: meta.imported,
                },
            );
        }
    }

    Ok(remote)
}

fn convert_gh(rel: GhRelease) -> ReleaseInfo {
    // Prefer the standalone .zip asset over the .exe installer.
    let zip_asset = rel
        .assets
        .iter()
        .find(|a| {
            let l = a.name.to_lowercase();
            l.ends_with(".zip") && l.contains("standalone")
        })
        .or_else(|| rel.assets.iter().find(|a| a.name.to_lowercase().ends_with(".zip")));

    ReleaseInfo {
        name: if rel.name.is_empty() { rel.tag_name.clone() } else { rel.name },
        tag: rel.tag_name,
        published_at: rel.published_at,
        body: rel.body,
        html_url: rel.html_url,
        asset_name: zip_asset.map(|a| a.name.clone()),
        download_url: zip_asset.map(|a| a.browser_download_url.clone()),
        size: zip_asset.map(|a| a.size).unwrap_or(0),
        prerelease: rel.prerelease,
        downloaded: false,
        local_size: 0,
        file_count: 0,
        imported: false,
    }
}

/// Extract a `.zip` archive into `dest_dir`, stripping a single top-level directory if all entries share one.
fn extract_zip_blocking(app: &AppHandle, tag: &str, zip_path: &Path, dest_dir: &Path) -> Res<(u64, usize)> {
    let file = std::fs::File::open(zip_path).es()?;
    let mut archive = zip::ZipArchive::new(file).es()?;

    // Determine if all files share a single top-level folder.
    let mut file_names: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        if let Ok(f) = archive.by_index_raw(i) {
            if !f.is_dir() {
                file_names.push(f.name().replace('\\', "/"));
            }
        }
    }
    let common_prefix: Option<String> = file_names
        .first()
        .and_then(|first| first.split_once('/').map(|(top, _)| format!("{top}/")))
        .filter(|prefix| file_names.iter().all(|n| n.starts_with(prefix)));

    if dest_dir.exists() {
        let _ = std::fs::remove_dir_all(dest_dir);
    }
    std::fs::create_dir_all(dest_dir).es()?;

    let total = archive.len();
    let mut total_bytes = 0u64;
    let mut count = 0usize;

    for i in 0..total {
        let mut entry = archive.by_index(i).es()?;
        if entry.is_dir() {
            continue;
        }
        let raw_name = entry.name().replace('\\', "/");
        let rel = match &common_prefix {
            Some(p) => raw_name.strip_prefix(p).unwrap_or(&raw_name),
            None => &raw_name,
        };
        let rel = rel.trim_start_matches('/');
        if !is_safe_rel(rel) {
            continue;
        }
        let out_path = dest_dir.join(rel);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).es()?;
        }
        let mut out_file = std::fs::File::create(&out_path).es()?;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = entry.read(&mut buf).es()?;
            if n == 0 {
                break;
            }
            out_file.write_all(&buf[..n]).es()?;
            total_bytes += n as u64;
        }
        count += 1;
        let pct = if total > 0 { ((i + 1) as f64 / total as f64) * 100.0 } else { 100.0 };
        let _ = app.emit(
            "download-progress",
            DownloadProgress {
                tag: tag.to_string(),
                phase: "extract".into(),
                downloaded: (i + 1) as u64,
                total: total as u64,
                percent: pct,
            },
        );
    }

    if resolve_payload_root(dest_dir).is_none() {
        let _ = std::fs::remove_dir_all(dest_dir);
        return Err("Archive does not contain dxgi.dll (not a valid DLSS Unlocked standalone package)".into());
    }

    Ok((total_bytes, count))
}

fn infer_tag_from_path(p: &Path) -> String {
    let stem = p
        .file_stem()
        .or_else(|| p.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "local".into());
    for prefix in [
        "dlss-unlocked-standalone-",
        "dlss-unlocked-setup-",
        "dlss-unlocked-",
        "dlss_unlocked_",
    ] {
        if let Some(rest) = stem.strip_prefix(prefix) {
            if !rest.is_empty() {
                return sanitize_tag(rest);
            }
        }
    }
    sanitize_tag(&stem)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Return cached releases + locally downloaded packages immediately without network wait.
#[tauri::command]
pub fn get_packages(app: AppHandle) -> Res<Vec<ReleaseInfo>> {
    let cache = data_dir(&app)?.join(CACHE_FILE);
    let cached: Vec<ReleaseInfo> = read_json(&cache).unwrap_or_default();
    merge_with_local(&app, cached)
}

/// Fetch latest releases from GitHub, update cache, and merge with local state.
#[tauri::command]
pub async fn fetch_releases(app: AppHandle) -> Res<Vec<ReleaseInfo>> {
    let client = reqwest::Client::builder()
        .user_agent("dlss-unlocked-manager/0.1.0")
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .es()?;

    let resp = client.get(REPO_API).send().await;
    match resp {
        Ok(r) if r.status().is_success() => {
            let gh: Vec<GhRelease> = r.json().await.es()?;
            let list: Vec<ReleaseInfo> = gh.into_iter().map(convert_gh).collect();
            if let Ok(d) = data_dir(&app) {
                let _ = write_json(&d.join(CACHE_FILE), &list);
            }
            merge_with_local(&app, list)
        }
        Ok(r) => {
            let status = r.status();
            let fallback = get_packages(app)?;
            if !fallback.is_empty() {
                Ok(fallback)
            } else {
                Err(format!("GitHub API returned {status}"))
            }
        }
        Err(e) => {
            let fallback = get_packages(app)?;
            if !fallback.is_empty() {
                Ok(fallback)
            } else {
                Err(format!("Failed to reach GitHub: {e}"))
            }
        }
    }
}

/// Download and extract a release standalone `.zip` package.
#[tauri::command]
pub async fn download_release(app: AppHandle, tag: String, url: String, name: Option<String>) -> Res<ReleaseInfo> {
    let safe_tag = sanitize_tag(&tag);
    let pdir = packages_dir(&app)?;
    let part_path = pdir.join(format!("{safe_tag}.zip.part"));
    let dest_dir = pdir.join(&safe_tag);

    let client = reqwest::Client::builder()
        .user_agent("dlss-unlocked-manager/0.1.0")
        .build()
        .es()?;

    let resp = client.get(&url).send().await.es()?;
    if !resp.status().is_success() {
        return Err(format!("Download failed: HTTP {}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(0);
    let mut downloaded = 0u64;

    let mut file = tokio::fs::File::create(&part_path).await.es()?;
    let mut stream = resp.bytes_stream();
    let mut last_emit = std::time::Instant::now();

    let _ = app.emit(
        "download-progress",
        DownloadProgress {
            tag: tag.clone(),
            phase: "download".into(),
            downloaded: 0,
            total,
            percent: 0.0,
        },
    );

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.es()?;
        file.write_all(&chunk).await.es()?;
        downloaded += chunk.len() as u64;
        if last_emit.elapsed().as_millis() >= 80 || (total > 0 && downloaded >= total) {
            last_emit = std::time::Instant::now();
            let pct = if total > 0 { (downloaded as f64 / total as f64) * 100.0 } else { 0.0 };
            let _ = app.emit(
                "download-progress",
                DownloadProgress {
                    tag: tag.clone(),
                    phase: "download".into(),
                    downloaded,
                    total,
                    percent: pct,
                },
            );
        }
    }
    file.flush().await.es()?;
    drop(file);

    let app2 = app.clone();
    let tag2 = tag.clone();
    let part2 = part_path.clone();
    let dest2 = dest_dir.clone();
    let (local_size, file_count) = tauri::async_runtime::spawn_blocking(move || {
        extract_zip_blocking(&app2, &tag2, &part2, &dest2)
    })
    .await
    .es()??;

    let _ = std::fs::remove_file(&part_path);

    let meta = LocalMeta {
        tag: tag.clone(),
        name: name.clone().unwrap_or_else(|| tag.clone()),
        installed_at: now_iso(),
        size: local_size,
        file_count,
        imported: false,
    };
    let _ = write_json(&dest_dir.join(META_FILE), &meta);

    let _ = app.emit(
        "download-progress",
        DownloadProgress {
            tag: tag.clone(),
            phase: "done".into(),
            downloaded: local_size,
            total: local_size,
            percent: 100.0,
        },
    );

    let all = get_packages(app)?;
    all.into_iter()
        .find(|r| sanitize_tag(&r.tag) == safe_tag)
        .ok_or_else(|| "Package extracted but not found in index".into())
}

/// Import a standalone `.zip` archive or extracted folder from disk.
#[tauri::command]
pub async fn import_package(app: AppHandle, path: String, label: Option<String>) -> Res<ReleaseInfo> {
    let src = PathBuf::from(&path);
    if !src.exists() {
        return Err("Selected path does not exist".into());
    }
    let tag = label
        .as_deref()
        .map(sanitize_tag)
        .filter(|s| !s.is_empty() && s != "custom-package")
        .unwrap_or_else(|| infer_tag_from_path(&src));
    let safe_tag = sanitize_tag(&tag);
    let dest_dir = packages_dir(&app)?.join(&safe_tag);

    let app2 = app.clone();
    let tag2 = tag.clone();
    let dest2 = dest_dir.clone();
    let (local_size, file_count) = tauri::async_runtime::spawn_blocking(move || -> Res<(u64, usize)> {
        if src.is_file() {
            extract_zip_blocking(&app2, &tag2, &src, &dest2)
        } else if src.is_dir() {
            let Some(payload_src) = resolve_payload_root(&src) else {
                return Err("Selected folder does not contain dxgi.dll".into());
            };
            if dest2.exists() {
                let _ = std::fs::remove_dir_all(&dest2);
            }
            std::fs::create_dir_all(&dest2).es()?;
            let mut bytes = 0u64;
            let mut files = 0usize;
            for e in WalkDir::new(&payload_src).follow_links(false).into_iter().flatten() {
                if !e.file_type().is_file() {
                    continue;
                }
                let rel = rel_slash(e.path().strip_prefix(&payload_src).es()?);
                if !is_safe_rel(&rel) {
                    continue;
                }
                let target = dest2.join(&rel);
                if let Some(p) = target.parent() {
                    std::fs::create_dir_all(p).es()?;
                }
                let b = std::fs::copy(e.path(), &target).es()?;
                bytes += b;
                files += 1;
            }
            Ok((bytes, files))
        } else {
            Err("Unsupported path".into())
        }
    })
    .await
    .es()??;

    let meta = LocalMeta {
        tag: tag.clone(),
        name: format!("DLSS Unlocked ({tag})"),
        installed_at: now_iso(),
        size: local_size,
        file_count,
        imported: true,
    };
    let _ = write_json(&dest_dir.join(META_FILE), &meta);

    let all = get_packages(app)?;
    all.into_iter()
        .find(|r| sanitize_tag(&r.tag) == safe_tag)
        .ok_or_else(|| "Imported package not found".into())
}

#[tauri::command]
pub fn delete_package(app: AppHandle, tag: String) -> Res<()> {
    let dir = packages_dir(&app)?.join(sanitize_tag(&tag));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).es()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_release_tag() {
        assert_eq!(
            infer_tag_from_path(Path::new("C:\\Downloads\\dlss-unlocked-standalone-NR-v0.9.80.zip")),
            "NR-v0.9.80"
        );
        assert_eq!(sanitize_tag("../bad/tag"), ".._bad_tag");
        assert_eq!(sanitize_tag(".."), "custom-package");
    }
}
