//! Script runner: spawns registered scripts and streams their output to the UI.

use serde::Serialize;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::Emitter;

use crate::scripts::{is_supported_extension, Script, ScriptRegistry, StopAction};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Windows flag preventing spawned processes from opening a console window.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// How often a watcher thread polls its process for exit.
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(200);
const STOP_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const STOP_GRACE_PERIOD: Duration = Duration::from_secs(5);

/// Process-wide suspend/resume through `ntdll`.
///
/// These functions are undocumented but have been stable for decades and are
/// what tools like Process Explorer use for their own Suspend action. The
/// pause is a hard freeze: the script is unaware of it, so on resume it may
/// find network connections dropped or sleep-based schedules shifted.
#[cfg(windows)]
mod suspend {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;

    #[link(name = "ntdll")]
    extern "system" {
        fn NtSuspendProcess(process_handle: isize) -> i32;
        fn NtResumeProcess(process_handle: isize) -> i32;
    }

    /// Freezes every thread of the child process.
    pub fn suspend(child: &Child) -> Result<(), String> {
        let status = unsafe { NtSuspendProcess(child.as_raw_handle() as isize) };

        if status == 0 {
            Ok(())
        } else {
            Err(format!("暂停进程失败（状态码 {status}）。"))
        }
    }

    /// Resumes every thread of the child process.
    pub fn resume(child: &Child) -> Result<(), String> {
        let status = unsafe { NtResumeProcess(child.as_raw_handle() as isize) };

        if status == 0 {
            Ok(())
        } else {
            Err(format!("继续进程失败（状态码 {status}）。"))
        }
    }
}

#[cfg(not(windows))]
mod suspend {
    use std::process::Child;

    /// Pause is only implemented for Windows (the project's current scope).
    pub fn suspend(_child: &Child) -> Result<(), String> {
        Err("暂停功能仅支持 Windows。".into())
    }

    /// Resume is only implemented for Windows (the project's current scope).
    pub fn resume(_child: &Child) -> Result<(), String> {
        Err("继续功能仅支持 Windows。".into())
    }
}

/// A managed process, its input pipe, and its pause state.
pub struct RunningScript {
    child: Child,
    stdin: Option<ChildStdin>,
    paused: bool,
    stopping: bool,
    stop: Option<StopAction>,
    working_dir: String,
}

/// Processes currently running, keyed by script id.
#[derive(Default)]
pub struct RunningScripts(pub Arc<Mutex<HashMap<String, RunningScript>>>);

/// Running-state snapshot sent to the UI on initialization.
#[derive(Clone, Serialize)]
pub struct RunningInfo {
    pub id: String,
    pub paused: bool,
}

/// A chunk of output produced by a running script. Chunks are not
/// line-aligned: prompts printed without a trailing newline arrive too.
#[derive(Clone, Serialize)]
struct OutputChunk {
    id: String,
    stream: &'static str,
    chunk: String,
}

/// End-of-run notification carrying the script's exit code.
#[derive(Clone, Serialize)]
struct ExitedEvent {
    id: String,
    code: Option<i32>,
    monitored: bool,
}

/// Builds the command to launch a script, chosen by its file extension.
///
/// `.py` is run unbuffered (`-u`) so output streams live. `.ps1` is run
/// without `-ExecutionPolicy Bypass`: if the local policy blocks the
/// script, that surfaces as a normal PowerShell error in the output pane
/// rather than being silently overridden. `.exe` is launched directly with
/// no interpreter, so it can run arbitrary compiled binaries, not just
/// readable/inspectable scripts.
fn launch_command(path: &Path) -> Result<Command, String> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
        .unwrap_or_default();

    if !is_supported_extension(&extension) {
        return Err(format!("不支持的脚本类型：.{extension}"));
    }

    let mut command = match extension.as_str() {
        "py" => {
            let mut command = Command::new("python");
            command.arg("-u");
            command
        }
        "ps1" => {
            let mut command = Command::new("powershell");
            command.args(["-NoLogo", "-NoProfile", "-File"]);
            command
        }
        "bat" | "cmd" => {
            let mut command = Command::new("cmd");
            command.arg("/c");
            command
        }
        "js" => Command::new("node"),
        // Requires a bash on PATH (Git Bash or WSL both provide one).
        "sh" => Command::new("bash"),
        "exe" => return Ok(Command::new(path)),
        _ => unreachable!("checked by is_supported_extension above"),
    };

    command.arg(path);

    Ok(command)
}

