//! Comment- and order-preserving INI reader/writer for OptiScaler.ini.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::util::{ErrStr, Res};

#[derive(Serialize, Clone, Debug)]
pub struct IniEntry {
    pub key: String,
    pub value: String,
    /// Comment lines directly above the key (OptiScaler documents each option this way).
    pub desc: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct IniSection {
    pub name: String,
    pub entries: Vec<IniEntry>,
}

#[derive(Serialize)]
pub struct IniDoc {
    pub path: String,
    pub exists: bool,
    pub sections: Vec<IniSection>,
    pub raw: String,
}

#[derive(Deserialize)]
pub struct IniChange {
    pub section: String,
    pub key: String,
    pub value: String,
}

fn section_name(line: &str) -> Option<&str> {
    let t = line.trim();
    (t.len() > 2 && t.starts_with('[') && t.ends_with(']')).then(|| t[1..t.len() - 1].trim())
}

fn is_comment(t: &str) -> bool {
    t.starts_with(';') || t.starts_with('#')
}

fn split_kv(line: &str) -> Option<(&str, &str)> {
    let t = line.trim();
    if t.is_empty() || is_comment(t) || section_name(t).is_some() {
        return None;
    }
    let idx = t.find('=')?;
    Some((t[..idx].trim(), t[idx + 1..].trim()))
}

pub fn parse(text: &str) -> Vec<IniSection> {
    let mut sections: Vec<IniSection> = Vec::new();
    let mut comments: Vec<String> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            comments.clear();
            continue;
        }
        if let Some(name) = section_name(t) {
            sections.push(IniSection { name: name.to_string(), entries: vec![] });
            comments.clear();
            continue;
        }
        if is_comment(t) {
            comments.push(t.trim_start_matches([';', '#']).trim().to_string());
            continue;
        }
        if let Some((k, v)) = split_kv(t) {
            if sections.is_empty() {
                sections.push(IniSection { name: String::new(), entries: vec![] });
            }
            let desc = comments.join("\n");
            comments.clear();
            sections.last_mut().unwrap().entries.push(IniEntry { key: k.to_string(), value: v.to_string(), desc });
        }
    }
    sections
}

/// Update `key=value` pairs in place, keeping comments, ordering and line endings.
/// When `add_missing` is true, absent keys/sections are appended.
pub fn set_values(text: &str, changes: &[(String, String, String)], add_missing: bool) -> String {
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();

    for (section, key, value) in changes {
        let mut cur = String::new();
        let mut found = false;
        let mut section_seen = false;
        let mut last_in_section: Option<usize> = None;
        for (i, line) in lines.iter_mut().enumerate() {
            if let Some(name) = section_name(line) {
                cur = name.to_string();
                if cur.eq_ignore_ascii_case(section) {
                    section_seen = true;
                    last_in_section = Some(i);
                }
                continue;
            }
            if !cur.eq_ignore_ascii_case(section) {
                continue;
            }
            if !line.trim().is_empty() {
                last_in_section = Some(i);
            }
            if let Some((k, _)) = split_kv(line) {
                if k.eq_ignore_ascii_case(key) {
                    let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
                    *line = format!("{indent}{k}={value}");
                    found = true;
                    break;
                }
            }
        }
        if !found && add_missing {
            if section_seen {
                let at = last_in_section.map(|i| i + 1).unwrap_or(lines.len());
                lines.insert(at, format!("{key}={value}"));
            } else {
                if lines.last().map_or(false, |l| !l.trim().is_empty()) {
                    lines.push(String::new());
                }
                lines.push(format!("[{section}]"));
                lines.push(format!("{key}={value}"));
            }
        }
    }

    let mut out = lines.join(nl);
    if text.ends_with('\n') {
        out.push_str(nl);
    }
    out
}

/// Remove `key=` lines that appear outside `keep_section` (cleanup done by the official installer).
pub fn remove_key_outside(text: &str, key: &str, keep_section: &str) -> String {
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut cur = String::new();
    let mut out: Vec<&str> = Vec::new();
    for line in text.lines() {
        if let Some(name) = section_name(line) {
            cur = name.to_string();
        } else if let Some((k, _)) = split_kv(line) {
            if k.eq_ignore_ascii_case(key) && !cur.eq_ignore_ascii_case(keep_section) {
                continue;
            }
        }
        out.push(line);
    }
    let mut s = out.join(nl);
    if text.ends_with('\n') {
        s.push_str(nl);
    }
    s
}

fn ini_path(dir: &str) -> PathBuf {
    Path::new(dir).join("OptiScaler.ini")
}

fn backup_once(path: &Path) {
    let bak = path.with_extension("ini.gui-bak");
    if path.exists() && !bak.exists() {
        let _ = std::fs::copy(path, bak);
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn read_ini(dir: String) -> Res<IniDoc> {
    let path = ini_path(&dir);
    if !path.exists() {
        return Ok(IniDoc { path: path.to_string_lossy().to_string(), exists: false, sections: vec![], raw: String::new() });
    }
    let bytes = std::fs::read(&path).es()?;
    let raw = String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}').to_string();
    Ok(IniDoc { path: path.to_string_lossy().to_string(), exists: true, sections: parse(&raw), raw })
}

#[tauri::command]
pub fn save_ini_values(dir: String, changes: Vec<IniChange>) -> Res<IniDoc> {
    let path = ini_path(&dir);
    let text = std::fs::read_to_string(&path).es()?;
    backup_once(&path);
    let ch: Vec<(String, String, String)> = changes.into_iter().map(|c| (c.section, c.key, c.value)).collect();
    std::fs::write(&path, set_values(&text, &ch, true)).es()?;
    read_ini(dir)
}

#[tauri::command]
pub fn save_ini_raw(dir: String, text: String) -> Res<IniDoc> {
    let path = ini_path(&dir);
    backup_once(&path);
    std::fs::write(&path, text).es()?;
    read_ini(dir)
}

#[tauri::command]
pub fn restore_ini_backup(dir: String) -> Res<IniDoc> {
    let path = ini_path(&dir);
    let bak = path.with_extension("ini.gui-bak");
    if !bak.exists() {
        return Err("No backup found".into());
    }
    std::fs::copy(&bak, &path).es()?;
    read_ini(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "[FrameGen]\r\n; Enable external FG\r\n; true or false\r\nExternal=false\r\n\r\n[DLSSG]\r\nAmpereMfgUnlock=false\r\n";

    #[test]
    fn parses_with_descriptions() {
        let s = parse(SAMPLE);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].entries[0].key, "External");
        assert_eq!(s[0].entries[0].desc, "Enable external FG\ntrue or false");
    }

    #[test]
    fn sets_values_preserving_format() {
        let out = set_values(
            SAMPLE,
            &[
                ("FrameGen".into(), "External".into(), "true".into()),
                ("DLSSG".into(), "AdaMfgUnlock".into(), "true".into()),
                ("New".into(), "K".into(), "1".into()),
            ],
            true,
        );
        assert!(out.contains("External=true\r\n"));
        assert!(out.contains("; Enable external FG\r\n"));
        assert!(out.contains("AmpereMfgUnlock=false\r\nAdaMfgUnlock=true"));
        assert!(out.contains("[New]\r\nK=1"));
    }

    #[test]
    fn removes_rogue_keys() {
        let t = "[A]\nAmpereMfgUnlock=true\n[DLSSG]\nAmpereMfgUnlock=true\n";
        assert_eq!(remove_key_outside(t, "AmpereMfgUnlock", "DLSSG"), "[A]\n[DLSSG]\nAmpereMfgUnlock=true\n");
    }
}
