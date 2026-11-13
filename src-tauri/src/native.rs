use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
};
use winreg::{enums::HKEY_CURRENT_USER, RegKey};
use zip::ZipArchive;

const RELEASE_API: &str = "https://api.github.com/repos/apex-compute/ANR/releases/latest";
const REGISTRY_PATH: &str = "Software\\Microsoft\\DirectX\\UserGpuPreferences";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuAdapter {
    pub name: String,
    pub kind: String,
    pub memory_mb: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameFolderInfo {
    pub executables: Vec<String>,
    pub has_optiscaler: bool,
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeStatus {
    pub installed: bool,
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub tag: String,
    pub name: String,
    pub published_at: String,
    pub page_url: String,
    pub cached: bool,
    pub offline: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GithubRelease {
    tag_name: String,
    name: Option<String>,
    published_at: Option<String>,
    html_url: String,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InstallRecord {
    kind: String,
    version: String,
    game_dir: String,
    files: Vec<String>,
    originals: BTreeMap<String, String>,
    #[serde(default)]
    before: BTreeMap<String, Option<String>>,
    #[serde(default)]
    proxy: Option<String>,
}

fn documents_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .document_dir()
        .map(|path| path.join("ANR Native Act"))
        .map_err(|error| format!("Could not locate Documents: {error}"))
}

fn http_client() -> Result<Client, String> {
    Client::builder()
        .user_agent("ANR-Native-Activation/0.1.0")
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|error| format!("Could not prepare the GitHub connection: {error}"))
}

fn fetch_release() -> Result<GithubRelease, String> {
    let response = http_client()?
        .get(RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|error| format!("Could not check GitHub for the latest ANR release: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "GitHub returned HTTP {} while checking releases",
            response.status()
        ));
    }
    response
        .json::<GithubRelease>()
        .map_err(|error| format!("GitHub returned an unreadable release record: {error}"))
}

fn release_cache_file(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(documents_dir(app)?.join("latest-release.json"))
}

fn public_release(app: &AppHandle, release: &GithubRelease, offline: bool) -> ReleaseInfo {
    let cache_root = documents_dir(app).ok();
    let cached = cache_root
        .as_ref()
        .map(|root| {
            release.assets.iter().any(|asset| {
                let file = root
                    .join("releases")
                    .join(&release.tag_name)
                    .join(&asset.name);
                file.is_file()
            })
        })
        .unwrap_or(false);
    ReleaseInfo {
        tag: release.tag_name.clone(),
        name: release
            .name
            .clone()
            .unwrap_or_else(|| release.tag_name.clone()),
        published_at: release.published_at.clone().unwrap_or_default(),
        page_url: release.html_url.clone(),
        cached,
        offline,
    }
}

#[tauri::command]
pub async fn choose_game_folder(app: AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .set_title("Choose the game folder containing its .exe")
        .blocking_pick_folder()
        .and_then(|path| path.into_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn get_gpus() -> Result<Vec<GpuAdapter>, String> {
    // DXGI reports dedicated memory as SIZE_T; WMI's AdapterRAM field can under-report
    // cards above 4 GiB because it is a 32-bit value.
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }
        .map_err(|error| format!("Could not enumerate Windows graphics adapters: {error}"))?;
    let mut adapters = Vec::new();
    let mut index = 0;
    loop {
        let adapter = unsafe { factory.EnumAdapters1(index) };
        let Ok(adapter) = adapter else { break };
        index += 1;
        let Ok(description) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        if description.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        let name = String::from_utf16_lossy(&description.Description)
            .trim_end_matches('\0')
            .trim()
            .to_owned();
        if name.is_empty() {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        let dedicated_bytes = description.DedicatedVideoMemory as u64;
        let integrated = match description.VendorId {
            0x8086 => !lower.contains("arc"),
            0x1002 => {
                dedicated_bytes < 2 * 1024 * 1024 * 1024
                    || lower.contains("780m")
                    || lower.contains("760m")
                    || lower.contains("680m")
                    || lower.contains("vega")
            }
            _ => dedicated_bytes < 512 * 1024 * 1024,
        };
        adapters.push(GpuAdapter {
            kind: if integrated { "integrated" } else { "discrete" }.into(),
            name,
            memory_mb: if integrated || dedicated_bytes == 0 {
                None
            } else {
                Some(dedicated_bytes / (1024 * 1024))
            },
        });
    }
    Ok(adapters)
}

#[tauri::command]
pub fn get_vc_runtime_status() -> RuntimeStatus {
    let system_root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let required = ["MSVCP140.dll", "VCRUNTIME140.dll", "VCRUNTIME140_1.dll"];
    let missing: Vec<String> = required
        .iter()
        .filter(|name| !system_root.join("System32").join(name).is_file())
        .map(|name| (*name).to_owned())
        .collect();
    RuntimeStatus {
        installed: missing.is_empty(),
        missing,
    }
}

#[tauri::command]
pub fn open_vc_runtime_download() -> Result<(), String> {
    Command::new("rundll32.exe")
        .args([
            "url.dll,FileProtocolHandler",
            "https://aka.ms/vs/17/release/vc_redist.x64.exe",
        ])
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not open Microsoft's Visual C++ runtime download: {error}"))
}

#[tauri::command]
pub fn inspect_game_folder(game_dir: String) -> Result<GameFolderInfo, String> {
    let root = validate_game_dir(&game_dir)?;
    let mut executables = Vec::new();
    for entry in
        fs::read_dir(&root).map_err(|error| format!("Could not read game folder: {error}"))?
    {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.is_file()
            && path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        {
            if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
                executables.push(name.to_owned());
            }
        }
    }
    executables.sort_by_key(|value| value.to_ascii_lowercase());
    let full_record = read_record(&root.join("OptiScaler").join("nr_fg_install.json"))?;
    let full_install = full_record.as_ref().is_some_and(|record| {
        record.kind == "full"
            && record
                .game_dir
                .eq_ignore_ascii_case(root.to_string_lossy().as_ref())
    });
    let addon_record = find_addon_record(&root).and_then(|path| read_record(&path).ok().flatten());
    let addon_install = addon_record.as_ref().is_some_and(|record| {
        record.kind == "addon"
            && record
                .game_dir
                .eq_ignore_ascii_case(root.to_string_lossy().as_ref())
    });
    let has_optiscaler = root.join("OptiScaler.ini").is_file() && !full_install;
    let installed = full_install || addon_install;
    Ok(GameFolderInfo {
        executables,
        has_optiscaler,
        installed,
    })
}

#[tauri::command]
pub fn get_latest_release(app: AppHandle) -> Result<ReleaseInfo, String> {
    match fetch_release() {
        Ok(release) => {
            if let Some(parent) = release_cache_file(&app)?.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("Could not prepare the ANR cache: {error}"))?;
            }
            let data = serde_json::to_vec_pretty(&release).map_err(|error| error.to_string())?;
            fs::write(release_cache_file(&app)?, data)
                .map_err(|error| format!("Could not save release details: {error}"))?;
            Ok(public_release(&app, &release, false))
        }
        Err(network_error) => {
            let cached: GithubRelease = serde_json::from_slice(
                &fs::read(release_cache_file(&app).map_err(|_| network_error.clone())?)
                    .map_err(|_| network_error.clone())?,
            )
            .map_err(|_| network_error.clone())?;
            Ok(public_release(&app, &cached, true))
        }
    }
}