fn shell_command(shell: &str, text: &str) -> Result<Command, String> {
    let mut command = match shell {
        "cmd" => {
            let mut command = Command::new("cmd");
            command.arg("/C");
            command
        }
        "pwsh" => {
            let mut command = Command::new("pwsh");
            command.args(["-NoLogo", "-NoProfile", "-Command"]);
            command
        }
        "bash" => {
            let mut command = Command::new("bash");
            command.arg("-lc");
            command
        }
        _ => return Err(format!("未知的命令解释器：{shell}")),
    };
    command.arg(text);
    Ok(command)
}

fn apply_working_dir(command: &mut Command, working_dir: &str, file: Option<&Path>) {
    if !working_dir.is_empty() {
        command.current_dir(working_dir);
    } else if let Some(parent) = file.and_then(Path::parent) {
        if !parent.as_os_str().is_empty() {
            command.current_dir(parent);
        }
    }
}

fn start_command(script: &Script) -> Result<Command, String> {
    if script.mode == "command" {
        let mut command = shell_command(&script.shell, &script.command)?;
        apply_working_dir(&mut command, &script.working_dir, None);
        Ok(command)
    } else {
        let path = Path::new(&script.path);
        let mut command = launch_command(path)?;
        apply_working_dir(&mut command, &script.working_dir, Some(path));
        Ok(command)
    }
}

fn stop_command(stop: &StopAction, working_dir: &str) -> Result<Command, String> {
    if !stop.command.is_empty() {
        let mut command = shell_command(&stop.shell, &stop.command)?;
        apply_working_dir(&mut command, working_dir, None);
        Ok(command)
    } else {
        let path = Path::new(&stop.path);
        let mut command = launch_command(path)?;
        apply_working_dir(&mut command, working_dir, Some(path));
        Ok(command)
    }
}

/// Forwards a script's output to the UI as `script-output` events, chunk by
/// chunk (not line by line), so input prompts printed without a trailing
/// newline still show up immediately.
fn stream_output<R: std::io::Read + Send + 'static>(
    app: tauri::AppHandle,
    id: String,
    stream: &'static str,
    mut reader: R,
) {
    thread::spawn(move || {
        let mut buffer = [0_u8; 4096];

        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    let chunk = String::from_utf8_lossy(&buffer[..count]).to_string();

                    let _ = app.emit(
                        "script-output",
                        OutputChunk {
                            id: id.clone(),
                            stream,
                            chunk,
                        },
                    );
                }
            }
        }
    });
}

/// Polls a running process until it exits, then removes it from the
/// running map and notifies the UI with a `script-exited` event.
fn watch_exit(
    app: tauri::AppHandle,
    id: String,
    running: Arc<Mutex<HashMap<String, RunningScript>>>,
) {
    thread::spawn(move || loop {
        thread::sleep(EXIT_POLL_INTERVAL);

        let mut map = running.lock().unwrap();

        let Some(entry) = map.get_mut(&id) else {
            return;
        };

        match entry.child.try_wait() {
            Ok(None) => {
                // Still running, keep polling.
            }
            Ok(Some(status)) => {
                map.remove(&id);
                crate::monitor::launcher_finished(&app, &id, status.success());
                let monitored = crate::scripts::find_script(&app, &id)
                    .ok()
                    .flatten()
                    .is_some_and(|script| script.monitor.is_some());
                let _ = app.emit(
                    "script-exited",
                    ExitedEvent {
                        id,
                        code: status.code(),
                        monitored,
                    },
                );
                return;
            }
            Err(_) => {
                map.remove(&id);
                let monitored = crate::scripts::find_script(&app, &id)
                    .ok()
                    .flatten()
                    .is_some_and(|script| script.monitor.is_some());
                let _ = app.emit(
                    "script-exited",
                    ExitedEvent {
                        id,
                        code: None,
                        monitored,
                    },
                );
                return;
            }
        }
    });
}

/// Starts a registered file or shell command.
#[tauri::command]
pub async fn start_script(
    id: String,
    app: tauri::AppHandle,
    registry: tauri::State<'_, ScriptRegistry>,
    running: tauri::State<'_, RunningScripts>,
) -> Result<(), String> {
    start_registered_script(id, app, registry, running, false).await
}

