const tauri = window.__TAURI__ || {};
const invoke = tauri.core ? tauri.core.invoke : async () => {};
const listen = tauri.event ? tauri.event.listen : async () => () => {};
const dialog = tauri.dialog || {};
const opener = tauri.opener || {};

const state = {
  tab: "games",
  gpus: [],
  appInfo: null,
  games: [],
  filter: "all",
  search: "",
  selectedGameId: null,
  preflight: null,
  packages: [],
  downloadingTag: null,
  downloadProgress: {},
  // INI editor state
  iniDir: "",
  iniDoc: null,
  iniMode: "quick",
  iniSearch: "",
  iniPending: {}, // "Section::Key" -> value
  // Install form options
  installOpts: {
    tag: "",
    proxy: "dxgi",
    preset: "auto",
    streamline: true,
    repatch: false,
    resetConfig: false,
  },
};

function esc(str) {
  return String(str ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function fmtBytes(bytes) {
  if (!bytes || bytes <= 0) return "0 MB";
  const mb = bytes / (1024 * 1024);
  return mb >= 1024 ? `${(mb / 1024).toFixed(2)} GB` : `${mb.toFixed(1)} MB`;
}

function fmtDate(iso) {
  if (!iso) return "";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

function toast(msg, isError = false) {
  const wrap = document.getElementById("toast-container");
  if (!wrap) return;
  const el = document.createElement("div");
  el.className = `toast ${isError ? "error" : ""}`;
  el.textContent = msg;
  wrap.appendChild(el);
  setTimeout(() => el.remove(), 4200);
}

function setActivity(text, pct = null) {
  const bar = document.getElementById("activity-bar");
  const txt = document.getElementById("activity-text");
  const fill = document.getElementById("activity-progress");
  if (!text) {
    bar.classList.add("hidden");
    return;
  }
  bar.classList.remove("hidden");
  txt.textContent = text;
  fill.style.width = `${Math.max(0, Math.min(100, pct ?? 100))}%`;
}

function recommendedPreset() {
  const primary = state.gpus[0];
  if (!primary) return "ampere";
  if (primary.preset === "ada") return "ada";
  if (primary.preset === "ampere") return "ampere";
  return "none";
}

function presetLabel(preset) {
  switch (preset) {
    case "ampere":
      return "RTX 20/30 · dlssg_sm86";
    case "ada":
      return "RTX 40 · AdaMfgUnlock";
    case "native":
      return "RTX 50 · Native MFG";
    case "xefg":
      return "Intel Arc · XeFG";
    case "fsr":
      return "Radeon · FSR 3.1 FG";
    default:
      return "Standard";
  }
}

// ---------------------------------------------------------------------------
// Sidebar & Navigation
// ---------------------------------------------------------------------------

function switchTab(tab) {
  state.tab = tab;
  document.querySelectorAll(".nav-item").forEach((btn) => {
    btn.classList.toggle("active", btn.dataset.tab === tab);
  });
  document.querySelectorAll(".tab-view").forEach((sec) => {
    sec.classList.toggle("active", sec.id === `tab-${tab}`);
  });
  if (tab === "ini") {
    populateIniGameSelect();
    renderIniView();
  } else if (tab === "packages") {
    renderPackages();
  } else if (tab === "guide") {
    renderGuide();
  }
}

function renderGpuCard() {
  const nameEl = document.getElementById("gpu-name");
  const metaEl = document.getElementById("gpu-meta");
  const badgeEl = document.getElementById("gpu-preset-badge");

  if (!state.gpus.length) {
    nameEl.textContent = "No Discrete GPU Found";
    metaEl.textContent = "Defaulting to Ampere SM86 preset";
    badgeEl.textContent = "Generic";
    return;
  }
  const g = state.gpus[0];
  nameEl.textContent = g.name;
  const vram = g.vram_mb ? `${(g.vram_mb / 1024).toFixed(0)} GB VRAM` : "";
  const drv = g.driver ? `Driver ${g.driver}` : "";
  metaEl.textContent = [g.arch.toUpperCase(), vram, drv].filter(Boolean).join(" · ");
  badgeEl.textContent = presetLabel(g.preset);
}

// ---------------------------------------------------------------------------
// Games Library & Inspector
// ---------------------------------------------------------------------------

function filteredGames() {
  const q = state.search.trim().toLowerCase();
  return state.games.filter((g) => {
    if (state.filter === "dlss" && !g.features.dlss && !g.features.dlssg && !g.features.streamline) return false;
    if (state.filter === "installed" && !g.install.installed) return false;
    if (["steam", "epic", "gog", "custom"].includes(state.filter) && g.source !== state.filter) return false;
    if (q) {
      const hay = `${g.name} ${g.source} ${g.install_dir} ${g.exe_dir || ""}`.toLowerCase();
      if (!hay.includes(q)) return false;
    }
    return true;
  });
}

function initials(name) {
  return (
    name
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 3)
      .map((w) => w[0])
      .join("")
      .toUpperCase() || "GM"
  );
}

function renderGames() {
  document.getElementById("nav-games-count").textContent = String(state.games.length);
  const grid = document.getElementById("games-grid");
  const list = filteredGames();

  if (!list.length) {
    grid.innerHTML = `
      <div class="insp-box" style="grid-column: 1 / -1; padding: 28px; text-align: center; align-items: center;">
        <h3>No matching games found</h3>
        <p style="color: var(--text-muted); max-width: 420px; margin: 6px 0 12px;">
          Click <strong>Rescan Library</strong> to scan Steam, Epic Games, and GOG, or add any game folder manually.
        </p>
      </div>
    `;
    return;
  }

  grid.innerHTML = list
    .map((g) => {
      const f = g.features;
      const inst = g.install;
      const selected = g.id === state.selectedGameId ? "selected" : "";
      const chips = [];
      if (f.dlss) chips.push(`<span class="chip dlss">DLSS</span>`);
      if (f.dlssg) chips.push(`<span class="chip dlssg">DLSS-G</span>`);
      if (f.dlssd) chips.push(`<span class="chip dlssg">Ray Recon</span>`);
      if (f.streamline) chips.push(`<span class="chip">Streamline</span>`);
      if (f.unreal) chips.push(`<span class="chip engine">Unreal</span>`);
      if (f.re_engine) chips.push(`<span class="chip engine">RE Engine</span>`);
      if (f.anti_cheat) chips.push(`<span class="chip warn">${esc(f.anti_cheat)}</span>`);
      if (!chips.length) chips.push(`<span class="chip">DX12 / Manual</span>`);

      const coverHtml = g.cover
        ? `<img src="${esc(g.cover)}" alt="" loading="lazy" onerror="this.remove()" /><span class="game-cover-fallback">${esc(initials(g.name))}</span>`
        : `<span class="game-cover-fallback">${esc(initials(g.name))}</span>`;

      const ribbon = inst.installed
        ? `<span class="installed-ribbon">${esc(inst.version || "INSTALLED")}</span>`
        : `<span></span>`;

      return `
        <div class="game-card ${selected}" data-game-id="${esc(g.id)}">
          <div class="game-cover">
            ${coverHtml}
            <div class="game-top-badges">
              <span class="store-tag">${esc(g.source)}</span>
              ${ribbon}
            </div>
          </div>
          <div class="game-card-body">
            <div class="game-title" title="${esc(g.name)}">${esc(g.name)}</div>
            <div class="game-exe-sub" title="${esc(g.exe_dir || g.install_dir)}">${esc(g.exe_name || g.install_dir)}</div>
            <div class="feature-chips">${chips.join("")}</div>
          </div>
        </div>
      `;
    })
    .join("");

  grid.querySelectorAll(".game-card").forEach((card) => {
    card.addEventListener("click", () => selectGame(card.dataset.gameId));
  });
}

async function selectGame(id) {
  state.selectedGameId = id;
  renderGames();
  const game = state.games.find((g) => g.id === id);
  if (!game) {
    document.getElementById("inspector-empty").classList.remove("hidden");
    document.getElementById("inspector-content").classList.add("hidden");
    return;
  }

  // Default install options tailored to the game
  if (game.install.installed) {
    state.installOpts.proxy = game.install.proxy || "dxgi";
    state.installOpts.streamline = game.install.streamline ?? true;
    state.installOpts.repatch = game.install.repatch ?? game.features.re_engine;
    state.installOpts.preset = game.install.preset || "keep";
  } else {
    state.installOpts.proxy = "dxgi";
    state.installOpts.streamline = true;
    state.installOpts.repatch = Boolean(game.features.re_engine);
    state.installOpts.preset = "auto";
  }

  const downloadedPkgs = state.packages.filter((p) => p.downloaded);
  if (downloadedPkgs.length && !downloadedPkgs.some((p) => p.tag === state.installOpts.tag)) {
    state.installOpts.tag = downloadedPkgs[0].tag;
  }

  await refreshPreflight(game);
  renderInspector();
}

async function refreshPreflight(game) {
  const dir = game.exe_dir || game.install_dir;
  try {
    state.preflight = await invoke("preflight", { dir, proxy: state.installOpts.proxy });
  } catch {
    state.preflight = null;
  }
}

function renderInspector() {
  const emptyEl = document.getElementById("inspector-empty");
  const contentEl = document.getElementById("inspector-content");
  const game = state.games.find((g) => g.id === state.selectedGameId);
  if (!game) {
    emptyEl.classList.remove("hidden");
    contentEl.classList.add("hidden");
    return;
  }
  emptyEl.classList.add("hidden");
  contentEl.classList.remove("hidden");

  const targetDir = game.exe_dir || game.install_dir;
  const downloadedPkgs = state.packages.filter((p) => p.downloaded);
  const pf = state.preflight;

  const pkgOptions = downloadedPkgs.length
    ? downloadedPkgs
        .map(
          (p) =>
            `<option value="${esc(p.tag)}" ${p.tag === state.installOpts.tag ? "selected" : ""}>${esc(p.name)} (${esc(p.tag)})</option>`
        )
        .join("")
    : `<option value="">No package downloaded yet — click Packages tab</option>`;

  const resolvedAuto = recommendedPreset();
  const presetOptions = [
    { val: "auto", label: `Auto Recommended (${presetLabel(resolvedAuto)})` },
    { val: "ampere", label: "RTX 20 / 30 Series (AmpereMfgUnlock / dlssg_sm86)" },
    { val: "ada", label: "RTX 40 Series (AdaMfgUnlock)" },
    { val: "none", label: "Intel Arc / AMD / RTX 50 (No MFG Unlock Override)" },
    { val: "keep", label: "Keep Existing OptiScaler.ini Settings" },
  ];

  const banners = [];
  if (game.features.anti_cheat) {
    banners.push(`
      <div class="banner banner-danger">
        <strong>Anti-Cheat Detected: ${esc(game.features.anti_cheat)}</strong>
        <span>Installing proxy DLLs in online anti-cheat games may trigger bans or block launch. Use only in offline/single-player modes.</span>
      </div>
    `);
  }
  if (pf && pf.running && pf.running.length) {
    banners.push(`
      <div class="banner banner-danger">
        <strong>Game Currently Running</strong>
        <span>Close ${esc(pf.running.join(", "))} before installing or uninstalling.</span>
      </div>
    `);
  }
  if (pf && pf.conflicts && pf.conflicts.length) {
    banners.push(`
      <div class="banner banner-warn">
        <strong>Existing Proxy DLLs Found: ${esc(pf.conflicts.join(", "))}</strong>
        <span>Any overwritten DLL will be safely backed up as <code>*.dlss-unlocked.bak</code> and restored on uninstall.</span>
      </div>
    `);
  }
  if (game.install.installed) {
    banners.push(`
      <div class="banner banner-ok">
        <strong>DLSS Unlocked Installed ${game.install.version ? `(${esc(game.install.version)})` : ""}</strong>
        <span>Proxy: <code>${esc(game.install.proxy || "dxgi")}.dll</code> · ${game.install.fileCount} managed files</span>
      </div>
    `);
  }

  contentEl.innerHTML = `
    <div class="insp-header">
      <div class="insp-title-row">
        <div class="insp-title">${esc(game.name)}</div>
        <span class="store-tag">${esc(game.source)}</span>
      </div>
      <div style="font-size: 11.5px; color: var(--text-muted);">
        ${game.exe_name ? `Detected Executable: <code>${esc(game.exe_name)}</code>` : "Executable folder"}
        ${game.exe_dir_overridden ? `<span class="chip warn" style="margin-left:6px;">Custom Exe Path</span>` : ""}
      </div>
    </div>

    ${banners.join("")}

    <div class="insp-box">
      <div class="insp-box-title">
        <span>Target Executable Directory</span>
        <div style="display:flex; gap:4px;">
          <button class="btn btn-sm" id="insp-browse-exe">Change...</button>
          ${game.exe_dir_overridden ? `<button class="btn btn-sm" id="insp-reset-exe">Reset</button>` : ""}
          <button class="btn btn-sm" id="insp-open-dir">Open</button>
        </div>
      </div>
      <div class="path-display">${esc(targetDir)}</div>
    </div>

    <div class="insp-box">
      <div class="insp-box-title">
        <span>Installation Options</span>
      </div>

      <div class="form-row">
        <label>DLSS Unlocked Package</label>
        <div style="display:flex; gap:6px;">
          <select id="insp-pkg" class="select-input" style="flex:1;">${pkgOptions}</select>
          ${
            !downloadedPkgs.length
              ? `<button class="btn btn-sm btn-primary" id="insp-get-pkg">Get Package</button>`
              : ""
          }
        </div>
      </div>

      <div class="form-row">
        <label>GPU MFG Unlock Preset</label>
        <select id="insp-preset" class="select-input">
          ${presetOptions
            .map(
              (o) =>
                `<option value="${o.val}" ${state.installOpts.preset === o.val ? "selected" : ""}>${esc(o.label)}</option>`
            )
            .join("")}
        </select>
      </div>

      <div class="form-row">
        <label>Proxy Hook DLL</label>
        <select id="insp-proxy" class="select-input">
          <option value="dxgi" ${state.installOpts.proxy === "dxgi" ? "selected" : ""}>dxgi.dll (Recommended Default)</option>
          <option value="version" ${state.installOpts.proxy === "version" ? "selected" : ""}>version.dll</option>
          <option value="winmm" ${state.installOpts.proxy === "winmm" ? "selected" : ""}>winmm.dll</option>
          <option value="asi" ${state.installOpts.proxy === "asi" ? "selected" : ""}>plugins/dlss-unlocked.asi (ASI Loader)</option>
        </select>
      </div>

      <label class="check-row">
        <input type="checkbox" id="insp-streamline" ${state.installOpts.streamline ? "checked" : ""} />
        <div>
          <span>Deploy Bundled NVIDIA Streamline DLLs</span>
          <small>Installs OptiScaler/streamline interposer &amp; DLSS-G modules</small>
        </div>
      </label>

      <label class="check-row">
        <input type="checkbox" id="insp-repatch" ${state.installOpts.repatch ? "checked" : ""} />
        <div>
          <span>Install REPatch (<code>dinput8.dll</code>)</span>
          <small>Required for Capcom RE Engine titles (Resident Evil, Monster Hunter, Dragon's Dogma 2)</small>
        </div>
      </label>

      ${
        game.install.installed
          ? `
      <label class="check-row">
        <input type="checkbox" id="insp-reset-cfg" ${state.installOpts.resetConfig ? "checked" : ""} />
        <div>
          <span>Reset <code>OptiScaler.ini</code> to Package Defaults</span>
          <small>Leave unchecked to preserve your existing per-game INI tuning</small>
        </div>
      </label>`
          : ""
      }
    </div>

    <div class="insp-actions">
      <button class="btn btn-primary" id="insp-btn-install" ${!downloadedPkgs.length ? "disabled" : ""}>
        ${game.install.installed ? "Upgrade / Reapply DLSS Unlocked" : "Install DLSS Unlocked"}
      </button>

      <div class="insp-actions-row">
        <button class="btn" id="insp-btn-ini" ${!(pf && pf.hasIni) && !game.install.installed ? "disabled" : ""}>
          Tune OptiScaler.ini
        </button>
        ${
          game.install.installed
            ? `<button class="btn btn-danger" id="insp-btn-uninstall">Uninstall Mod</button>`
            : `<button class="btn" id="insp-btn-hide">${game.source === "custom" ? "Remove Game" : "Hide Game"}</button>`
        }
      </div>
    </div>
  `;

  // Wire inspector events
  document.getElementById("insp-open-dir")?.addEventListener("click", () => {
    invoke("open_folder", { path: targetDir }).catch((e) => toast(e, true));
  });

  document.getElementById("insp-browse-exe")?.addEventListener("click", async () => {
    if (!dialog.open) return;
    const chosen = await dialog.open({ directory: true, defaultPath: targetDir, title: "Select Folder Containing Game .exe" });
    if (chosen) {
      try {
        await invoke("set_exe_dir", { id: game.id, dir: chosen });
        const updated = await invoke("refresh_game", { game: { ...game, exe_dir: chosen } });
        replaceGameInState(updated);
        await selectGame(updated.id);
        toast("Executable directory updated");
      } catch (e) {
        toast(e, true);
      }
    }
  });

  document.getElementById("insp-reset-exe")?.addEventListener("click", async () => {
    try {
      await invoke("set_exe_dir", { id: game.id, dir: null });
      const updated = await invoke("refresh_game", { game: { ...game, exe_dir: null } });
      replaceGameInState(updated);
      await selectGame(updated.id);
      toast("Executable directory reset to auto-detected path");
    } catch (e) {
      toast(e, true);
    }
  });

  document.getElementById("insp-get-pkg")?.addEventListener("click", () => switchTab("packages"));

  document.getElementById("insp-pkg")?.addEventListener("change", (e) => {
    state.installOpts.tag = e.target.value;
  });
  document.getElementById("insp-preset")?.addEventListener("change", (e) => {
    state.installOpts.preset = e.target.value;
  });
  document.getElementById("insp-proxy")?.addEventListener("change", async (e) => {
    state.installOpts.proxy = e.target.value;
    await refreshPreflight(game);
    renderInspector();
  });
  document.getElementById("insp-streamline")?.addEventListener("change", (e) => {
    state.installOpts.streamline = e.target.checked;
  });
  document.getElementById("insp-repatch")?.addEventListener("change", (e) => {
    state.installOpts.repatch = e.target.checked;
  });
  document.getElementById("insp-reset-cfg")?.addEventListener("change", (e) => {
    state.installOpts.resetConfig = e.target.checked;
  });

  document.getElementById("insp-btn-install")?.addEventListener("click", () => runInstall(game));
  document.getElementById("insp-btn-uninstall")?.addEventListener("click", () => runUninstall(game));
  document.getElementById("insp-btn-ini")?.addEventListener("click", () => {
    state.iniDir = targetDir;
    switchTab("ini");
    loadIniForDir(targetDir);
  });
  document.getElementById("insp-btn-hide")?.addEventListener("click", async () => {
    try {
      await invoke("remove_game", { id: game.id });
      state.games = state.games.filter((g) => g.id !== game.id);
      state.selectedGameId = null;
      renderGames();
      renderInspector();
      toast("Game removed from library view");
    } catch (e) {
      toast(e, true);
    }
  });
}

function replaceGameInState(updated) {
  const idx = state.games.findIndex((g) => g.id === updated.id);
  if (idx >= 0) {
    state.games[idx] = updated;
  } else {
    state.games.unshift(updated);
  }
}

async function runInstall(game) {
  const dir = game.exe_dir || game.install_dir;
  const tag = state.installOpts.tag;
  if (!tag) {
    toast("Please download or import a DLSS Unlocked package first", true);
    switchTab("packages");
    return;
  }
  const effectivePreset = state.installOpts.preset === "auto" ? recommendedPreset() : state.installOpts.preset;
  try {
    setActivity(`Installing DLSS Unlocked (${tag}) into ${game.name}...`, 10);
    const res = await invoke("install_game", {
      dir,
      tag,
      options: {
        proxy: state.installOpts.proxy,
        streamline: state.installOpts.streamline,
        repatch: state.installOpts.repatch,
        resetConfig: state.installOpts.resetConfig,
        preset: effectivePreset,
      },
    });
    const updated = await invoke("refresh_game", { game });
    replaceGameInState(updated);
    await selectGame(updated.id);
    toast(`Installed ${res.files} files into ${game.name}`);
  } catch (e) {
    toast(`Install failed: ${e}`, true);
  } finally {
    setActivity(null);
  }
}

async function runUninstall(game) {
  const dir = game.exe_dir || game.install_dir;
  try {
    setActivity(`Uninstalling DLSS Unlocked from ${game.name}...`, 50);
    const count = await invoke("uninstall_game", { dir });
    const updated = await invoke("refresh_game", { game });
    replaceGameInState(updated);
    await selectGame(updated.id);
    toast(`Uninstalled ${count} files and restored backups`);
  } catch (e) {
    toast(`Uninstall failed: ${e}`, true);
  } finally {
    setActivity(null);
  }
}

// ---------------------------------------------------------------------------
// OptiScaler.ini Studio & Editor
// ---------------------------------------------------------------------------

const QUICK_CONTROLS = [
  {
    card: "Multi-Frame Generation (MFG & DLSS-G)",
    badge: "FrameGen / DLSSG",
    items: [
      {
        section: "FrameGen",
        key: "External",
        title: "External Frame Generation Bridge",
        desc: "Enable OptiScaler external frame generation bridge (required for DLSS-G / MFG unlock)",
        type: "bool",
      },
      {
        section: "FrameGen",
        key: "FGType",
        title: "Frame Generation Backend (FGType)",
        desc: "Select frame generation provider (auto, dlssg, xefg, nukems, fsr31)",
        type: "select",
        options: [
          { v: "auto", l: "auto (Default)" },
          { v: "dlssg", l: "dlssg (NVIDIA DLSS-G / MFG)" },
          { v: "xefg", l: "xefg (Intel XeFG)" },
          { v: "nukems", l: "nukems (dlssg-to-fsr3)" },
        ],
      },
      {
        section: "DLSSG",
        key: "AmpereMfgUnlock",
        title: "Turing / Ampere MFG Unlocker (dlssg_sm86)",
        desc: "Unlock native DLSS FrameGen & Multi-FrameGen (2X/3X/4X) on RTX 20 & RTX 30 series GPUs",
        type: "bool",
      },
      {
        section: "DLSSG",
        key: "AdaMfgUnlock",
        title: "Ada Lovelace MFG Unlocker (RTX 40 Series)",
        desc: "Unlock 3X / 4X Multi-Frame Generation on GeForce RTX 40 series GPUs",
        type: "bool",
      },
    ],
  },
  {
    card: "Upscalers & Neural Rendering (DLSS-NR)",
    badge: "Upscalers / DLSS",
    items: [
      {
        section: "Upscalers",
        key: "Dx12Upscaler",
        title: "DirectX 12 Upscaler Backend",
        desc: "Primary upscaler for DX12 games (auto, dlss, xess, fsr22, fsr31)",
        type: "select",
        options: [
          { v: "auto", l: "auto (Default)" },
          { v: "dlss", l: "dlss (NVIDIA DLSS Super Resolution)" },
          { v: "xess", l: "xess (Intel XeSS)" },
          { v: "fsr31", l: "fsr31 (AMD FSR 3.1)" },
          { v: "fsr22", l: "fsr22 (AMD FSR 2.2)" },
        ],
      },
      {
        section: "DLSS",
        key: "Enabled",
        title: "Enable DLSS Overrides",
        desc: "Allow OptiScaler to manage DLSS presets and Neural Rendering hooks",
        type: "bool",
      },
      {
        section: "DLSS",
        key: "RenderPresetOverride",
        title: "DLSS Render Preset Override",
        desc: "Force latest Transformer / CNN render presets across quality modes",
        type: "bool",
      },
      {
        section: "Sharpness",
        key: "OverrideSharpness",
        title: "Override Image Sharpness",
        desc: "Enable custom RCAS / sharpening pass after upscaling",
        type: "bool",
      },
    ],
  },
  {
    card: "In-Game Overlay & Linux / Proton Compatibility",
    badge: "Menu / Spoofing",
    items: [
      {
        section: "Menu",
        key: "OverlayMenu",
        title: "In-Game ImGui Overlay Menu (Insert key)",
        desc: "Show in-game tuning menu. Disable if a Vulkan/Proton title experiences device-lost crashes",
        type: "bool",
      },
      {
        section: "Spoofing",
        key: "SpoofedGPUName",
        title: "Spoofed GPU Name",
        desc: "Optional GPU name spoofing for games that lock DLSS/MFG menus in UI",
        type: "text",
      },
      {
        section: "NvApi",
        key: "OverrideNvapiDll",
        title: "Override NVAPI DLL",
        desc: "Allow OptiScaler to hook NVAPI calls for Reflex & Frame Generation telemetry",
        type: "bool",
      },
    ],
  },
];

function populateIniGameSelect() {
  const sel = document.getElementById("ini-game-select");
  const installedOrAll = state.games.filter((g) => g.exe_dir || g.install_dir);
  const opts = [`<option value="">Select game folder</option>`];
  for (const g of installedOrAll) {
    const dir = g.exe_dir || g.install_dir;
    const tag = g.install.installed ? " [Installed]" : "";
    const selected = dir === state.iniDir ? "selected" : "";
    opts.push(`<option value="${esc(dir)}" ${selected}>${esc(g.name)}${tag}</option>`);
  }
  if (state.iniDir && !installedOrAll.some((g) => (g.exe_dir || g.install_dir) === state.iniDir)) {
    opts.push(`<option value="${esc(state.iniDir)}" selected>${esc(state.iniDir)}</option>`);
  }
  sel.innerHTML = opts.join("");
}

async function loadIniForDir(dir) {
  state.iniDir = dir;
  state.iniPending = {};
  populateIniGameSelect();
  if (!dir) {
    state.iniDoc = null;
    renderIniView();
    return;
  }
  try {
    state.iniDoc = await invoke("read_ini", { dir });
    renderIniView();
  } catch (e) {
    toast(`Could not read OptiScaler.ini: ${e}`, true);
  }
}

function getIniValue(section, key, fallback = "auto") {
  const pendKey = `${section}::${key}`;
  if (pendKey in state.iniPending) return state.iniPending[pendKey];
  if (!state.iniDoc || !state.iniDoc.sections) return fallback;
  const sec = state.iniDoc.sections.find((s) => s.name.toLowerCase() === section.toLowerCase());
  if (!sec) return fallback;
  const ent = sec.entries.find((e) => e.key.toLowerCase() === key.toLowerCase());
  return ent ? ent.value : fallback;
}

function setIniPending(section, key, value) {
  state.iniPending[`${section}::${key}`] = value;
  const saveBtn = document.getElementById("btn-ini-save");
  const count = Object.keys(state.iniPending).length;
  saveBtn.textContent = count > 0 ? `Save Changes (${count})` : "Save Changes";
}

function renderIniView() {
  const body = document.getElementById("ini-body");
  const searchWrap = document.getElementById("ini-search-wrap");
  searchWrap.style.display = state.iniMode === "sections" ? "block" : "none";

  if (!state.iniDir || !state.iniDoc) {
    body.innerHTML = `
      <div class="insp-box" style="padding: 32px; text-align: center; align-items: center;">
        <h3>Select a Game with OptiScaler.ini</h3>
        <p style="color: var(--text-muted); max-width: 440px; margin-top: 6px;">
          Choose an installed game from the dropdown above or click <strong>Browse Folder...</strong> to open any <code>OptiScaler.ini</code> file.
        </p>
      </div>
    `;
    return;
  }

  if (!state.iniDoc.exists) {
    body.innerHTML = `
      <div class="insp-box" style="padding: 32px; text-align: center; align-items: center;">
        <h3>No <code>OptiScaler.ini</code> found in this folder</h3>
        <div class="path-display" style="margin: 8px 0;">${esc(state.iniDoc.path)}</div>
        <p style="color: var(--text-muted);">Install DLSS Unlocked to this game first so <code>OptiScaler.ini</code> is deployed.</p>
      </div>
    `;
    return;
  }

  if (state.iniMode === "raw") {
    body.innerHTML = `<textarea id="ini-raw-textarea" class="ini-raw-editor" spellcheck="false">${esc(state.iniDoc.raw)}</textarea>`;
    return;
  }

  if (state.iniMode === "quick") {
    body.innerHTML = `
      <div class="studio-grid">
        ${QUICK_CONTROLS.map(
          (group) => `
          <div class="studio-card">
            <h3>
              <span>${esc(group.card)}</span>
            </h3>
            ${group.items
              .map((item) => {
                const cur = getIniValue(item.section, item.key, "auto");
                let controlHtml = "";
                if (item.type === "bool") {
                  controlHtml = `
                    <select class="select-input ini-quick-input" data-sec="${esc(item.section)}" data-key="${esc(item.key)}">
                      <option value="auto" ${cur === "auto" ? "selected" : ""}>auto</option>
                      <option value="true" ${cur === "true" ? "selected" : ""}>true (Enabled)</option>
                      <option value="false" ${cur === "false" ? "selected" : ""}>false (Disabled)</option>
                    </select>
                  `;
                } else if (item.type === "select") {
                  controlHtml = `
                    <select class="select-input ini-quick-input" data-sec="${esc(item.section)}" data-key="${esc(item.key)}">
                      ${item.options
                        .map((o) => `<option value="${esc(o.v)}" ${cur === o.v ? "selected" : ""}>${esc(o.l)}</option>`)
                        .join("")}
                    </select>
                  `;
                } else {
                  controlHtml = `
                    <input type="text" class="text-input ini-quick-input" data-sec="${esc(item.section)}" data-key="${esc(item.key)}" value="${esc(cur)}" style="width:160px;" />
                  `;
                }
                return `
                  <div class="studio-field">
                    <div class="studio-field-info">
                      <span class="studio-field-name">${esc(item.title)}</span>
                      <span class="studio-field-desc">${esc(item.desc)}</span>
                    </div>
                    ${controlHtml}
                  </div>
                `;
              })
              .join("")}
          </div>
        `
        ).join("")}
      </div>
    `;

    body.querySelectorAll(".ini-quick-input").forEach((el) => {
      el.addEventListener("change", () => setIniPending(el.dataset.sec, el.dataset.key, el.value));
      el.addEventListener("input", () => setIniPending(el.dataset.sec, el.dataset.key, el.value));
    });
    return;
  }

  // All INI Sections mode
  const q = state.iniSearch.trim().toLowerCase();
  const sectionsHtml = state.iniDoc.sections
    .map((sec) => {
      const entries = sec.entries.filter((e) => {
        if (!q) return true;
        return (
          sec.name.toLowerCase().includes(q) ||
          e.key.toLowerCase().includes(q) ||
          e.desc.toLowerCase().includes(q) ||
          e.value.toLowerCase().includes(q)
        );
      });
      if (!entries.length) return "";
      return `
        <div class="ini-section-card">
          <div class="ini-section-title">[${esc(sec.name)}]</div>
          ${entries
            .map((e) => {
              const cur = getIniValue(sec.name, e.key, e.value);
              const isBool = ["true", "false", "auto"].includes(cur.toLowerCase());
              const inputHtml = isBool
                ? `<select class="select-input ini-sec-input" data-sec="${esc(sec.name)}" data-key="${esc(e.key)}">
                    <option value="auto" ${cur === "auto" ? "selected" : ""}>auto</option>
                    <option value="true" ${cur === "true" ? "selected" : ""}>true</option>
                    <option value="false" ${cur === "false" ? "selected" : ""}>false</option>
                   </select>`
                : `<input type="text" class="text-input ini-sec-input" data-sec="${esc(sec.name)}" data-key="${esc(e.key)}" value="${esc(cur)}" />`;
              return `
                <div class="ini-entry-row">
                  <div class="ini-key">${esc(e.key)}</div>
                  <div class="ini-desc">${esc(e.desc || "OptiScaler setting")}</div>
                  ${inputHtml}
                </div>
              `;
            })
            .join("")}
        </div>
      `;
    })
    .filter(Boolean)
    .join("");

  body.innerHTML = sectionsHtml || `<div class="insp-box">No INI keys match "${esc(state.iniSearch)}".</div>`;
  body.querySelectorAll(".ini-sec-input").forEach((el) => {
    el.addEventListener("change", () => setIniPending(el.dataset.sec, el.dataset.key, el.value));
    el.addEventListener("input", () => setIniPending(el.dataset.sec, el.dataset.key, el.value));
  });
}

async function saveIni() {
  if (!state.iniDir || !state.iniDoc || !state.iniDoc.exists) return;
  try {
    if (state.iniMode === "raw") {
      const rawText = document.getElementById("ini-raw-textarea")?.value ?? state.iniDoc.raw;
      state.iniDoc = await invoke("save_ini_raw", { dir: state.iniDir, text: rawText });
      toast("Saved raw OptiScaler.ini");
      return;
    }
    const changes = Object.entries(state.iniPending).map(([k, value]) => {
      const [section, key] = k.split("::");
      return { section, key, value };
    });
    if (!changes.length) {
      toast("No pending changes to save");
      return;
    }
    state.iniDoc = await invoke("save_ini_values", { dir: state.iniDir, changes });
    state.iniPending = {};
    document.getElementById("btn-ini-save").textContent = "Save Changes";
    renderIniView();
    toast(`Saved ${changes.length} setting(s) to OptiScaler.ini`);
  } catch (e) {
    toast(`Failed to save INI: ${e}`, true);
  }
}

// ---------------------------------------------------------------------------
// Packages & Releases
// ---------------------------------------------------------------------------

function updatePkgBadge() {
  const downloaded = state.packages.filter((p) => p.downloaded).length;
  document.getElementById("nav-pkg-count").textContent = String(downloaded);
}

function renderPackages() {
  updatePkgBadge();
  const wrap = document.getElementById("packages-list");
  if (!state.packages.length) {
    wrap.innerHTML = `
      <div class="insp-box" style="padding: 32px; text-align: center; align-items: center;">
        <h3>No Packages Loaded</h3>
        <p style="color: var(--text-muted); max-width: 460px; margin: 6px 0 12px;">
          Click <strong>Check GitHub Releases</strong> to fetch official releases from <code>ShyVortex/dlss-unlocked</code>, or import a standalone <code>.zip</code> file from disk.
        </p>
      </div>
    `;
    return;
  }

  wrap.innerHTML = state.packages
    .map((p) => {
      const prog = state.downloadProgress[p.tag];
      const isBusy = state.downloadingTag === p.tag;
      return `
        <div class="pkg-card ${p.downloaded ? "installed" : ""}">
          <div class="pkg-top">
            <div class="pkg-title-group">
              <span class="pkg-tag">${esc(p.tag)}</span>
              <span class="pkg-name">${esc(p.name)}</span>
              ${p.downloaded ? `<span class="installed-ribbon">READY (${p.fileCount} files)</span>` : ""}
              ${p.imported ? `<span class="chip engine">Local Import</span>` : ""}
            </div>
            <div style="display:flex; gap:8px;">
              ${
                p.downloadUrl && !p.downloaded
                  ? `<button class="btn btn-primary btn-dl-pkg" data-tag="${esc(p.tag)}" ${isBusy ? "disabled" : ""}>
                      ${isBusy ? "Downloading..." : `Download Standalone (${fmtBytes(p.size)})`}
                     </button>`
                  : ""
              }
              ${
                p.downloaded
                  ? `<button class="btn btn-danger btn-del-pkg" data-tag="${esc(p.tag)}">Delete Cached Copy</button>`
                  : ""
              }
            </div>
          </div>

          <div class="pkg-meta">
            ${p.publishedAt ? `<span>Published: ${esc(fmtDate(p.publishedAt))}</span>` : ""}
            ${p.assetName ? `<span>Asset: <code>${esc(p.assetName)}</code></span>` : ""}
            <span>Size: ${fmtBytes(p.localSize || p.size)}</span>
          </div>

          ${
            prog
              ? `<div class="banner banner-ok">
                  <strong>${prog.phase === "extract" ? "Extracting archive..." : "Downloading package..."} (${prog.percent.toFixed(1)}%)</strong>
                  <span>${
                    prog.phase === "extract"
                      ? `${prog.downloaded} / ${prog.total} files`
                      : `${fmtBytes(prog.downloaded)} / ${fmtBytes(prog.total)}`
                  }</span>
                 </div>`
              : ""
          }

          ${p.body ? `<div class="pkg-notes">${esc(p.body)}</div>` : ""}
        </div>
      `;
    })
    .join("");

  wrap.querySelectorAll(".btn-dl-pkg").forEach((btn) => {
    btn.addEventListener("click", () => {
      const pkg = state.packages.find((x) => x.tag === btn.dataset.tag);
      if (pkg && pkg.downloadUrl) startDownload(pkg);
    });
  });

  wrap.querySelectorAll(".btn-del-pkg").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const tag = btn.dataset.tag;
      try {
        await invoke("delete_package", { tag });
        state.packages = await invoke("get_packages");
        renderPackages();
        renderInspector();
        toast(`Removed cached package ${tag}`);
      } catch (e) {
        toast(e, true);
      }
    });
  });
}