#[tauri::command]
pub async fn install_anr(app: AppHandle, game_dir: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || install_anr_blocking(&app, &game_dir))
        .await
        .map_err(|error| format!("Installer task failed: {error}"))?
}

fn install_anr_blocking(app: &AppHandle, game_dir: &str) -> Result<String, String> {
    let root = validate_game_dir(game_dir)?;
    let info = inspect_game_folder(game_dir.to_owned())?;
    if info.executables.is_empty() {
        return Err("Choose the folder that contains the game's .exe file.".into());
    }
    let running = running_game_names(&root);
    if !running.is_empty() {
        return Err(format!(
            "Close the game before installing ANR ({} is running).",
            running[0]
        ));
    }
    let kind = if info.has_optiscaler { "addon" } else { "full" };
    let release = get_release_record(app)?;
    let asset = release
        .assets
        .iter()
        .find(|asset| {
            asset.name.ends_with(format!("-{kind}.zip").as_str())
                && asset.name.starts_with("anr-fg-")
        })
        .ok_or_else(|| {
            format!("The latest GitHub release does not include an ANR FG {kind} package.")
        })?;
    let archive_path = ensure_asset_cached(app, &release, asset)?;
    if kind == "full" {
        let backups = backup_root(app, &root)?;
        install_full_package(&root, &archive_path, &release.tag_name, &backups)?;
        Ok(format!(
            "ANR FG {} installed with OptiScaler.",
            release.tag_name
        ))
    } else {
        let backups = backup_root(app, &root)?;
        install_addon_package(&root, &archive_path, &release.tag_name, &backups)?;
        Ok(format!(
            "ANR FG {} added to the existing OptiScaler install.",
            release.tag_name
        ))
    }
}

