//! Script registry persisted as editable JSON in the app data directory.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Manager;

pub const SUPPORTED_EXTENSIONS: &[&str] = &["py", "ps1", "bat", "cmd", "js", "sh", "exe"];
pub const SUPPORTED_SHELLS: &[&str] = &["cmd", "pwsh", "bash"];

pub fn is_supported_extension(extension: &str) -> bool {
    SUPPORTED_EXTENSIONS.contains(&extension.to_lowercase().as_str())
}

fn default_mode() -> String {
    "file".into()
}

/// A custom stop action. Exactly one of `path` and `command` must be set.
#[derive(Clone, Serialize, Deserialize)]
pub struct StopAction {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub shell: String,
}

/// Old entries containing only id/name/path remain valid.
#[derive(Clone, Serialize, Deserialize)]
pub struct Script {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub shell: String,
    #[serde(default)]
    pub working_dir: String,
    #[serde(default)]
    pub stop: Option<StopAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<crate::monitor::PortMonitor>,
}

#[derive(Default)]
pub struct ScriptRegistry(pub Mutex<Vec<Script>>);

fn registry_file(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir.join("scripts.json"))
}

fn ensure_registry_file(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let file = registry_file(app)?;
    if !file.exists() {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file)
        {
            Ok(mut handle) => {
                use std::io::Write;
                handle
                    .write_all(b"[]\n")
                    .map_err(|error| error.to_string())?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(file)
}

fn new_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos().to_string())
        .unwrap_or_else(|_| format!("{}", std::process::id()))
}

fn validate_file(path: &str, require_exists: bool) -> Result<(), String> {
    let file = Path::new(path);
    let supported = file
        .extension()
        .and_then(|extension| extension.to_str())
        .map(is_supported_extension)
        .unwrap_or(false);
    if !supported {
        return Err(format!(
            "不支持此文件类型。支持：{}。",
            SUPPORTED_EXTENSIONS.join(", ")
        ));
    }
    if require_exists && !file.is_file() {
        return Err(format!("文件不存在：{path}"));
    }
    Ok(())
}

fn validate_shell(shell: &str) -> Result<(), String> {
    if SUPPORTED_SHELLS.contains(&shell) {
        Ok(())
    } else {
        Err(format!(
            "命令解释器必须为以下之一：{}。",
            SUPPORTED_SHELLS.join(", ")
        ))
    }
}

fn normalize_script(script: &mut Script, require_file: bool) -> Result<(), String> {
    script.id = script.id.trim().to_string();
    script.name = script.name.trim().to_string();
    script.mode = script.mode.trim().to_lowercase();
    script.path = script.path.trim().to_string();
    script.command = script.command.trim().to_string();
    script.shell = script.shell.trim().to_lowercase();
    script.working_dir = script.working_dir.trim().to_string();
    if let Some(monitor) = script.monitor.as_mut() {
        monitor.normalize()?;
    }

    match script.mode.as_str() {
        "file" => {
            if script.path.is_empty() || !script.command.is_empty() {
                return Err("文件条目必须填写文件路径，且不能同时填写命令。".into());
            }
            validate_file(&script.path, require_file)?;
            script.shell.clear();
            if script.name.is_empty() {
                script.name = Path::new(&script.path)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("script")
                    .to_string();
            }
        }
        "command" => {
            if script.command.is_empty() || !script.path.is_empty() {
                return Err("命令条目必须填写命令，且不能同时填写文件路径。".into());
            }
            validate_shell(&script.shell)?;
            if script.name.is_empty() {
                return Err("请为命令条目填写名称。".into());
            }
        }
        _ => return Err("条目模式必须为 file（文件）或 command（命令）。".into()),
    }

    if !script.working_dir.is_empty() && require_file && !Path::new(&script.working_dir).is_dir() {
        return Err(format!("工作目录不存在：{}", script.working_dir));
    }

    if let Some(stop) = script.stop.as_mut() {
        stop.path = stop.path.trim().to_string();
        stop.command = stop.command.trim().to_string();
        stop.shell = stop.shell.trim().to_lowercase();
        match (stop.path.is_empty(), stop.command.is_empty()) {
            (false, true) => {
                validate_file(&stop.path, require_file)?;
                stop.shell.clear();
            }
            (true, false) => {
                validate_shell(&stop.shell)?;
            }
            _ => return Err("停止操作必须填写文件路径或命令中的一项。".into()),
        }
    }
    if script.id.is_empty() {
        script.id = new_id();
    }
    Ok(())
}

