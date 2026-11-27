//! Checks the release feed (tauri.conf.json plugins.updater → GitHub releases) shortly after launch and
//! every 6 hours. An update is offered, never forced, and never installed while a transcript or a
//! recording is running. A declined version isn't offered again until the app restarts.
use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_updater::UpdaterExt;

pub fn spawn(app: AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    std::thread::spawn(move || {
        let mut declined: Option<String> = None;
        std::thread::sleep(std::time::Duration::from_secs(15));
        loop {
            if let Err(e) = tauri::async_runtime::block_on(check(&app, &mut declined)) {
                eprintln!("[updater] {e}");
            }
            std::thread::sleep(std::time::Duration::from_secs(6 * 3600));
        }
    });
}

fn busy() -> bool {
    crate::pipeline::any_live() || crate::record::active()
}

async fn check(app: &AppHandle, declined: &mut Option<String>) -> Result<(), String> {
    if busy() {
        return Ok(());
    }
    let Some(update) = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())? else {
        return Ok(());
    };
    if declined.as_deref() == Some(update.version.as_str()) {
        return Ok(());
    }
    // ponytail: English only; the UI language lives in the webview's localStorage
    let yes = app
        .dialog()
        .message(format!("Hanmadi {} is available. You have {}.", update.version, update.current_version))
        .title("Update Hanmadi")
        .buttons(MessageDialogButtons::OkCancelCustom("Install and restart".into(), "Later".into()))
        .blocking_show();
    if !yes {
        *declined = Some(update.version.clone());
        return Ok(());
    }
    // a job started while the question was open: ask again next time
    if busy() {
        return Ok(());
    }
    update.download_and_install(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    app.restart();
}