fn get_release_record(app: &AppHandle) -> Result<GithubRelease, String> {
    match fetch_release() {
        Ok(release) => {
            let path = release_cache_file(app)?;
            fs::create_dir_all(path.parent().ok_or("Could not resolve ANR cache folder")?)
                .map_err(|error| error.to_string())?;
            fs::write(
                &path,
                serde_json::to_vec_pretty(&release).map_err(|error| error.to_string())?,
            )
            .map_err(|error| format!("Could not save release details: {error}"))?;
            Ok(release)
        }
        Err(error) => {
            serde_json::from_slice(&fs::read(release_cache_file(app)?).map_err(|_| error.clone())?)
                .map_err(|_| error)
        }
    }
}

fn ensure_asset_cached(
    app: &AppHandle,
    release: &GithubRelease,
    asset: &ReleaseAsset,
) -> Result<PathBuf, String> {
    let filename = Path::new(&asset.name);
    if filename.components().count() != 1
        || filename.file_name().and_then(|name| name.to_str()) != Some(asset.name.as_str())
    {
        return Err("GitHub provided an invalid package filename.".into());
    }
    if release
        .tag_name
        .chars()
        .any(|character| matches!(character, '/' | '\\' | ':'))
        || release.tag_name == "."
        || release.tag_name == ".."
    {
        return Err("GitHub provided an invalid release tag.".into());
    }
    let root = documents_dir(app)?.join("releases").join(&release.tag_name);
    fs::create_dir_all(&root)
        .map_err(|error| format!("Could not prepare the ANR download folder: {error}"))?;
    let archive_path = root.join(&asset.name);
    if archive_path.is_file() {
        if verify_digest(&archive_path, asset.digest.as_deref())? {
            return Ok(archive_path);
        }
        fs::remove_file(&archive_path)
            .map_err(|error| format!("Could not replace a damaged cached package: {error}"))?;
    }
    let mut response = http_client()?
        .get(&asset.browser_download_url)
        .send()
        .map_err(|error| format!("Could not download {}: {error}", asset.name))?;
    if !response.status().is_success() {
        return Err(format!(
            "GitHub returned HTTP {} while downloading {}",
            response.status(),
            asset.name
        ));
    }
    let temporary = archive_path.with_extension("zip.part");
    let mut output = File::create(&temporary)
        .map_err(|error| format!("Could not create the package cache: {error}"))?;
    response
        .copy_to(&mut output)
        .map_err(|error| format!("Download interrupted: {error}"))?;
    output.flush().map_err(|error| error.to_string())?;
    drop(output);
    if !verify_digest(&temporary, asset.digest.as_deref())? {
        let _ = fs::remove_file(&temporary);
        return Err("The downloaded package did not match GitHub's SHA-256 digest.".into());
    }
    fs::rename(&temporary, &archive_path)
        .map_err(|error| format!("Could not finish caching the package: {error}"))?;
    Ok(archive_path)
}

fn verify_digest(path: &Path, digest: Option<&str>) -> Result<bool, String> {
    let Some(expected) = digest.and_then(|value| value.strip_prefix("sha256:")) else {
        return Ok(true);
    };
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(expected))
}

