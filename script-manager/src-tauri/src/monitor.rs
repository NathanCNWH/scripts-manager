//! TCP monitoring is independent from the short-lived launcher process.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

use crate::scripts::ScriptRegistry;

// Windows may take about two seconds to report a refused loopback connection.
// A shorter timeout would misclassify a stopped local service as a network error.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const CHECK_INTERVAL: Duration = Duration::from_secs(2);
const START_TIMEOUT: Duration = Duration::from_secs(120);
const STOP_TIMEOUT: Duration = Duration::from_secs(30);

fn default_host() -> String {
    "127.0.0.1".into()
}

/// An optional TCP target; old entries omit this field entirely.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortMonitor {
    pub kind: String,
    #[serde(default = "default_host")]
    pub host: String,
    pub port: u16,
}

impl PortMonitor {
    pub fn normalize(&mut self) -> Result<(), String> {
        self.host = self.host.trim().to_string();
        if self.host.is_empty() || self.host.eq_ignore_ascii_case("localhost") {
            self.host = default_host();
        }
        if self.kind != "tcp" {
            return Err("运行状态检测仅支持 tcp。".into());
        }
        if self.port == 0 {
            return Err("检测端口必须在 1–65535 之间。".into());
        }
        self.address()?;
        Ok(())
    }