/// Automatic starts skip running processes/services; manual starts retain
/// the existing duplicate-start feedback.
pub(crate) async fn start_registered_script(
    id: String,
    app: tauri::AppHandle,
    registry: tauri::State<'_, ScriptRegistry>,
    running: tauri::State<'_, RunningScripts>,
    skip_running: bool,
) -> Result<(), String> {
    let script = crate::scripts::get_scripts(app.clone(), registry)?
        .into_iter()
        .find(|script| script.id == id)
        .ok_or("未知脚本。")?;

    if running.0.lock().unwrap().contains_key(&id) {
        if skip_running {
            return Ok(());
        }
        return Err("此脚本已在运行。".into());
    }
    if let Some(config) = &script.monitor {
        crate::monitor::begin_start(&app, &id, config)?;
        let config = config.clone();
        let result = tauri::async_runtime::spawn_blocking(move || crate::monitor::probe(&config))
            .await
            .map_err(|error| error.to_string());
        match result {
            Ok(result) => {
                let already_running = result == crate::monitor::Probe::Open;
                let accepted = crate::monitor::accept_start_probe(&app, &id, result);
                if skip_running && already_running {
                    return Ok(());
                }
                accepted?;
            }
            Err(message) => {
                crate::monitor::operation_failed(&app, &id, &message);
                return Err(message);
            }
        }
    }

    let mut map = running.0.lock().unwrap();

    if map.contains_key(&id) {
        if skip_running {
            return Ok(());
        }
        return Err("此脚本已在运行。".into());
    }

    let mut command = start_command(&script).map_err(|message| {
        crate::monitor::operation_failed(&app, &id, &message);
        message
    })?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut child = command.spawn().map_err(|error| {
        let message = format!("启动脚本失败：{error}");
        crate::monitor::operation_failed(&app, &id, &message);
        message
    })?;

    if let Some(stdout) = child.stdout.take() {
        stream_output(app.clone(), id.clone(), "stdout", stdout);
    }

    if let Some(stderr) = child.stderr.take() {
        stream_output(app.clone(), id.clone(), "stderr", stderr);
    }

    let stdin = child.stdin.take();

    map.insert(
        id.clone(),
        RunningScript {
            child,
            stdin,
            paused: false,
            stopping: false,
            stop: script.stop,
            working_dir: script.working_dir,
        },
    );
    drop(map);

    watch_exit(app, id, Arc::clone(&running.0));

    Ok(())
}

fn emit_info(app: &tauri::AppHandle, id: &str, message: &str) {
    let _ = app.emit(
        "script-output",
        OutputChunk {
            id: id.to_string(),
            stream: "info",
            chunk: format!("[{message}]\n"),
        },
    );
}

fn wait_until_exited(
    id: &str,
    running: &Arc<Mutex<HashMap<String, RunningScript>>>,
    duration: Duration,
) -> bool {
    let deadline = Instant::now() + duration;
    loop {
        {
            let mut map = running.lock().unwrap();
            let Some(entry) = map.get_mut(id) else {
                return true;
            };
            if matches!(entry.child.try_wait(), Ok(Some(_))) {
                return true;
            }
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(EXIT_POLL_INTERVAL);
    }
}

fn run_stop_action(stop: &StopAction, working_dir: &str, managed_pid: u32) -> Result<(), String> {
    let mut command = stop_command(stop, working_dir)?;
    command
        .env("SCRIPT_MANAGER_PID", managed_pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动停止操作失败：{error}"))?;
    let deadline = Instant::now() + STOP_COMMAND_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err(format!("停止操作已退出，状态为 {status}"))
                };
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(EXIT_POLL_INTERVAL),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("停止操作超过 10 秒仍未完成".into());
            }
            Err(error) => return Err(format!("停止操作失败：{error}")),
        }
    }
}