fn install_full_package(
    root: &Path,
    archive_path: &Path,
    version: &str,
    backups: &Path,
) -> Result<(), String> {
    let opti_dir = root.join("OptiScaler");
    let record_path = opti_dir.join("nr_fg_install.json");
    ensure_within_game(root, &record_path)?;
    let existing_record = read_record(&record_path)?;
    if let Some(record) = &existing_record {
        validate_install_record(root, record, backups)?;
        if record.kind != "full" {
            return Err("This folder has an add-on install record; it cannot be replaced by the full package.".into());
        }
    }
    if opti_dir.exists() && existing_record.is_none() {
        return Err("An OptiScaler folder already exists here. Choose the add-on option by using a game folder with its OptiScaler.ini, or remove that unrelated folder first.".into());
    }
    let proxy = match existing_record
        .as_ref()
        .and_then(|record| record.proxy.clone())
    {
        Some(proxy) => proxy,
        None => detect_proxy(root)?,
    };
    if existing_record.is_none() && root.join("OptiScaler.ini").is_file() {
        return Err("An OptiScaler.ini already exists. Use the add-on package with the existing OptiScaler install.".into());
    }
    let previous_files = existing_record
        .as_ref()
        .map(|record| record.files.clone())
        .unwrap_or_default();
    let mut archive = open_archive(archive_path)?;
    let mut file_list = Vec::new();
    let mut originals = existing_record
        .map(|record| record.originals)
        .unwrap_or_default();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("Could not read package entry: {error}"))?;
        if entry.is_dir() {
            continue;
        }
        let Some(relative) = entry.name().strip_prefix("game/") else {
            continue;
        };
        let Some(mut target_relative) = safe_relative(relative) else {
            return Err("The package contains an unsafe file path.".into());
        };
        // Upstream ships the proxy as dxgi.dll, but Vulkan-only games need it loaded as winmm.dll.
        if target_relative == Path::new("dxgi.dll") {
            target_relative = PathBuf::from(&proxy);
        }
        // A previous version owns these paths; capture the original once, before replacing it.
        let target = root.join(&target_relative);
        ensure_within_game(root, &target)?;
        let rel_text = target_relative.to_string_lossy().replace('/', "\\");
        if target.exists()
            && !originals.contains_key(&rel_text)
            && !previous_files.contains(&rel_text)
        {
            let backup = backup_file(root, backups, &target_relative)?;
            originals.insert(rel_text.clone(), backup.to_string_lossy().into_owned());
        }
        // Preserve game-specific OptiScaler settings across ANR updates.
        if target.exists()
            && target
                .file_name()
                .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("nr_fg.ini"))
        {
            file_list.push(rel_text);
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut output = File::create(&target)
            .map_err(|error| format!("Could not install {}: {error}", target.display()))?;
        std::io::copy(&mut entry, &mut output)
            .map_err(|error| format!("Could not extract package file: {error}"))?;
        file_list.push(rel_text);
    }
    if file_list.is_empty() {
        return Err("The full package did not contain game files.".into());
    }
    remove_stale_files(root, &previous_files, &file_list, &mut originals)?;
    fs::create_dir_all(&opti_dir).map_err(|error| error.to_string())?;
    let record = InstallRecord {
        kind: "full".into(),
        version: version.into(),
        game_dir: root.to_string_lossy().into_owned(),
        files: file_list,
        originals,
        before: BTreeMap::new(),
        proxy: Some(proxy),
    };
    write_record(&record_path, &record)
}

fn install_addon_package(
    root: &Path,
    archive_path: &Path,
    version: &str,
    backups: &Path,
) -> Result<(), String> {
    let ini_path = root.join("OptiScaler.ini");
    if !ini_path.is_file() {
        return Err(
            "The add-on package requires an existing OptiScaler.ini in the selected game folder."
                .into(),
        );
    }
    let running = running_game_names(root);
    if !running.is_empty() {
        return Err(format!(
            "Close the game before installing ANR ({} is running).",
            running[0]
        ));
    }
    let ini_bytes =
        fs::read(&ini_path).map_err(|error| format!("Could not read OptiScaler.ini: {error}"))?;
    let main_dir = normalize_path(&optiscaler_main_dir(root, &ini_bytes));
    if !windows_path_within(root, &main_dir) {
        return Err("OptiScaler.ini points outside the selected game folder. For safety, ANR can only be installed inside this game folder.".into());
    }
    let dest = main_dir.join("custom").join("Apex Neural Rendering FG");
    let record_path = dest.join("nr_fg_install.json");
    ensure_within_game(root, &record_path)?;
    if dest.exists() && !record_path.exists() {
        return Err("The ANR destination already exists and is not marked as managed by this app. Nothing was changed.".into());
    }
    let existing = read_record(&record_path)?;
    if let Some(record) = &existing {
        validate_install_record(root, record, backups)?;
        if record.kind != "addon" {
            return Err(
                "This folder has a full install record; use its full package to update ANR.".into(),
            );
        }
    }
    let before = existing
        .as_ref()
        .map(|record| record.before.clone())
        .unwrap_or_else(|| {
            let text = String::from_utf8_lossy(&ini_bytes);
            let mut values = BTreeMap::new();
            values.insert("FGInput".into(), ini_value(&text, "FrameGen", "FGInput"));
            values.insert(
                "FGNvngxReplacement".into(),
                ini_value(&text, "FrameGen", "FGNvngxReplacement"),
            );
            values
        });
    let previous_files = existing
        .as_ref()
        .map(|record| record.files.clone())
        .unwrap_or_default();
    let mut originals = existing
        .as_ref()
        .map(|record| record.originals.clone())
        .unwrap_or_default();
    let mut files = existing
        .as_ref()
        .map(|record| record.files.clone())
        .unwrap_or_default();
    let mut archive = open_archive(archive_path)?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("Could not read package entry: {error}"))?;
        if entry.is_dir() {
            continue;
        }
        let Some(relative) = entry.name().strip_prefix("Apex Neural Rendering FG/") else {
            continue;
        };
        let Some(relative_path) = safe_relative(relative) else {
            return Err("The package contains an unsafe file path.".into());
        };
        if relative_path == Path::new("nr_fg_install.json") {
            continue;
        }
        let target = dest.join(&relative_path);
        ensure_within_game(root, &target)?;
        let rel_text = target
            .strip_prefix(root)
            .unwrap_or(&target)
            .to_string_lossy()
            .replace('/', "\\");
        if target.exists()
            && !originals.contains_key(&rel_text)
            && !previous_files.contains(&rel_text)
        {
            let backup = backup_file(root, backups, target.strip_prefix(root).unwrap_or(&target))?;
            originals.insert(rel_text.clone(), backup.to_string_lossy().into_owned());
        }
        // Keep a user's local ANR configuration when refreshing the release.
        if target
            .file_name()
            .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("nr_fg.ini"))
            && target.exists()
        {
            if !files.contains(&rel_text) {
                files.push(rel_text);
            }
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut output = File::create(&target)
            .map_err(|error| format!("Could not install package file: {error}"))?;
        std::io::copy(&mut entry, &mut output)
            .map_err(|error| format!("Could not extract package file: {error}"))?;
        if !files.contains(&rel_text) {
            files.push(rel_text);
        }
    }
    if files.is_empty() {
        return Err("The add-on package did not contain ANR files.".into());
    }
    remove_stale_files(root, &previous_files, &files, &mut originals)?;
    let patched = set_ini_value(
        &String::from_utf8_lossy(&ini_bytes),
        "FrameGen",
        "FGInput",
        "nvngxfg",
    );
    let patched = set_ini_value(
        &patched,
        "FrameGen",
        "FGNvngxReplacement",
        "Apex Neural Rendering FG",
    );
    fs::write(&ini_path, patched)
        .map_err(|error| format!("Could not update OptiScaler.ini: {error}"))?;
    let record = InstallRecord {
        kind: "addon".into(),
        version: version.into(),
        game_dir: root.to_string_lossy().into_owned(),
        files,
        originals,
        before,
        proxy: None,
    };
    write_record(&record_path, &record)
}

