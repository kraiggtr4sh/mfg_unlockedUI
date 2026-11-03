use serde::Serialize;
use serde_json::Value;

use crate::util::hidden_command;

#[derive(Serialize, Clone, Debug)]
pub struct GpuInfo {
    pub name: String,
    /// nvidia | intel | amd | other
    pub vendor: String,
    pub driver: String,
    pub vram_mb: u64,
    /// turing | ampere | ada | blackwell | gtx | arc | radeon | unknown
    pub arch: String,
    /// Recommended DLSS Unlocked preset:
    /// ampere (dlssg_sm86) | ada (AdaMfgUnlock) | native (RTX 50) | xefg | fsr | none
    pub preset: String,
}

fn parse_rtx_number(upper: &str) -> Option<u32> {
    let idx = upper.find("RTX ")?;
    let digits: String = upper[idx + 4..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.len() == 4 {
        digits.parse().ok()
    } else {
        None
    }
}

pub fn classify(name: &str) -> (String, String, String) {
    let u = name.to_uppercase();
    let (vendor, arch) = if u.contains("NVIDIA") || u.contains("GEFORCE") || u.contains("RTX") || u.contains("QUADRO") {
        let arch = if u.contains(" ADA") {
            "ada"
        } else if u.find("RTX A").map_or(false, |i| u.as_bytes().get(i + 5).map_or(false, |b| b.is_ascii_digit())) {
            "ampere"
        } else if u.contains("QUADRO RTX") || u.contains("TITAN RTX") {
            "turing"
        } else if let Some(n) = parse_rtx_number(&u) {
            match n / 1000 {
                2 => "turing",
                3 => "ampere",
                4 => "ada",
                5 => "blackwell",
                _ => "unknown",
            }
        } else if u.contains("GTX") {
            "gtx"
        } else {
            "unknown"
        };
        ("nvidia", arch)
    } else if u.contains("INTEL") {
        ("intel", if u.contains("ARC") { "arc" } else { "unknown" })
    } else if u.contains("AMD") || u.contains("RADEON") {
        ("amd", "radeon")
    } else {
        ("other", "unknown")
    };

    let preset = match arch {
        "turing" | "ampere" => "ampere",
        "ada" => "ada",
        "blackwell" => "native",
        "arc" => "xefg",
        "radeon" => "fsr",
        _ => "none",
    };
    (vendor.to_string(), arch.to_string(), preset.to_string())
}

fn is_virtual(name: &str) -> bool {
    let l = name.to_lowercase();
    ["microsoft basic", "remote display", "parsec", "virtual", "citrix", "spacedesk", "dummy"]
        .iter()
        .any(|k| l.contains(k))
}

fn nvidia_smi() -> Vec<(String, String, u64)> {
    let out = hidden_command("nvidia-smi")
        .args(["--query-gpu=name,driver_version,memory.total", "--format=csv,noheader,nounits"])
        .output();
    let Ok(out) = out else { return vec![] };
    if !out.status.success() {
        return vec![];
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let parts: Vec<&str> = l.split(',').map(|s| s.trim()).collect();
            if parts.len() < 3 {
                return None;
            }
            Some((parts[0].to_string(), parts[1].to_string(), parts[2].parse().unwrap_or(0)))
        })
        .collect()
}

fn wmi_adapters() -> Vec<(String, String, u64)> {
    let out = hidden_command("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion,AdapterRAM | ConvertTo-Json -Compress",
        ])
        .output();
    let Ok(out) = out else { return vec![] };
    let text = String::from_utf8_lossy(&out.stdout);
    let Ok(v) = serde_json::from_str::<Value>(text.trim()) else { return vec![] };
    let items = match v {
        Value::Array(a) => a,
        o @ Value::Object(_) => vec![o],
        _ => vec![],
    };
    items
        .iter()
        .map(|o| {
            let name = o["Name"].as_str().unwrap_or("").to_string();
            let driver = o["DriverVersion"].as_str().unwrap_or("").to_string();
            let ram = o["AdapterRAM"].as_u64().unwrap_or(0) / (1024 * 1024);
            (name, driver, ram)
        })
        .collect()
}

pub fn detect() -> Vec<GpuInfo> {
    let smi = nvidia_smi();
    let mut gpus: Vec<GpuInfo> = Vec::new();

    for (name, driver, vram) in &smi {
        let (vendor, arch, preset) = classify(name);
        gpus.push(GpuInfo { name: name.clone(), vendor, driver: driver.clone(), vram_mb: *vram, arch, preset });
    }

    for (name, driver, vram) in wmi_adapters() {
        if name.is_empty() || is_virtual(&name) {
            continue;
        }
        let (vendor, arch, preset) = classify(&name);
        // nvidia-smi already gave accurate data for NVIDIA cards.
        if vendor == "nvidia" && !smi.is_empty() {
            continue;
        }
        gpus.push(GpuInfo { name, vendor, driver, vram_mb: vram, arch, preset });
    }

    // Most capable card first: NVIDIA RTX > Intel Arc > others.
    let rank = |g: &GpuInfo| match g.preset.as_str() {
        "native" => 0,
        "ada" => 1,
        "ampere" => 2,
        "xefg" => 3,
        "fsr" => 4,
        _ => 5,
    };
    gpus.sort_by_key(rank);
    gpus
}

#[cfg(test)]
mod tests {
    use super::classify;

    #[test]
    fn maps_presets() {
        assert_eq!(classify("NVIDIA GeForce RTX 3070 Laptop GPU").2, "ampere");
        assert_eq!(classify("NVIDIA GeForce RTX 2060").2, "ampere");
        assert_eq!(classify("NVIDIA GeForce RTX 4090").2, "ada");
        assert_eq!(classify("NVIDIA GeForce RTX 5080").2, "native");
        assert_eq!(classify("NVIDIA RTX A4000").1, "ampere");
        assert_eq!(classify("NVIDIA RTX 2000 Ada Generation").1, "ada");
        assert_eq!(classify("NVIDIA GeForce GTX 1660 SUPER").2, "none");
        assert_eq!(classify("Intel(R) Arc(TM) A770 Graphics").2, "xefg");
        assert_eq!(classify("AMD Radeon RX 7900 XTX").2, "fsr");
    }
}