async function startDownload(pkg) {
  state.downloadingTag = pkg.tag;
  renderPackages();
  try {
    setActivity(`Downloading ${pkg.tag}...`, 2);
    await invoke("download_release", { tag: pkg.tag, url: pkg.downloadUrl, name: pkg.name });
    delete state.downloadProgress[pkg.tag];
    state.packages = await invoke("get_packages");
    if (!state.installOpts.tag) state.installOpts.tag = pkg.tag;
    renderPackages();
    renderInspector();
    toast(`Package ${pkg.tag} downloaded and ready!`);
  } catch (e) {
    toast(`Download error: ${e}`, true);
  } finally {
    state.downloadingTag = null;
    setActivity(null);
    renderPackages();
  }
}

// ---------------------------------------------------------------------------
// Guide & System View
// ---------------------------------------------------------------------------

function renderGuide() {
  const wrap = document.getElementById("guide-grid");
  const gpusHtml = state.gpus.length
    ? state.gpus
        .map(
          (g) => `
        <div class="studio-field">
          <div class="studio-field-info">
            <span class="studio-field-name">${esc(g.name)}</span>
            <span class="studio-field-desc">Vendor: ${esc(g.vendor.toUpperCase())} · Arch: ${esc(g.arch.toUpperCase())} · Driver: ${esc(g.driver || "N/A")}</span>
          </div>
          <span class="chip dlss">${esc(presetLabel(g.preset))}</span>
        </div>`
        )
        .join("")
    : `<div class="studio-field-desc">No adapters reported</div>`;

  wrap.innerHTML = `
    <div class="studio-card">
      <h3><span>Detected Graphics Hardware</span></h3>
      ${gpusHtml}
      <div class="studio-field">
        <div class="studio-field-info">
          <span class="studio-field-name">App Data Directory</span>
          <span class="studio-field-desc">${esc(state.appInfo?.dataDir || "")}</span>
        </div>
      </div>
    </div>

    <div class="studio-card">
      <h3><span>Hardware Preset Guide</span></h3>
      <div class="studio-field">
        <div class="studio-field-info">
          <span class="studio-field-name">GeForce RTX 20 &amp; 30 Series (Turing / Ampere)</span>
          <span class="studio-field-desc">Uses native <code>dlssg_sm86</code> unlocker (<code>AmpereMfgUnlock=true</code>, <code>External=true</code>) for 2X / 3X / 4X Multi-Frame Generation.</span>
        </div>
      </div>
      <div class="studio-field">
        <div class="studio-field-info">
          <span class="studio-field-name">GeForce RTX 40 Series (Ada Lovelace)</span>
          <span class="studio-field-desc">Uses <code>AdaMfgUnlock=true</code> to unlock 3X &amp; 4X Multi-Frame Generation alongside native DLSS 3 FG.</span>
        </div>
      </div>
      <div class="studio-field">
        <div class="studio-field-info">
          <span class="studio-field-name">Intel Arc &amp; AMD Radeon</span>
          <span class="studio-field-desc">Translates Streamline DLSS-G calls to Intel XeFG (<code>FGType=xefg</code>) or AMD FSR 3.1 Frame Generation via OptiScaler.</span>
        </div>
      </div>
    </div>

    <div class="studio-card">
      <h3><span>Steam / Linux Proton Launch Options</span></h3>
      <p class="studio-field-desc">When playing via Steam Proton on Linux or Steam Deck, add this to your game's Steam Launch Options:</p>
      <div class="path-display">WINEDLLOVERRIDES="dxgi=n,b" PROTON_ENABLE_NVAPI=1 PROTON_NVIDIA_NVCUDA=1 %command%</div>
      <p class="studio-field-desc">Tip: Press <strong>Insert</strong> in-game to open the OptiScaler overlay menu.</p>
    </div>
  `;
}