pub(crate) fn read_scripts(app: &tauri::AppHandle) -> Result<Vec<Script>, String> {
    let file = ensure_registry_file(app)?;
    let content = fs::read_to_string(&file).map_err(|error| error.to_string())?;
    let mut scripts: Vec<Script> = serde_json::from_str(&content)
        .map_err(|error| format!("配置文件无效 {}：{error}", file.display()))?;
    let needs_ids = scripts.iter().any(|script| script.id.trim().is_empty());
    let mut ids = HashSet::new();
    for (index, script) in scripts.iter_mut().enumerate() {
        normalize_script(script, false)
            .map_err(|error| format!("第 {} 个条目：{error}", index + 1))?;
        if !ids.insert(script.id.clone()) {
            return Err(format!("脚本编号重复：{}", script.id));
        }
    }
    if needs_ids {
        save_scripts(app, &scripts)?;
    }
    Ok(scripts)
}

/// Reads the current on-disk definition so a stop action edited while a
/// process is running can be used without restarting that process.
pub fn find_script(app: &tauri::AppHandle, id: &str) -> Result<Option<Script>, String> {
    Ok(read_scripts(app)?
        .into_iter()
        .find(|script| script.id == id))
}

fn save_scripts(app: &tauri::AppHandle, scripts: &[Script]) -> Result<(), String> {
    let file = registry_file(app)?;
    let content = serde_json::to_string_pretty(scripts).map_err(|error| error.to_string())?;
    fs::write(&file, format!("{content}\n")).map_err(|error| error.to_string())
}

pub fn load_scripts(app: &tauri::AppHandle) -> Vec<Script> {
    read_scripts(app).unwrap_or_default()
}

/// Reload on every read so agents can edit scripts.json while the app is open.
#[tauri::command]
pub fn get_scripts(
    app: tauri::AppHandle,
    state: tauri::State<ScriptRegistry>,
) -> Result<Vec<Script>, String> {
    let mut current = state.0.lock().unwrap();
    let scripts = read_scripts(&app)?;
    *current = scripts.clone();
    Ok(scripts)
}

#[tauri::command]
pub fn get_config_path(app: tauri::AppHandle) -> Result<String, String> {
    Ok(ensure_registry_file(&app)?.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn open_config_file(app: tauri::AppHandle) -> Result<(), String> {
    let file = ensure_registry_file(&app)?;
    #[cfg(windows)]
    let mut command = Command::new("notepad.exe");
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");
    command
        .arg(file)
        .spawn()
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn add_entry(
    mut entry: Script,
    app: tauri::AppHandle,
    state: tauri::State<ScriptRegistry>,
) -> Result<Vec<Script>, String> {
    normalize_script(&mut entry, true)?;
    let mut current = state.0.lock().unwrap();
    let mut scripts = read_scripts(&app)?;
    if scripts.iter().any(|script| {
        script.id == entry.id
            || (entry.mode == "file"
                && script.mode == "file"
                && script.path.eq_ignore_ascii_case(&entry.path))
    }) {
        return Err("此条目已在列表中。".into());
    }
    scripts.push(entry);
    save_scripts(&app, &scripts)?;
    *current = scripts.clone();
    Ok(scripts)
}

/// Replaces one entry in the current on-disk registry, preserving its id and
/// unrelated entries that may have been added by another program.
#[tauri::command]
pub fn update_entry(
    id: String,
    mut entry: Script,
    app: tauri::AppHandle,
    state: tauri::State<ScriptRegistry>,
) -> Result<Vec<Script>, String> {
    entry.id = id.clone();
    normalize_script(&mut entry, true)?;

    let mut current = state.0.lock().unwrap();
    let mut scripts = read_scripts(&app)?;
    let index = scripts
        .iter()
        .position(|script| script.id == id)
        .ok_or("此条目已不存在。")?;

    if entry.mode == "file"
        && scripts.iter().any(|script| {
            script.id != id
                && script.mode == "file"
                && script.path.eq_ignore_ascii_case(&entry.path)
        })
    {
        return Err("其他条目已使用此文件。".into());
    }

    scripts[index] = entry;
    save_scripts(&app, &scripts)?;
    *current = scripts.clone();
    Ok(scripts)
}

#[tauri::command]
pub fn add_script(
    path: String,
    app: tauri::AppHandle,
    state: tauri::State<ScriptRegistry>,
) -> Result<Vec<Script>, String> {
    add_entry(
        Script {
            id: String::new(),
            name: String::new(),
            mode: default_mode(),
            path,
            command: String::new(),
            shell: String::new(),
            working_dir: String::new(),
            stop: None,
            monitor: None,
        },
        app,
        state,
    )
}

#[tauri::command]
pub async fn remove_script(
    id: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, ScriptRegistry>,
    running: tauri::State<'_, crate::runner::RunningScripts>,
) -> Result<Vec<Script>, String> {
    crate::runner::stop_if_running(&id, &app, &running).await?;
    let mut current = state.0.lock().unwrap();
    let mut scripts = read_scripts(&app)?;
    scripts.retain(|script| script.id != id);
    save_scripts(&app, &scripts)?;
    *current = scripts.clone();
    Ok(scripts)
}