#[tauri::command]
pub fn launch_game(game_dir: String, executable: String, gpu: String) -> Result<String, String> {
    let root = validate_game_dir(&game_dir)?;
    if executable.is_empty()
        || Path::new(&executable).components().count() != 1
        || !executable.to_ascii_lowercase().ends_with(".exe")
    {
        return Err("Select a game executable from the list.".into());
    }
    let exe_path = root.join(&executable);
    if !exe_path.is_file() {
        return Err("The selected game executable is no longer in this folder.".into());
    }
    set_windows_gpu_preference(&exe_path, &gpu)?;
    Command::new(&exe_path)
        .current_dir(&root)
        .spawn()
        .map_err(|error| format!("Could not launch the game: {error}"))?;
    Ok(format!(
        "Launched {executable}. ANR will load when the game starts."
    ))
}

fn set_windows_gpu_preference(exe: &Path, gpu: &str) -> Result<(), String> {
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(REGISTRY_PATH, winreg::enums::KEY_SET_VALUE)
        .or_else(|_| {
            RegKey::predef(HKEY_CURRENT_USER)
                .create_subkey(REGISTRY_PATH)
                .map(|pair| pair.0)
        })
        .map_err(|error| {
            format!("Could not open the Windows graphics preference setting: {error}")
        })?;
    let executable = exe.to_string_lossy().to_string();
    match gpu.split(':').next().unwrap_or("auto") {
        "integrated" => key.set_value(&executable, &"GpuPreference=1;".to_owned()),
        "discrete" => key.set_value(&executable, &"GpuPreference=2;".to_owned()),
        _ => key.delete_value(&executable).or_else(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(error)
            }
        }),
    }
    .map_err(|error| format!("Could not apply the Windows GPU preference: {error}"))
}

#[tauri::command]
pub async fn uninstall_anr(app: AppHandle, game_dir: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || uninstall_anr_blocking(&app, &game_dir))
        .await
        .map_err(|error| format!("Uninstaller task failed: {error}"))?
}