    fn address(&self) -> Result<SocketAddr, String> {
        let ip: IpAddr = self
            .host
            .parse()
            .map_err(|_| "检测地址请填写 IPv4、IPv6 或 localhost。".to_string())?;
        Ok(SocketAddr::new(ip, self.port))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Probe {
    Open,
    Closed,
    Error(String),
}

pub(crate) fn probe(config: &PortMonitor) -> Probe {
    let address = match config.address() {
        Ok(address) => address,
        Err(message) => return Probe::Error(message),
    };
    match TcpStream::connect_timeout(&address, CONNECT_TIMEOUT) {
        Ok(_) => Probe::Open,
        Err(error) if error.kind() == ErrorKind::ConnectionRefused => Probe::Closed,
        Err(error) => Probe::Error(format!("无法检测 {}：{error}", address)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Starting,
    Stopping,
}

struct Entry {
    config: PortMonitor,
    status: String,
    message: String,
    phase: Option<(Phase, Instant)>,
}

impl Entry {
    fn new(config: PortMonitor) -> Self {
        Self {
            config,
            status: "checking".into(),
            message: "正在检测端口".into(),
            phase: None,
        }
    }

    fn observe(&mut self, result: Probe, now: Instant) {
        if let Some((phase, deadline)) = self.phase {
            let completed = matches!(
                (phase, &result),
                (Phase::Starting, Probe::Open) | (Phase::Stopping, Probe::Closed)
            );
            if !completed && now < deadline {
                self.status = match phase {
                    Phase::Starting => "starting",
                    Phase::Stopping => "stopping",
                }
                .into();
                self.message = match phase {
                    Phase::Starting => "等待服务端口开始监听",
                    Phase::Stopping => "等待服务端口关闭",
                }
                .into();
                return;
            }
            self.phase = None;
            if !completed {
                self.status = "error".into();
                self.message = match phase {
                    Phase::Starting => "启动等待超时，端口尚未就绪",
                    Phase::Stopping => "停止等待超时，端口尚未关闭",
                }
                .into();
                return;
            }
        }
        match result {
            Probe::Open => {
                self.status = "running".into();
                self.message = "TCP 端口可连接；此检测不代表应用健康".into();
            }
            Probe::Closed => {
                self.status = "stopped".into();
                self.message = "TCP 端口未监听".into();
            }
            Probe::Error(message) => {
                self.status = "error".into();
                self.message = message;
            }
        }
    }
}

#[derive(Default)]
pub struct PortStates(Mutex<HashMap<String, Entry>>);

/// Authoritative state shared by the list and tray, including detached services.
#[derive(Clone, Serialize)]
pub struct RuntimeInfo {
    pub id: String,
    pub status: String,
    pub monitored: bool,
    pub managed: bool,
    pub can_stop: bool,
    pub message: String,
}

fn entry<'a>(
    entries: &'a mut HashMap<String, Entry>,
    id: &str,
    config: &PortMonitor,
) -> &'a mut Entry {
    let current = entries
        .entry(id.to_string())
        .or_insert_with(|| Entry::new(config.clone()));
    if &current.config != config {
        *current = Entry::new(config.clone());
    }
    current
}

pub(crate) fn begin_start(
    app: &tauri::AppHandle,
    id: &str,
    config: &PortMonitor,
) -> Result<(), String> {
    let state = app.state::<PortStates>();
    let mut entries = state.0.lock().unwrap();
    let current = entry(&mut entries, id, config);
    if current.phase.is_some() {
        return Err("此条目正在启动或停止，请稍候。".into());
    }
    current.phase = Some((Phase::Starting, Instant::now() + START_TIMEOUT));
    current.status = "starting".into();
    current.message = "正在检查端口并启动服务".into();
    Ok(())
}

pub(crate) fn accept_start_probe(
    app: &tauri::AppHandle,
    id: &str,
    result: Probe,
) -> Result<(), String> {
    let state = app.state::<PortStates>();
    let mut entries = state.0.lock().unwrap();
    let current = entries.get_mut(id).ok_or("检测配置已变更，请重试。")?;
    match result {
        Probe::Closed => Ok(()),
        Probe::Open => {
            current.phase = None;
            current.observe(Probe::Open, Instant::now());
            Err("目标端口已在监听，已阻止重复启动。".into())
        }
        Probe::Error(message) => {
            current.phase = None;
            current.observe(Probe::Error(message.clone()), Instant::now());
            Err(format!("无法确认端口状态，未启动：{message}"))
        }
    }
}

pub(crate) fn operation_failed(app: &tauri::AppHandle, id: &str, message: &str) {
    let state = app.state::<PortStates>();
    if let Some(current) = state.0.lock().unwrap().get_mut(id) {
        current.phase = None;
        current.status = "error".into();
        current.message = message.into();
    };
}

pub(crate) fn launcher_finished(app: &tauri::AppHandle, id: &str, succeeded: bool) {
    if succeeded {
        return;
    }
    let state = app.state::<PortStates>();
    let mut entries = state.0.lock().unwrap();
    if let Some(current) = entries.get_mut(id) {
        if matches!(current.phase, Some((Phase::Starting, _))) {
            current.phase = None;
            current.status = "error".into();
            current.message = "启动脚本执行失败，正在重新检测端口".into();
        }
    }
}

pub(crate) fn begin_stop(
    app: &tauri::AppHandle,
    id: &str,
    config: &PortMonitor,
) -> Result<(), String> {
    let state = app.state::<PortStates>();
    let mut entries = state.0.lock().unwrap();
    let current = entry(&mut entries, id, config);
    if current.phase.is_some() {
        return Err("此条目正在启动或停止，请稍候。".into());
    }
    current.phase = Some((Phase::Stopping, Instant::now() + STOP_TIMEOUT));
    current.status = "stopping".into();
    current.message = "正在执行停止操作并等待端口关闭".into();
    Ok(())
}

pub(crate) fn wait_stopped(
    app: &tauri::AppHandle,
    id: &str,
    config: &PortMonitor,
) -> Result<(), String> {
    let deadline = app
        .state::<PortStates>()
        .0
        .lock()
        .unwrap()
        .get(id)
        .and_then(|current| current.phase.map(|(_, deadline)| deadline))
        .unwrap_or_else(Instant::now);
    loop {
        let result = probe(config);
        if result == Probe::Closed {
            let state = app.state::<PortStates>();
            let mut entries = state.0.lock().unwrap();
            let current = entry(&mut entries, id, config);
            current.phase = None;
            current.observe(result, Instant::now());
            return Ok(());
        }
        if Instant::now() >= deadline {
            let message = "停止操作结束后，仍未确认端口关闭。";
            operation_failed(app, id, message);
            return Err(message.into());
        }
        thread::sleep(Duration::from_millis(250));
    }
}

#[tauri::command]
pub fn get_script_states(app: tauri::AppHandle) -> Vec<RuntimeInfo> {
    let scripts = app.state::<ScriptRegistry>().0.lock().unwrap().clone();
    let managed: HashMap<_, _> = crate::runner::get_running_scripts(app.state())
        .into_iter()
        .map(|info| (info.id.clone(), info))
        .collect();
    let state = app.state::<PortStates>();
    let entries = state.0.lock().unwrap();
    scripts
        .into_iter()
        .map(|script| {
            let process = managed.get(&script.id);
            let can_stop = script.stop.is_some();
            let (status, message) = if let Some(config) = &script.monitor {
                match entries
                    .get(&script.id)
                    .filter(|value| &value.config == config)
                {
                    Some(value) => (value.status.clone(), value.message.clone()),
                    None => ("checking".into(), "正在检测端口".into()),
                }
            } else {
                (
                    match process {
                        Some(value) if value.paused => "paused",
                        Some(_) => "running",
                        None => "stopped",
                    }
                    .into(),
                    String::new(),
                )
            };
            RuntimeInfo {
                id: script.id,
                status,
                monitored: script.monitor.is_some(),
                managed: process.is_some(),
                can_stop: if script.monitor.is_some() {
                    can_stop
                } else {
                    can_stop || process.is_some()
                },
                message,
            }
        })
        .collect()
}

/// The worker keeps detecting even while both webview windows are hidden.
pub fn start_worker(app: tauri::AppHandle) {
    thread::spawn(move || loop {
        let scripts = match crate::scripts::read_scripts(&app) {
            Ok(scripts) => {
                *app.state::<ScriptRegistry>().0.lock().unwrap() = scripts.clone();
                scripts
            }
            Err(_) => app.state::<ScriptRegistry>().0.lock().unwrap().clone(),
        };
        let targets: Vec<_> = scripts
            .iter()
            .filter_map(|script| {
                script
                    .monitor
                    .clone()
                    .map(|config| (script.id.clone(), config))
            })
            .collect();
        for batch in targets.chunks(4) {
            let results = thread::scope(|scope| {
                let handles: Vec<_> = batch
                    .iter()
                    .map(|(id, config)| {
                        scope.spawn(move || (id.clone(), config.clone(), probe(config)))
                    })
                    .collect();
                handles
                    .into_iter()
                    .filter_map(|handle| handle.join().ok())
                    .collect::<Vec<_>>()
            });
            let state = app.state::<PortStates>();
            let mut entries = state.0.lock().unwrap();
            for (id, config, result) in results {
                if entries
                    .get(&id)
                    .is_some_and(|value| value.config != config && value.phase.is_some())
                {
                    continue;
                }
                entry(&mut entries, &id, &config).observe(result, Instant::now());
            }
        }
        app.state::<PortStates>().0.lock().unwrap().retain(|id, _| {
            scripts
                .iter()
                .any(|script| script.id == *id && script.monitor.is_some())
        });
        let _ = app.emit("script-states", get_script_states(app.clone()));
        thread::sleep(CHECK_INTERVAL);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripts::Script;
    use std::net::TcpListener;

    fn config(port: u16) -> PortMonitor {
        PortMonitor {
            kind: "tcp".into(),
            host: "127.0.0.1".into(),
            port,
        }
    }

    #[test]
    fn detects_external_listener_and_its_shutdown() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let config = config(listener.local_addr().unwrap().port());
        assert_eq!(probe(&config), Probe::Open);
        drop(listener);
        assert_eq!(probe(&config), Probe::Closed);
    }

    #[test]
    fn launcher_completion_does_not_stop_a_live_service() {
        let now = Instant::now();
        let mut state = Entry::new(config(8080));
        state.phase = Some((Phase::Starting, now + START_TIMEOUT));
        state.observe(Probe::Closed, now);
        assert_eq!(state.status, "starting");
        state.observe(Probe::Open, now);
        assert_eq!(state.status, "running");
        assert!(state.phase.is_none());
        state.observe(Probe::Open, now + Duration::from_secs(3));
        assert_eq!(state.status, "running");
    }

    #[test]
    fn stop_command_success_waits_for_actual_port_closure() {
        let now = Instant::now();
        let mut state = Entry::new(config(8080));
        state.phase = Some((Phase::Stopping, now + STOP_TIMEOUT));
        state.observe(Probe::Open, now);
        assert_eq!(state.status, "stopping");
        state.observe(Probe::Closed, now);
        assert_eq!(state.status, "stopped");
    }

    #[test]
    fn pending_operations_time_out_and_external_state_can_recover() {
        let now = Instant::now();
        for phase in [Phase::Starting, Phase::Stopping] {
            let mut state = Entry::new(config(8080));
            state.phase = Some((phase, now));
            state.observe(
                if phase == Phase::Starting {
                    Probe::Closed
                } else {
                    Probe::Open
                },
                now,
            );
            assert_eq!(state.status, "error");
            state.observe(Probe::Open, now + Duration::from_secs(1));
            assert_eq!(state.status, "running");
        }
    }

    #[test]
    fn network_errors_are_not_reported_as_stopped() {
        let mut state = Entry::new(config(8080));
        state.observe(Probe::Error("timeout".into()), Instant::now());
        assert_eq!(state.status, "error");
    }

    #[test]
    fn validates_target_and_resets_state_when_config_changes() {
        assert!(config(0).normalize().is_err());
        let mut invalid = config(8080);
        invalid.host = "invalid-host".into();
        assert!(invalid.normalize().is_err());
        invalid.host = "localhost".into();
        invalid.normalize().unwrap();
        assert_eq!(invalid.host, "127.0.0.1");
        let mut entries = HashMap::new();
        entry(&mut entries, "demo", &config(8080)).observe(Probe::Open, Instant::now());
        assert_eq!(
            entry(&mut entries, "demo", &config(8081)).status,
            "checking"
        );
    }

    #[test]
    fn legacy_config_deserializes_without_monitoring() {
        let script: Script =
            serde_json::from_str(r#"{"id":"legacy","name":"demo","path":"example.cmd"}"#).unwrap();
        assert!(script.monitor.is_none());
    }
}
