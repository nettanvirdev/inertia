//! The Tauri shell.
//!
//! This crate is intentionally the thinnest layer in the project. It owns
//! native window chrome, pre-workspace preferences, the command surface the
//! frontend calls, and the wiring that decides which concrete implementation
//! of each core trait the running app gets.
//!
//! All domain logic - providers, tools, storage, the agent loop - lives in
//! `crates/` and has no knowledge of Tauri. That boundary is what lets the same
//! logic run headless against mocks in `inertia-devkit`, and it is worth
//! defending: if you find yourself reaching for `tauri::` inside a `crates/`
//! member, the dependency is pointing the wrong way.

mod agents;
mod changes;
mod coalesce;
mod commands;
mod computer_tools;
mod computers;
mod cookies;
mod crew;
mod crew_commands;
mod desktop;
mod failures;
mod group;
mod group_tools;
mod hooks;
mod inertia_tools;
mod integrations;
mod llm;
mod logos;
mod memory;
mod notify;
mod permission;
mod platform;
mod preview;
mod project;
mod prompt;
mod question;
mod records;
mod routines;
mod skills;
mod state;
mod supervisor;
mod task_tool;
mod terminal;
mod tool_access;
mod turn;
mod voice;
mod ws;

pub use platform::window::apply_rounded_corners;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "inertia=info".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(state::AppState::new())
        // Process-wide and shared with the turn loop: the pane shows what has
        // run since the app started, across every thread.
        .manage(std::sync::Arc::new(inertia_hooks::Recorder::new()))
        .invoke_handler(tauri::generate_handler![
            // the small desktop errands
            desktop::app_info,
            desktop::app_fetch_image,
            desktop::app_save_image,
            desktop::app_run_snippet,
            desktop::app_can_run,
            desktop::app_clipboard_read,
            // providers, for the settings screen
            llm::llm_providers,
            llm::llm_models,
            llm::llm_test,
            // One completion with no tools: what the title generator and
            // `/compact` are made of.
            llm::llm_chat,
            llm::llm_cancel,
            llm::llm_cancel_all,
            llm::agent_summarize,
            // computers
            computers::computer_providers,
            computers::computer_settings,
            computers::computer_save_settings,
            computers::computer_list,
            computers::computer_refresh,
            computers::computer_create,
            computers::computer_start,
            computers::computer_stop,
            computers::computer_pause,
            computers::computer_resume,
            computers::computer_remove,
            computers::computer_exec,
            computers::computer_list_dir,
            computers::computer_read_file,
            computers::computer_read_file_bytes,
            computers::computer_download_file,
            computers::computer_write_file,
            computers::computer_snapshot,
            computers::computer_snapshots,
            computers::computer_restore,
            computers::computer_screenshot,
            computers::computer_assign,
            // The sandbox image the machines are built from, and the desktop
            // half: a live screen, and driving it.
            computers::computer_image,
            computers::computer_build_image,
            computers::computer_catalogue,
            computers::computer_cancel,
            // Bringing a signed-in session from this desktop into one of the
            // two browsers the app owns. Buttons, never tools - see
            // `cookies.rs`.
            cookies::cookie_sources,
            cookies::preview_import_cookies,
            cookies::computer_import_cookies,
            computers::computer_drive,
            computers::computer_screen,
            // voice
            voice::voice_configured,
            voice::voice_test,
            voice::voice_voices,
            voice::voice_models,
            voice::voice_speak,
            voice::voice_transcribe,
            // lifecycle hooks
            hooks::hooks_list,
            hooks::hooks_recent,
            hooks::hooks_write_example,
            hooks::hooks_set_enabled,
            hooks::hooks_reveal,
            // memory
            memory::memory_status,
            memory::memory_set_project,
            memory::memory_recall,
            memory::memory_save,
            memory::memory_forget,
            memory::memory_capture_now,
            // What Inertia does when its window is gone.
            notify::background_settings,
            notify::background_set_minimise_to_tray,
            notify::notifications_settings,
            notify::notifications_set,
            notify::background_set_launch_at_login,
            // setting a project up
            project::project_status,
            project::project_create,
            project::project_decline,
            project::project_decide,
            // the generic workspace surface the renderer is written against:
            // it names collections and documents, never paths
            ws::ws_status,
            ws::ws_layout,
            ws::ws_tree,
            ws::ws_browse,
            ws::ws_choose_folder,
            ws::ws_inspect,
            ws::ws_configure,
            ws::ws_reset,
            ws::ws_wipe,
            ws::ws_reveal,
            ws::ws_list_dir,
            ws::ws_list,
            ws::ws_get,
            ws::ws_put,
            ws::ws_patch,
            ws::ws_remove,
            ws::ws_rename,
            ws::ws_doc_get,
            ws::ws_doc_set,
            ws::ws_secret_list,
            ws::ws_secret_get,
            ws::ws_secret_set,
            ws::ws_secret_remove,
            ws::ws_file_read,
            ws::ws_file_write,
            ws::ws_file_read_bytes,
            ws::ws_file_write_bytes,
            ws::ws_file_read_image,
            ws::ws_file_remove,
            // MCP servers
            commands::mcp_list,
            commands::mcp_save,
            commands::mcp_delete,
            integrations::mcp_connect,
            integrations::mcp_disconnect,
            integrations::mcp_test,
            integrations::mcp_save_patch,
            integrations::mcp_status,
            integrations::mcp_tools,
            // OpenAPI imports
            commands::openapi_import,
            commands::openapi_list,
            commands::openapi_delete,
            integrations::openapi_update,
            integrations::openapi_operations,
            integrations::openapi_set_operations,
            integrations::openapi_details,
            integrations::openapi_test,
            // Composio
            commands::composio_start_connection,
            commands::composio_refresh,
            commands::composio_reconnect,
            commands::composio_disconnect,
            commands::composio_load_all,
            integrations::composio_configured,
            integrations::composio_connections,
            integrations::composio_permissions,
            integrations::composio_set_permissions,
            integrations::composio_toolkits,
            integrations::composio_tools,
            integrations::composio_connection_status,
            integrations::composio_logo,
            integrations::composio_invalidate,
            // running a turn, in the vocabulary the renderer speaks
            turn::agent_run,
            turn::agent_cancel,
            turn::agent_cancel_all,
            turn::agent_active,
            turn::agent_steer,
            turn::tools_list,
            // The crew panel: what the team is doing, and its buttons.
            crew_commands::crew_snapshot,
            crew_commands::crew_cancel,
            crew_commands::crew_pause,
            crew_commands::crew_resume,
            crew_commands::crew_restart,
            crew_commands::crew_interrupt,
            crew_commands::crew_followup,
            crew_commands::crew_timeline,
            crew_commands::crew_watch,
            crew_commands::crew_forget,
            // The terminal beside the conversation: a real pty per tab.
            // The browser pane: a child webview layered over the window.
            preview::preview_open,
            preview::preview_navigate,
            preview::preview_history,
            preview::preview_place,
            preview::preview_state,
            preview::preview_list,
            preview::preview_console,
            preview::preview_network,
            preview::preview_dev_tools,
            preview::preview_close,
            terminal::terminal_capability,
            terminal::terminal_open,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_run,
            terminal::terminal_interrupt,
            terminal::terminal_read,
            terminal::terminal_set_cwd,
            terminal::terminal_close,
            terminal::terminal_list,
            turn::permission_reply,
            // The other thing that suspends a tool: the model asking the
            // person something and waiting for the answer.
            question::agent_questions_waiting,
            question::agent_answer,
            question::agent_dismiss,
            turn::permission_waiting,
            turn::permission_grants,
            turn::permission_revoke,
            turn::agent_rules,
            turn::agent_open_path,
            turn::agent_forget,
            // What a turn actually did, kept so a reload has something to draw
            // and a conversation from last week can be read back.
            // What a turn changed on disk, and the way back.
            changes::snapshot_available,
            changes::snapshot_diff,
            changes::snapshot_revert,
            records::agent_record,
            records::agent_history,
            // Scheduled and triggered playbooks.
            routines::routine_run,
            routines::routine_next,
            routines::routine_describe,
            routines::routine_tick,
            turn::tools_invalidate,
        ])
        .setup(|app| {
            for (_, window) in tauri::Manager::webview_windows(app) {
                apply_rounded_corners(&window);
            }
            let recorder = tauri::Manager::state::<std::sync::Arc<inertia_hooks::Recorder>>(app);
            hooks::forward_runs(app.handle(), &recorder);
            // Brings the MCP servers up and keeps them up. Without it they were
            // configured and never started: every turn ran with none of their
            // tools, while the agents' own instructions named them.
            supervisor::start(app.handle());
            // Keeps time for the routines: writes each enabled one its next
            // run and starts a turn for anything due. The screen existed from
            // the first build; nothing was ever running the schedule.
            routines::start(app.handle());
            // Pushes a snapshot to the crew panel whenever a run changes.
            // Without it the panel draws once on mount and never moves again.
            crew_commands::start(app.handle());
            // Streams each terminal tab's output to the pane. Without it the
            // calls all work and the screen never moves.
            terminal::start(&app.handle().clone());

            // How a notice becomes a banner the operating system draws. A hook
            // rather than a direct call, so the deciding half stays testable
            // without a notification daemon.
            notify::set_banner(|app, notice| {
                use tauri_plugin_notification::NotificationExt;
                let _ = app
                    .notification()
                    .builder()
                    .title(&notice.title)
                    .body(&notice.body)
                    .show();
            });
            notify::set_login_item(std::sync::Arc::new(Autostart(app.handle().clone())));

            // The icon Inertia leaves behind when its window is gone. Without
            // it, a closed window with minimise-to-tray on is an app nobody can
            // reach and nobody can stop.
            if let Err(error) = build_tray(app) {
                tracing::warn!(%error, "the tray icon could not be created");
            }

            // Closing hides, unless the person turned that off.
            for (_, window) in tauri::Manager::webview_windows(app) {
                let handle = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        if notify::minimise_to_tray_now() {
                            api.prevent_close();
                            let _ = handle.hide();
                        }
                    }
                });
            }
            // A turn that was running when the app last stopped is not going to
            // finish: its loop died with the process. Settling them on the way
            // in keeps the history screen honest.
            if let Some(state) = tauri::Manager::try_state::<state::AppState>(app) {
                if let Ok(workspace) = state.workspace() {
                    records::settle_workspace(&workspace.layout);
                    // Warm the close handler's answer before a window can be
                    // closed: it runs on the UI thread and cannot wait on a
                    // file read.
                    notify::minimise_to_tray(&workspace.layout);
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| {
            // A conversation left open at quitting time is the second of the
            // two ways conversations actually end, and it is the one where a
            // capture pass has never had its quiet moment. The drain has its
            // own budget, so a slow model cannot hold the quit open.
            if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
                // A `npm run dev` in a terminal tab must not outlive the app
                // that is holding its port.
                terminal::shutdown();
                // A leaked language server is a gigabyte of memory with no
                // window attached to explain it.
                if let Some(state) = tauri::Manager::try_state::<state::AppState>(app) {
                    if let Ok(workspace) = state.workspace() {
                        let lsp = std::sync::Arc::clone(&workspace.lsp);
                        tauri::async_runtime::block_on(async move {
                            lsp.shutdown().await;
                        });
                    }
                }
                // Every turn still open is about to stop existing, so its
                // record is closed honestly rather than left saying "running"
                // for the next launch to puzzle over.
                records::Recorder::global().end_all("interrupted");
                if let Some(state) = tauri::Manager::try_state::<state::AppState>(app) {
                    let capture = std::sync::Arc::clone(&state.capture);
                    tauri::async_runtime::block_on(async move {
                        capture.drain().await;
                    });
                }
            }
        });
}