fn uninstall_anr_blocking(app: &AppHandle, game_dir: &str) -> Result<String, String> {
    let root = validate_game_dir(game_dir)?;
    let running = running_game_names(&root);
    if !running.is_empty() {
        return Err(format!(
            "Close the game before uninstalling ANR ({} is running).",
            running[0]
        ));
    }
    let full_record_path = root.join("OptiScaler").join("nr_fg_install.json");
    if let Some(record) = read_record(&full_record_path)? {
        let backups = backup_root(app, &root)?;
        if record.kind != "full" {
            return Err("The ANR full-package record is invalid. Nothing was changed.".into());
        }
        restore_record(&root, &record, &full_record_path, &backups)?;
        return Ok("ANR FG and its bundled OptiScaler files were removed; backed-up game files were restored.".into());
    }
    let addon_record =
        find_addon_record(&root).ok_or("ANR FG is not installed in this game folder.")?;
    let record = read_record(&addon_record)?.ok_or("ANR install record is missing.")?;
    if record.kind != "addon" {
        return Err("The ANR add-on record is invalid. Nothing was changed.".into());
    }
    let backups = backup_root(app, &root)?;
    validate_install_record(&root, &record, &backups)?;
    let ini_path = root.join("OptiScaler.ini");
    if ini_path.is_file() {
        let text = fs::read_to_string(&ini_path)
            .map_err(|error| format!("Could not read OptiScaler.ini: {error}"))?;
        let restored = set_optional_ini_value(
            &text,
            "FrameGen",
            "FGInput",
            record.before.get("FGInput").cloned().unwrap_or(None),
        );
        let restored = set_optional_ini_value(
            &restored,
            "FrameGen",
            "FGNvngxReplacement",
            record
                .before
                .get("FGNvngxReplacement")
                .cloned()
                .unwrap_or(None),
        );
        fs::write(&ini_path, restored)
            .map_err(|error| format!("Could not restore OptiScaler.ini: {error}"))?;
    }
    restore_record(&root, &record, &addon_record, &backups)?;
    Ok(
        "ANR FG was removed and the previous OptiScaler frame-generation settings were restored."
            .into(),
    )
}

fn restore_record(
    root: &Path,
    record: &InstallRecord,
    record_path: &Path,
    backups: &Path,
) -> Result<(), String> {
    validate_install_record(root, record, backups)?;
    for relative in &record.files {
        let path = root.join(relative);
        ensure_within_game(root, &path)?;
        if path.is_file() {
            fs::remove_file(&path)
                .map_err(|error| format!("Could not remove {}: {error}", path.display()))?;
        }
    }
    for (relative, backup) in &record.originals {
        let path = root.join(relative);
        ensure_within_game(root, &path)?;
        let backup = PathBuf::from(backup);
        if backup.is_file() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::copy(&backup, &path)
                .map_err(|error| format!("Could not restore {}: {error}", path.display()))?;
        }
    }
    if record_path.is_file() {
        let _ = fs::remove_file(record_path);
    }
    for relative in &record.files {
        if let Some(parent) = root.join(relative).parent() {
            remove_empty_ancestors(parent, root);
        }
    }
    Ok(())
}

fn validate_install_record(
    root: &Path,
    record: &InstallRecord,
    backups: &Path,
) -> Result<(), String> {
    let recorded_root = PathBuf::from(&record.game_dir)
        .canonicalize()
        .map_err(|_| "The ANR install record does not point to this game folder.".to_string())?;
    if !same_windows_path(&recorded_root, root) {
        return Err(
            "The ANR install record belongs to another folder. Nothing was changed.".into(),
        );
    }
    if record.kind != "full" && record.kind != "addon" {
        return Err("The ANR install record has an unknown package type.".into());
    }
    for relative in &record.files {
        if safe_relative(relative).is_none() {
            return Err(
                "The ANR install record contains an unsafe game path. Nothing was changed.".into(),
            );
        }
        ensure_within_game(root, &root.join(relative))?;
    }
    for (relative, backup) in &record.originals {
        if safe_relative(relative).is_none() {
            return Err(
                "The ANR install record contains an unsafe backup mapping. Nothing was changed."
                    .into(),
            );
        }
        ensure_within_game(root, &root.join(relative))?;
        let backup = PathBuf::from(backup);
        if backup.exists() {
            let actual = backup
                .canonicalize()
                .map_err(|error| format!("Could not validate an ANR backup: {error}"))?;
            let allowed = backups
                .canonicalize()
                .unwrap_or_else(|_| normalize_path(backups));
            if !windows_path_within(&allowed, &actual) {
                return Err("The ANR install record references a backup outside the app's backup folder. Nothing was changed.".into());
            }
        }
    }
    Ok(())
}

fn remove_stale_files(
    root: &Path,
    previous: &[String],
    current: &[String],
    originals: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    for relative in previous
        .iter()
        .filter(|relative| !current.contains(relative))
    {
        let path = root.join(relative);
        ensure_within_game(root, &path)?;
        if path.is_file() {
            fs::remove_file(&path)
                .map_err(|error| format!("Could not remove an obsolete ANR file: {error}"))?;
        }
        if let Some(backup) = originals.remove(relative) {
            let backup = PathBuf::from(backup);
            if backup.is_file() {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                fs::copy(backup, &path).map_err(|error| {
                    format!("Could not restore an obsolete game's original file: {error}")
                })?;
            }
        }
        if let Some(parent) = path.parent() {
            remove_empty_ancestors(parent, root);
        }
    }
    Ok(())
}

