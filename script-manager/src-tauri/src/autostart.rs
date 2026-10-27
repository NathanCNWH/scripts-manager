//! Start opted-in projects once per manager process, independently of webviews.

use serde::Serialize;
use std::sync::Mutex;
use tauri::Manager;

#[derive(Clone, Serialize)]
pub struct StartupIssue {
    id: String,
    name: String,
    message: String,
}

#[derive(Default)]
pub struct StartupIssues(Mutex<Vec<StartupIssue>>);

/// Startup can finish before the webview subscribes to events. Keep errors
/// available until exit so opening the tray later still explains failures.
#[tauri::command]
pub fn get_autostart_errors(app: tauri::AppHandle) -> Vec<StartupIssue> {
    app.state::<StartupIssues>().0.lock().unwrap().clone()
}

pub fn start_projects(app: tauri::AppHandle) {
    let projects: Vec<_> = app
        .state::<crate::scripts::ScriptRegistry>()
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|script| script.autostart)
        .map(|script| (script.id.clone(), script.name.clone()))
        .collect();
    tauri::async_runtime::spawn(async move {
        for (id, name) in projects {
            // Re-read the flag in case the user disabled/removed a queued entry.
            match crate::scripts::find_script(&app, &id) {
                Ok(Some(script)) if script.autostart => {}
                Ok(_) => continue,
                Err(message) => {
                    app.state::<StartupIssues>()
                        .0
                        .lock()
                        .unwrap()
                        .push(StartupIssue { id, name, message });
                    continue;
                }
            }
            if let Err(message) = crate::runner::start_registered_script(
                id.clone(),
                app.clone(),
                app.state(),
                app.state(),
                true,
            )
            .await
            {
                app.state::<StartupIssues>()
                    .0
                    .lock()
                    .unwrap()
                    .push(StartupIssue { id, name, message });
            }
        }
    });
}