fn kill_process_tree(entry: &mut RunningScript) -> Result<(), String> {
    if matches!(entry.child.try_wait(), Ok(Some(_))) {
        return Ok(());
    }
    #[cfg(windows)]
    {
        let status = Command::new("taskkill")
            .args(["/PID", &entry.child.id().to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if matches!(status, Ok(code) if code.success()) {
            return Ok(());
        }
    }
    entry
        .child
        .kill()
        .map_err(|error| format!("停止进程失败：{error}"))
}

fn stop_managed(
    id: &str,
    app: &tauri::AppHandle,
    running: &Arc<Mutex<HashMap<String, RunningScript>>>,
    stop: Option<StopAction>,
    working_dir: &str,
    managed_pid: u32,
) -> Result<(), String> {
    if let Some(action) = stop {
        emit_info(app, id, "正在执行已配置的停止操作");
        if let Err(error) = run_stop_action(&action, working_dir, managed_pid) {
            emit_info(app, id, &error);
        }
        if wait_until_exited(id, running, STOP_GRACE_PERIOD) {
            return Ok(());
        }
        emit_info(app, id, "等待停止超时，正在结束进程树");
    }

    let mut map = running.lock().unwrap();
    if let Some(entry) = map.get_mut(id) {
        kill_process_tree(entry)?;
    }
    Ok(())
}

/// Stops a managed process, running its optional stop action first.
async fn stop_managed_if_running(
    id: &str,
    app: &tauri::AppHandle,
    running: &RunningScripts,
) -> Result<(), String> {
    let processes = Arc::clone(&running.0);
    let latest = crate::scripts::find_script(app, id).ok().flatten();
    let (stop, working_dir, managed_pid) = {
        let mut map = processes.lock().unwrap();
        let Some(entry) = map.get_mut(id) else {
            return Ok(());
        };
        if entry.stopping {
            return Ok(());
        }
        entry.stopping = true;
        (
            latest
                .as_ref()
                .map(|script| script.stop.clone())
                .unwrap_or_else(|| entry.stop.clone()),
            latest
                .as_ref()
                .map(|script| script.working_dir.clone())
                .unwrap_or_else(|| entry.working_dir.clone()),
            entry.child.id(),
        )
    };
    let app = app.clone();
    let id = id.to_string();
    let run_id = id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        stop_managed(&run_id, &app, &processes, stop, &working_dir, managed_pid)
    })
    .await
    .map_err(|error| error.to_string())?;
    if result.is_err() {
        if let Some(entry) = running.0.lock().unwrap().get_mut(&id) {
            entry.stopping = false;
        }
    }
    result
}

async fn stop_port_service(
    script: crate::scripts::Script,
    app: &tauri::AppHandle,
    running: &RunningScripts,
) -> Result<(), String> {
    let config = script.monitor.clone().ok_or("此条目未配置端口检测。")?;
    let stop = script
        .stop
        .clone()
        .ok_or("端口检测条目需要配置停止操作，才能停止后台服务。")?;
    crate::monitor::begin_stop(app, &script.id, &config)?;
    let result = if running.0.lock().unwrap().contains_key(&script.id) {
        stop_managed_if_running(&script.id, app, running).await
    } else {
        let working_dir = script.working_dir.clone();
        tauri::async_runtime::spawn_blocking(move || run_stop_action(&stop, &working_dir, 0))
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result)
    };
    if let Err(message) = result {
        crate::monitor::operation_failed(app, &script.id, &message);
        return Err(message);
    }
    let app = app.clone();
    let id = script.id;
    tauri::async_runtime::spawn_blocking(move || crate::monitor::wait_stopped(&app, &id, &config))
        .await
        .map_err(|error| error.to_string())?
}

/// Removal waits for a monitored service to stop before forgetting its entry.
pub async fn stop_if_running(
    id: &str,
    app: &tauri::AppHandle,
    running: &RunningScripts,
) -> Result<(), String> {
    if let Some(script) = crate::scripts::find_script(app, id)? {
        if let Some(config) = script.monitor.clone() {
            let result =
                tauri::async_runtime::spawn_blocking(move || crate::monitor::probe(&config))
                    .await
                    .map_err(|error| error.to_string())?;
            if result != crate::monitor::Probe::Closed || running.0.lock().unwrap().contains_key(id)
            {
                return stop_port_service(script, app, running).await;
            }
            return Ok(());
        }
    }
    stop_managed_if_running(id, app, running).await
}