fn find_addon_record(root: &Path) -> Option<PathBuf> {
    let candidates = [
        root.join("OptiScaler")
            .join("custom")
            .join("Apex Neural Rendering FG")
            .join("nr_fg_install.json"),
        root.join("custom")
            .join("Apex Neural Rendering FG")
            .join("nr_fg_install.json"),
    ];
    candidates.into_iter().find(|path| path.is_file())
}

fn read_record(path: &Path) -> Result<Option<InstallRecord>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let data =
        fs::read(path).map_err(|error| format!("Could not read ANR install record: {error}"))?;
    serde_json::from_slice(&data)
        .map(Some)
        .map_err(|error| format!("ANR install record is invalid: {error}"))
}

fn write_record(path: &Path, record: &InstallRecord) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let temporary = path.with_extension("json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(record).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("Could not save install record: {error}"))?;
    fs::rename(temporary, path).map_err(|error| format!("Could not finish install record: {error}"))
}

fn safe_relative(path: &str) -> Option<PathBuf> {
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    Some(path.to_path_buf())
}

fn open_archive(path: &Path) -> Result<ZipArchive<File>, String> {
    let file =
        File::open(path).map_err(|error| format!("Could not open cached ANR package: {error}"))?;
    ZipArchive::new(file).map_err(|error| format!("The ANR package is not a valid ZIP: {error}"))
}

fn validate_game_dir(path: &str) -> Result<PathBuf, String> {
    let root = PathBuf::from(path)
        .canonicalize()
        .map_err(|error| format!("Could not open the selected game folder: {error}"))?;
    if !root.is_dir() {
        return Err("Select a folder containing the game's .exe file.".into());
    }
    Ok(root)
}

fn detect_proxy(root: &Path) -> Result<String, String> {
    let executable = fs::read_dir(root)
        .map_err(|error| format!("Could not inspect game executables: {error}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        })
        .max_by_key(|path| path.metadata().map(|metadata| metadata.len()).unwrap_or(0))
        .ok_or("Choose the folder containing the game's .exe file.")?;
    let bytes = fs::read(&executable).map_err(|error| {
        format!(
            "Could not inspect {}: {error}",
            executable.file_name().unwrap_or_default().to_string_lossy()
        )
    })?;
    let vulkan = binary_contains_name(&bytes, b"vulkan-1.dll");
    let directx = [
        b"dxgi.dll".as_slice(),
        b"d3d12.dll".as_slice(),
        b"d3d11.dll".as_slice(),
    ]
    .iter()
    .any(|name| binary_contains_name(&bytes, name));
    Ok(if vulkan && !directx {
        "winmm.dll"
    } else {
        "dxgi.dll"
    }
    .into())
}

fn binary_contains_name(bytes: &[u8], needle: &[u8]) -> bool {
    let ascii_match = bytes.windows(needle.len()).any(|window| {
        window
            .iter()
            .zip(needle)
            .all(|(left, right)| left.to_ascii_lowercase() == *right)
    });
    if ascii_match {
        return true;
    }
    let utf16: Vec<u8> = needle
        .iter()
        .flat_map(|byte| [byte.to_ascii_lowercase(), 0])
        .collect();
    bytes.windows(utf16.len()).any(|window| window == utf16)
}

fn backup_root(app: &AppHandle, game: &Path) -> Result<PathBuf, String> {
    let mut hasher = Sha256::new();
    hasher.update(game.to_string_lossy().to_lowercase().as_bytes());
    let id = format!("{:x}", hasher.finalize());
    Ok(documents_dir(app)?.join("backups").join(id))
}

fn backup_file(root: &Path, backups: &Path, relative: &Path) -> Result<PathBuf, String> {
    let backup = backups.join(relative);
    if let Some(parent) = backup.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::copy(root.join(relative), &backup)
        .map_err(|error| format!("Could not back up {}: {error}", relative.display()))?;
    Ok(backup)
}

fn running_game_names(root: &Path) -> Vec<String> {
    // The game path is passed as a quoted literal from a temporary encoded PowerShell command.
    let escaped = root.to_string_lossy().replace('\'', "''");
    let script = format!("$root='{}'; Get-CimInstance Win32_Process | Where-Object {{ $_.ExecutablePath -and $_.ExecutablePath.StartsWith($root,[StringComparison]::OrdinalIgnoreCase) }} | Select-Object -ExpandProperty Name | ConvertTo-Json -Compress", escaped);
    let encoded = base64_utf16(&script);
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if text.is_empty() {
        return Vec::new();
    }
    serde_json::from_str::<Vec<String>>(&text).unwrap_or_else(|_| {
        serde_json::from_str::<String>(&text)
            .map(|value| vec![value])
            .unwrap_or_default()
    })
}

fn base64_utf16(text: &str) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut output = String::new();
    for chunk in bytes.chunks(3) {
        let a = chunk[0] as u32;
        let b = (*chunk.get(1).unwrap_or(&0)) as u32;
        let c = (*chunk.get(2).unwrap_or(&0)) as u32;
        let block = (a << 16) | (b << 8) | c;
        output.push(TABLE[((block >> 18) & 63) as usize] as char);
        output.push(TABLE[((block >> 12) & 63) as usize] as char);
        output.push(if chunk.len() > 1 {
            TABLE[((block >> 6) & 63) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[(block & 63) as usize] as char
        } else {
            '='
        });
    }
    output
}

fn optiscaler_main_dir(root: &Path, ini: &[u8]) -> PathBuf {
    let text = String::from_utf8_lossy(ini);
    if let Some(path) = ini_any_value(&text, "OptiDllPath") {
        let trimmed = path.trim_matches('"');
        if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("auto") {
            let specified = PathBuf::from(trimmed);
            return if specified.is_absolute() {
                specified
            } else {
                root.join(specified)
            };
        }
    }
    if root.join("OptiScaler").is_dir() {
        root.join("OptiScaler")
    } else {
        root.to_path_buf()
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}

fn ensure_within_game(root: &Path, target: &Path) -> Result<(), String> {
    let normalized = normalize_path(target);
    if !windows_path_within(root, &normalized) {
        return Err("The resolved ANR file path falls outside the selected game folder. Nothing was changed.".into());
    }
    let mut existing = normalized.as_path();
    while !existing.exists() {
        existing = existing
            .parent()
            .ok_or("Could not resolve the game file path.")?;
    }
    let actual = existing
        .canonicalize()
        .map_err(|error| format!("Could not validate the game file path: {error}"))?;
    if !windows_path_within(root, &actual) {
        return Err(
            "A game folder link points outside the selected game folder. Nothing was changed."
                .into(),
        );
    }
    Ok(())
}

fn same_windows_path(left: &Path, right: &Path) -> bool {
    windows_path_string(left) == windows_path_string(right)
}

fn windows_path_within(root: &Path, target: &Path) -> bool {
    let base = windows_path_string(root);
    let child = windows_path_string(target);
    child == base
        || child
            .strip_prefix(&base)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

fn windows_path_string(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn ini_any_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (name, value) = line.trim().split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case(key)
            .then(|| value.trim().to_owned())
    })
}

fn ini_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_section = line[1..line.len() - 1].eq_ignore_ascii_case(section);
            continue;
        }
        if in_section {
            if let Some((name, value)) = line.split_once('=') {
                if name.trim().eq_ignore_ascii_case(key) {
                    return Some(value.trim().to_owned());
                }
            }
        }
    }
    None
}