/// Starting with the machine: a registry entry on Windows, a login item on
/// macOS. Read back from the system every time rather than remembered, because
/// the person can change it there and the switch must show what is true.
#[derive(Debug)]
struct Autostart(tauri::AppHandle);

impl notify::LoginItem for Autostart {
    fn supported(&self) -> bool {
        true
    }

    fn enabled(&self) -> bool {
        use tauri_plugin_autostart::ManagerExt;
        self.0.autolaunch().is_enabled().unwrap_or(false)
    }

    fn set(&self, on: bool) -> Result<(), String> {
        use tauri_plugin_autostart::ManagerExt;
        let manager = self.0.autolaunch();
        if on { manager.enable() } else { manager.disable() }
            .map_err(|e| format!("The system refused to change what starts at login: {e}"))
    }
}

/// Bring the window back, however it went away.
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = tauri::Manager::get_webview_window(app, "main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// The tray icon and its menu.
///
/// Quitting goes through `app.exit` rather than a shorter teardown of its own,
/// so background commands, MCP servers and language servers are stopped by the
/// one path that knows how to stop them.
fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let open = MenuItem::with_id(app, "open", "Open Inertia", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Inertia", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;

    let mut tray = TrayIconBuilder::with_id("main")
        .menu(&menu)
        // A left click reopens the window; the menu belongs to the right one.
        .show_menu_on_left_click(false)
        .tooltip(notify::tray_tooltip("Inertia", 0))
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }
    tray.build(app)?;
    Ok(())
}