// ---------------------------------------------------------------------------
// Initialization & Event Listeners
// ---------------------------------------------------------------------------

async function init() {
  // Navigation
  document.querySelectorAll(".nav-item").forEach((btn) => {
    btn.addEventListener("click", () => switchTab(btn.dataset.tab));
  });

  // Search & filters
  document.getElementById("game-search")?.addEventListener("input", (e) => {
    state.search = e.target.value;
    renderGames();
  });

  document.querySelectorAll("#game-filters .pill").forEach((pill) => {
    pill.addEventListener("click", () => {
      document.querySelectorAll("#game-filters .pill").forEach((p) => p.classList.remove("active"));
      pill.classList.add("active");
      state.filter = pill.dataset.filter;
      renderGames();
    });
  });

  // Scan & Add Custom Game
  document.getElementById("btn-scan-games")?.addEventListener("click", async () => {
    try {
      setActivity("Scanning Steam, Epic Games, GOG & custom libraries...", 5);
      state.games = await invoke("scan_games");
      renderGames();
      if (state.selectedGameId) await selectGame(state.selectedGameId);
      toast(`Found ${state.games.length} games`);
    } catch (e) {
      toast(`Scan failed: ${e}`, true);
    } finally {
      setActivity(null);
    }
  });

  document.getElementById("btn-add-game")?.addEventListener("click", async () => {
    if (!dialog.open) return;
    const dir = await dialog.open({ directory: true, title: "Select Game Folder" });
    if (!dir) return;
    try {
      setActivity("Analyzing game folder...", 40);
      const game = await invoke("add_custom_game", { dir, name: null });
      replaceGameInState(game);
      renderGames();
      await selectGame(game.id);
      toast(`Added ${game.name}`);
    } catch (e) {
      toast(`Could not add game: ${e}`, true);
    } finally {
      setActivity(null);
    }
  });

  // INI controls
  document.getElementById("ini-game-select")?.addEventListener("change", (e) => {
    loadIniForDir(e.target.value);
  });

  document.getElementById("btn-ini-browse")?.addEventListener("click", async () => {
    if (!dialog.open) return;
    const dir = await dialog.open({ directory: true, title: "Select Folder Containing OptiScaler.ini" });
    if (dir) loadIniForDir(dir);
  });

  document.getElementById("btn-ini-save")?.addEventListener("click", saveIni);

  document.getElementById("btn-ini-restore")?.addEventListener("click", async () => {
    if (!state.iniDir) return;
    try {
      state.iniDoc = await invoke("restore_ini_backup", { dir: state.iniDir });
      state.iniPending = {};
      renderIniView();
      toast("Restored OptiScaler.ini from backup");
    } catch (e) {
      toast(e, true);
    }
  });

  document.querySelectorAll("[data-ini-mode]").forEach((btn) => {
    btn.addEventListener("click", () => {
      document.querySelectorAll("[data-ini-mode]").forEach((b) => b.classList.remove("active"));
      btn.classList.add("active");
      state.iniMode = btn.dataset.iniMode;
      renderIniView();
    });
  });

  document.getElementById("ini-search")?.addEventListener("input", (e) => {
    state.iniSearch = e.target.value;
    renderIniView();
  });

  // Packages controls
  document.getElementById("btn-refresh-releases")?.addEventListener("click", async () => {
    try {
      setActivity("Fetching releases from GitHub...", 30);
      state.packages = await invoke("fetch_releases");
      renderPackages();
      toast("Release list updated");
    } catch (e) {
      toast(e, true);
    } finally {
      setActivity(null);
    }
  });

  document.getElementById("btn-import-zip")?.addEventListener("click", async () => {
    if (!dialog.open) return;
    const file = await dialog.open({
      multiple: false,
      filters: [{ name: "DLSS Unlocked Standalone Zip", extensions: ["zip"] }],
      title: "Select dlss-unlocked-standalone-*.zip",
    });
    if (!file) return;
    try {
      setActivity("Importing & extracting standalone .zip...", 35);
      const pkg = await invoke("import_package", { path: file, label: null });
      state.packages = await invoke("get_packages");
      state.installOpts.tag = pkg.tag;
      renderPackages();
      renderInspector();
      toast(`Imported ${pkg.tag} (${pkg.fileCount} files)`);
    } catch (e) {
      toast(`Import failed: ${e}`, true);
    } finally {
      setActivity(null);
    }
  });

  document.getElementById("btn-import-folder")?.addEventListener("click", async () => {
    if (!dialog.open) return;
    const folder = await dialog.open({ directory: true, title: "Select Extracted DLSS Unlocked Folder (containing dxgi.dll)" });
    if (!folder) return;
    try {
      setActivity("Importing extracted package folder...", 50);
      const pkg = await invoke("import_package", { path: folder, label: null });
      state.packages = await invoke("get_packages");
      state.installOpts.tag = pkg.tag;
      renderPackages();
      renderInspector();
      toast(`Imported ${pkg.tag} (${pkg.fileCount} files)`);
    } catch (e) {
      toast(`Import failed: ${e}`, true);
    } finally {
      setActivity(null);
    }
  });

  document.getElementById("btn-open-pkg-dir")?.addEventListener("click", () => {
    if (state.appInfo?.packagesDir) {
      invoke("open_folder", { path: state.appInfo.packagesDir }).catch((e) => toast(e, true));
    }
  });

  document.getElementById("btn-open-data-dir")?.addEventListener("click", () => {
    if (state.appInfo?.dataDir) {
      invoke("open_folder", { path: state.appInfo.dataDir }).catch((e) => toast(e, true));
    }
  });

  document.getElementById("btn-unhide-games")?.addEventListener("click", async () => {
    try {
      await invoke("unhide_all");
      state.games = await invoke("scan_games");
      renderGames();
      toast("Restored hidden games and rescanned library");
    } catch (e) {
      toast(e, true);
    }
  });

  // Backend progress events
  await listen("scan-progress", (ev) => {
    const { current, total, name } = ev.payload;
    const pct = total > 0 ? (current / total) * 100 : 50;
    setActivity(`Scanning (${current}/${total}): ${name}`, pct);
  });

  await listen("op-progress", (ev) => {
    const { phase, done, total, file } = ev.payload;
    if (phase === "copy") {
      const pct = total > 0 ? (done / total) * 100 : 50;
      setActivity(`Deploying (${done}/${total}): ${file}`, pct);
    }
  });

  await listen("download-progress", (ev) => {
    const p = ev.payload;
    state.downloadProgress[p.tag] = p;
    setActivity(
      p.phase === "extract"
        ? `Extracting ${p.tag} (${p.Percent ? p.Percent.toFixed(0) : p.percent.toFixed(0)}%)...`
        : `Downloading ${p.tag} (${p.percent.toFixed(1)}%)...`,
      p.percent
    );
    if (state.tab === "packages") renderPackages();
  });

  // Initial data load
  try {
    state.appInfo = await invoke("get_app_info");
  } catch {}

  invoke("detect_gpus")
    .then((gpus) => {
      state.gpus = gpus || [];
      renderGpuCard();
    })
    .catch(() => renderGpuCard());

  try {
    state.packages = await invoke("get_packages");
    updatePkgBadge();
  } catch {}

  // Load cached games first, or run an initial scan if empty
  try {
    const cached = await invoke("get_cached_games");
    if (cached && cached.length) {
      state.games = cached;
      renderGames();
      await selectGame(cached[0].id);
    } else {
      setActivity("Scanning game libraries...", 15);
      state.games = await invoke("scan_games");
      renderGames();
      if (state.games.length) await selectGame(state.games[0].id);
      setActivity(null);
    }
  } catch {
    setActivity(null);
  }

  // Refresh GitHub releases in background
  invoke("fetch_releases")
    .then((pkgs) => {
      state.packages = pkgs || [];
      updatePkgBadge();
      if (state.tab === "packages") renderPackages();
      renderInspector();
    })
    .catch(() => {});
}

window.addEventListener("DOMContentLoaded", init);