fn set_ini_value(text: &str, section: &str, key: &str, value: &str) -> String {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let mut in_section = false;
    let mut section_index = None;
    let mut key_index = None;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_section = trimmed[1..trimmed.len() - 1].eq_ignore_ascii_case(section);
            if in_section {
                section_index = Some(index);
            }
        } else if in_section
            && trimmed
                .split_once('=')
                .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case(key))
        {
            key_index = Some(index);
            break;
        }
    }
    if let Some(index) = key_index {
        lines[index] = format!("{key}={value}");
    } else if let Some(index) = section_index {
        let end = lines
            .iter()
            .enumerate()
            .skip(index + 1)
            .find(|(_, line)| line.trim().starts_with('['))
            .map(|(index, _)| index)
            .unwrap_or(lines.len());
        lines.insert(end, format!("{key}={value}"));
    } else {
        if !lines.last().is_some_and(|line| line.is_empty()) {
            lines.push(String::new());
        }
        lines.push(format!("[{section}]"));
        lines.push(format!("{key}={value}"));
    }
    lines.join(newline)
}

fn set_optional_ini_value(text: &str, section: &str, key: &str, value: Option<String>) -> String {
    match value {
        Some(value) => set_ini_value(text, section, key, &value),
        None => {
            let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
            let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
            let mut in_section = false;
            lines.retain(|line| {
                let trimmed = line.trim();
                if trimmed.starts_with('[') && trimmed.ends_with(']') {
                    in_section = trimmed[1..trimmed.len() - 1].eq_ignore_ascii_case(section);
                    return true;
                }
                !(in_section
                    && trimmed
                        .split_once('=')
                        .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case(key)))
            });
            lines.join(newline)
        }
    }
}

fn remove_empty_ancestors(mut path: &Path, root: &Path) {
    while path.starts_with(root) && path != root {
        if fs::remove_dir(path).is_err() {
            break;
        }
        let Some(parent) = path.parent() else { break };
        path = parent;
    }
}