/// Stops a managed script. If the script was started outside this app, its
/// configured stop action can still be invoked directly.
#[tauri::command]
pub async fn stop_script(
    id: String,
    app: tauri::AppHandle,
    running: tauri::State<'_, RunningScripts>,
) -> Result<(), String> {
    let script = crate::scripts::find_script(&app, &id)?.ok_or("未找到脚本。")?;
    if script.monitor.is_some() {
        return stop_port_service(script, &app, &running).await;
    }
    if running.0.lock().unwrap().contains_key(&id) {
        return stop_managed_if_running(&id, &app, &running).await;
    }

    let script =
        crate::scripts::find_script(&app, &id)?.ok_or_else(|| "未找到脚本。".to_string())?;
    let stop = script
        .stop
        .ok_or_else(|| "此脚本未配置停止操作。".to_string())?;
    let working_dir = script.working_dir;
    let app_for_task = app.clone();
    let id_for_task = id.clone();

    tauri::async_runtime::spawn_blocking(move || {
        emit_info(&app_for_task, &id_for_task, "正在执行已配置的停止操作");
        run_stop_action(&stop, &working_dir, 0)?;
        emit_info(&app_for_task, &id_for_task, "停止操作已完成");
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn stop_all_scripts(
    app: tauri::AppHandle,
    running: tauri::State<'_, RunningScripts>,
) -> Result<(), String> {
    let mut ids: std::collections::HashSet<String> =
        running.0.lock().unwrap().keys().cloned().collect();
    for info in crate::monitor::get_script_states(app.clone()) {
        if info.monitored && info.status == "running" && info.can_stop {
            ids.insert(info.id);
        }
    }
    let mut errors = Vec::new();
    for id in ids {
        if let Err(error) = stop_if_running(&id, &app, &running).await {
            errors.push(format!("{id}: {error}"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Sends a line of input to a running script's stdin and echoes it to the
/// terminal as a `script-output` event with the `stdin` stream tag.
#[tauri::command]
pub fn send_input(
    id: String,
    text: String,
    app: tauri::AppHandle,
    running: tauri::State<RunningScripts>,
) -> Result<(), String> {
    ensure_process_controls(&app, &id)?;
    let mut map = running.0.lock().unwrap();

    let entry = map.get_mut(&id).ok_or("此脚本未运行。")?;

    let stdin = entry.stdin.as_mut().ok_or("此脚本当前无法接收输入。")?;

    writeln!(stdin, "{text}").map_err(|error| error.to_string())?;
    stdin.flush().map_err(|error| error.to_string())?;

    let _ = app.emit(
        "script-output",
        OutputChunk {
            id,
            stream: "stdin",
            chunk: format!("{text}\n"),
        },
    );

    Ok(())
}

/// Freezes a running script's process until it is resumed or stopped.
#[tauri::command]
pub fn pause_script(
    id: String,
    app: tauri::AppHandle,
    running: tauri::State<RunningScripts>,
) -> Result<(), String> {
    ensure_process_controls(&app, &id)?;
    let mut map = running.0.lock().unwrap();

    let entry = map.get_mut(&id).ok_or("此脚本未运行。")?;

    if entry.paused {
        return Err("此脚本已暂停。".into());
    }

    suspend::suspend(&entry.child)?;
    entry.paused = true;

    Ok(())
}

/// Resumes a paused script's process.
#[tauri::command]
pub fn resume_script(
    id: String,
    app: tauri::AppHandle,
    running: tauri::State<RunningScripts>,
) -> Result<(), String> {
    ensure_process_controls(&app, &id)?;
    let mut map = running.0.lock().unwrap();

    let entry = map.get_mut(&id).ok_or("此脚本未运行。")?;

    if !entry.paused {
        return Err("此脚本未暂停。".into());
    }

    suspend::resume(&entry.child)?;
    entry.paused = false;

    Ok(())
}

/// Prevent controls from targeting a launcher instead of its detached service.
fn ensure_process_controls(app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    if crate::scripts::find_script(app, id)?.is_some_and(|script| script.monitor.is_some()) {
        return Err("端口检测条目不支持暂停、继续或标准输入。".into());
    }
    Ok(())
}

/// Returns directly managed processes; service status is reported separately.
#[tauri::command]
pub fn get_running_scripts(running: tauri::State<RunningScripts>) -> Vec<RunningInfo> {
    running
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|(id, entry)| RunningInfo {
            id: id.clone(),
            paused: entry.paused,
        })
        .collect()
}
