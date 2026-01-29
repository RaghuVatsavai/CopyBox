#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use arboard::{Clipboard, Error as ClipboardError, ImageData};
use enigo::{Direction, Enigo, Key, Keyboard, Settings as EnigoSettings};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{
    menu::Menu,
    menu::MenuItemBuilder,
    tray::TrayIconBuilder,
    AppHandle,
    Emitter,
    Manager,
    State,
    WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

const HISTORY_FILE: &str = "history.json";
const IMAGES_DIR: &str = "images";
const MAX_HISTORY_LIMIT: usize = 99;
const DEFAULT_HISTORY_LIMIT: usize = 50;
const POLL_INTERVAL: Duration = Duration::from_millis(600);

#[derive(Clone)]
struct AppState {
    history: Arc<Mutex<HistoryState>>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct HistorySettings {
    max_items: usize,
    auto_paste: bool,
    capture_paused: bool,
    theme: String,
}

impl Default for HistorySettings {
    fn default() -> Self {
        Self {
            max_items: DEFAULT_HISTORY_LIMIT,
            auto_paste: false,
            capture_paused: false,
            theme: "light".to_string(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClipboardItem {
    id: String,
    created_at: u64,
    #[serde(flatten)]
    content: ClipboardContent,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum ClipboardContent {
    Text { text: String },
    Link { url: String },
    Files { paths: Vec<String> },
    Image { path: String, width: u32, height: u32 },
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CaptureStatus {
    last_capture_at: Option<u64>,
    last_error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredState {
    items: Vec<ClipboardItem>,
    settings: HistorySettings,
    #[serde(default)]
    status: CaptureStatus,
}

impl Default for StoredState {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            settings: HistorySettings::default(),
            status: CaptureStatus::default(),
        }
    }
}

struct HistoryState {
    data: StoredState,
    last_seen_hash: Option<String>,
    skip_next_hash: Option<String>,
}

#[tauri::command]
fn get_history(state: State<'_, AppState>) -> StoredState {
    let history = state.history.lock().expect("history lock poisoned");
    history.data.clone()
}

#[tauri::command]
fn select_item(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<StoredState, String> {
    let (item, settings, payload) = {
        let mut history = state.history.lock().map_err(|_| "history lock poisoned")?;
        let Some(index) = history.data.items.iter().position(|item| item.id == id) else {
            return Err("Item not found".to_string());
        };
        let mut item = history.data.items.remove(index);
        item.created_at = now_millis();
        let max_items = history.data.settings.max_items;
        history.data.items.insert(0, item.clone());
        history.data.items.truncate(max_items);
        history.skip_next_hash = Some(item.id.clone());
        persist_state(&app, &history.data);
        let payload = history.data.clone();
        let settings = history.data.settings.clone();
        (item, settings, payload)
    };

    set_clipboard(&app, &item)?;

    if settings.auto_paste {
        trigger_auto_paste();
    }

    emit_history(&app, &payload);
    hide_overlay(&app);
    Ok(payload)
}

#[tauri::command]
fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    max_items: usize,
    auto_paste: bool,
    theme: String,
) -> Result<StoredState, String> {
    let payload = {
        let mut history = state.history.lock().map_err(|_| "history lock poisoned")?;
        let max_items = max_items.clamp(1, MAX_HISTORY_LIMIT);
        history.data.settings.max_items = max_items;
        history.data.settings.auto_paste = auto_paste;
        history.data.settings.theme = normalize_theme(&theme);
        history.data.items.truncate(max_items);
        persist_state(&app, &history.data);
        history.data.clone()
    };

    emit_history(&app, &payload);
    Ok(payload)
}

#[tauri::command]
fn clear_history(app: AppHandle, state: State<'_, AppState>) -> Result<StoredState, String> {
    let payload = {
        let mut history = state.history.lock().map_err(|_| "history lock poisoned")?;
        for item in &history.data.items {
            if let ClipboardContent::Image { path, .. } = &item.content {
                let _ = fs::remove_file(path);
            }
        }
        history.data.items.clear();
        history.last_seen_hash = None;
        history.skip_next_hash = None;
        persist_state(&app, &history.data);
        history.data.clone()
    };

    emit_history(&app, &payload);
    Ok(payload)
}

#[tauri::command]
fn toggle_capture(app: AppHandle, state: State<'_, AppState>) -> Result<StoredState, String> {
    let payload = {
        let mut history = state.history.lock().map_err(|_| "history lock poisoned")?;
        history.data.settings.capture_paused = !history.data.settings.capture_paused;
        if history.data.settings.capture_paused {
            history.data.status.last_error = Some("Capture paused".to_string());
        } else if history.data.status.last_error.as_deref() == Some("Capture paused") {
            history.data.status.last_error = None;
        }
        persist_state(&app, &history.data);
        history.data.clone()
    };

    emit_history(&app, &payload);
    Ok(payload)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            let stored_state = load_state(app.handle());
            let history = Arc::new(Mutex::new(HistoryState {
                data: stored_state,
                last_seen_hash: None,
                skip_next_hash: None,
            }));
            app.manage(AppState {
                history: history.clone(),
            });
            start_clipboard_watcher(app.handle().clone(), history);
            setup_tray(app.handle())?;
            setup_window_events(app.handle());
            if let Err(error) = app.global_shortcut().on_shortcut(
                "CommandOrControl+Shift+V",
                |app, _shortcut, event| {
                    if event.state != ShortcutState::Pressed {
                        return;
                    }
                    if let Some(window) = app.get_webview_window("main") {
                        let is_visible = window.is_visible().unwrap_or(false);
                        if is_visible {
                            hide_overlay(app);
                        } else {
                            show_overlay(app);
                        }
                    }
                },
            ) {
                eprintln!("Failed to register global shortcut: {error}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_history,
            select_item,
            update_settings,
            clear_history,
            toggle_capture
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItemBuilder::with_id("open", "Open CopyBox").build(app)?;
    let pause = MenuItemBuilder::with_id("pause", "Pause Capture").build(app)?;
    let clear = MenuItemBuilder::with_id("clear", "Clear History").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
    let menu = Menu::with_items(app, &[&open, &pause, &clear, &quit])?;

    let mut tray_builder = TrayIconBuilder::new()
        .tooltip("CopyBox")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => {
                show_overlay(app);
            }
            "pause" => {
                let state = app.state::<AppState>();
                let _ = toggle_capture(app.clone(), state);
            }
            "clear" => {
                let state = app.state::<AppState>();
                let _ = clear_history(app.clone(), state);
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        tray_builder = tray_builder.icon(icon);
    }

    tray_builder.build(app)?;

    Ok(())
}

fn setup_window_events(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_visible_on_all_workspaces(true);
        let app_handle = app.clone();
        window.on_window_event(move |event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                hide_overlay(&app_handle);
            }
            WindowEvent::Focused(false) => {
                hide_overlay(&app_handle);
            }
            _ => {}
        });
    }
}

fn show_overlay(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_visible_on_all_workspaces(true);
        let _ = window.show();
        let _ = window.set_focus();
    }
    let _ = app.emit("open-overlay", ());
}

fn hide_overlay(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    let _ = app.emit("close-overlay", ());
}

fn start_clipboard_watcher(app: AppHandle, history: Arc<Mutex<HistoryState>>) {
    std::thread::spawn(move || {
        let mut clipboard = Clipboard::new().ok();
        loop {
            let capture_paused = {
                let state = history.lock().expect("history lock poisoned");
                state.data.settings.capture_paused
            };

            if !capture_paused {
                if clipboard.is_none() {
                    match Clipboard::new() {
                        Ok(instance) => {
                            clipboard = Some(instance);
                        }
                        Err(error) => {
                            let payload = {
                                let mut state = history.lock().expect("history lock poisoned");
                                if mark_capture_error(&mut state.data, error.to_string()) {
                                    persist_state(&app, &state.data);
                                    Some(state.data.clone())
                                } else {
                                    None
                                }
                            };

                            if let Some(payload) = payload {
                                emit_history(&app, &payload);
                            }
                            std::thread::sleep(POLL_INTERVAL);
                            continue;
                        }
                    }
                }

                if let Some(clipboard) = clipboard.as_mut() {
                    match read_clipboard_item(&app, clipboard) {
                        Ok(Some(item)) => {
                            let payload = {
                                let mut state = history.lock().expect("history lock poisoned");
                                if state.skip_next_hash.as_deref() == Some(&item.id) {
                                    state.skip_next_hash = None;
                                    state.last_seen_hash = Some(item.id);
                                    None
                                } else if state.last_seen_hash.as_deref() == Some(&item.id) {
                                    None
                                } else {
                                    state.last_seen_hash = Some(item.id.clone());
                                    upsert_history_item(&mut state.data, item);
                                    mark_capture_success(&mut state.data);
                                    persist_state(&app, &state.data);
                                    Some(state.data.clone())
                                }
                            };

                            if let Some(payload) = payload {
                                emit_history(&app, &payload);
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            let payload = {
                                let mut state = history.lock().expect("history lock poisoned");
                                if mark_capture_error(&mut state.data, error.to_string()) {
                                    persist_state(&app, &state.data);
                                    Some(state.data.clone())
                                } else {
                                    None
                                }
                            };

                            if let Some(payload) = payload {
                                emit_history(&app, &payload);
                            }
                        }
                    }
                }
            }

            std::thread::sleep(POLL_INTERVAL);
        }
    });
}

fn upsert_history_item(data: &mut StoredState, item: ClipboardItem) {
    if let Some(index) = data.items.iter().position(|entry| entry.id == item.id) {
        let mut existing = data.items.remove(index);
        existing.created_at = item.created_at;
        data.items.insert(0, existing);
    } else {
        data.items.insert(0, item);
    }
    let max_items = data.settings.max_items;
    data.items.truncate(max_items);
}

fn read_clipboard_item(
    app: &AppHandle,
    clipboard: &mut Clipboard,
) -> Result<Option<ClipboardItem>, ClipboardError> {
    match clipboard.get_text() {
        Ok(text) => {
            let trimmed = text.trim().to_string();
            if trimmed.is_empty() {
                return Ok(None);
            }
            return Ok(Some(build_text_item(trimmed)));
        }
        Err(error) => {
            if !matches!(error, ClipboardError::ContentNotAvailable) {
                return Err(error);
            }
        }
    }

    match clipboard.get_image() {
        Ok(image) => Ok(build_image_item(app, image)),
        Err(error) => {
            if matches!(error, ClipboardError::ContentNotAvailable) {
                Ok(None)
            } else {
                Err(error)
            }
        }
    }
}

fn build_text_item(text: String) -> ClipboardItem {
    let content = if looks_like_url(&text) {
        ClipboardContent::Link { url: text.clone() }
    } else if let Some(paths) = parse_file_paths(&text) {
        ClipboardContent::Files { paths }
    } else {
        ClipboardContent::Text { text: text.clone() }
    };

    let id = match &content {
        ClipboardContent::Text { text } => hash_text(text),
        ClipboardContent::Link { url } => hash_text(url),
        ClipboardContent::Files { paths } => hash_text(&paths.join("\n")),
        ClipboardContent::Image { .. } => hash_text(&text),
    };

    ClipboardItem {
        id,
        created_at: now_millis(),
        content,
    }
}

fn build_image_item(app: &AppHandle, image: ImageData<'_>) -> Option<ClipboardItem> {
    let hash = hash_bytes(image.bytes.as_ref());
    let width = image.width as u32;
    let height = image.height as u32;
    let path = persist_image(app, &hash, image)?;
    Some(ClipboardItem {
        id: hash,
        created_at: now_millis(),
        content: ClipboardContent::Image { path, width, height },
    })
}

fn persist_image(app: &AppHandle, hash: &str, image: ImageData<'_>) -> Option<String> {
    let dir = images_dir(app)?;
    let _ = fs::create_dir_all(&dir);
    let path = dir.join(format!("{hash}.png"));
    if !path.exists() {
        let buffer = image::RgbaImage::from_raw(
            image.width as u32,
            image.height as u32,
            image.bytes.into_owned(),
        )?;
        if buffer.save(&path).is_err() {
            return None;
        }
    }
    Some(path.to_string_lossy().to_string())
}

fn parse_file_paths(text: &str) -> Option<Vec<String>> {
    let candidates = text
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    if candidates.is_empty() {
        return None;
    }

    let all_exist = candidates
        .iter()
        .all(|path| Path::new(path).exists());

    if all_exist {
        Some(candidates)
    } else {
        None
    }
}

fn looks_like_url(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

fn hash_text(text: &str) -> String {
    hash_bytes(text.as_bytes())
}

fn hash_bytes(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn set_clipboard(_app: &AppHandle, item: &ClipboardItem) -> Result<(), String> {
    let mut clipboard = Clipboard::new().map_err(|err| err.to_string())?;
    match &item.content {
        ClipboardContent::Text { text } => clipboard
            .set_text(text.clone())
            .map_err(|err| err.to_string())?,
        ClipboardContent::Link { url } => clipboard
            .set_text(url.clone())
            .map_err(|err| err.to_string())?,
        ClipboardContent::Files { paths } => clipboard
            .set_text(paths.join("\n"))
            .map_err(|err| err.to_string())?,
        ClipboardContent::Image { path, .. } => {
            let image = image::open(path).map_err(|err| err.to_string())?;
            let rgba = image.to_rgba8();
            let width = rgba.width() as usize;
            let height = rgba.height() as usize;
            clipboard
                .set_image(ImageData {
                    width,
                    height,
                    bytes: Cow::Owned(rgba.into_raw()),
                })
                .map_err(|err| err.to_string())?;
        }
    }
    Ok(())
}

fn trigger_auto_paste() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(140));
        let Ok(mut enigo) = Enigo::new(&EnigoSettings::default()) else {
            return;
        };

        #[cfg(target_os = "macos")]
        let modifier = Key::Meta;
        #[cfg(not(target_os = "macos"))]
        let modifier = Key::Control;

        let _ = enigo.key(modifier, Direction::Press);
        let _ = enigo.key(Key::Unicode('v'), Direction::Click);
        let _ = enigo.key(modifier, Direction::Release);
    });
}

fn emit_history(app: &AppHandle, payload: &StoredState) {
    let _ = app.emit("history-updated", payload);
}

fn mark_capture_success(data: &mut StoredState) {
    data.status.last_capture_at = Some(now_millis());
    data.status.last_error = None;
}

fn mark_capture_error(data: &mut StoredState, error: String) -> bool {
    if data.status.last_error.as_deref() == Some(&error) {
        return false;
    }
    data.status.last_error = Some(error);
    true
}

fn normalize_theme(value: &str) -> String {
    if value.eq_ignore_ascii_case("dark") {
        "dark".to_string()
    } else {
        "light".to_string()
    }
}

fn load_state(app: &AppHandle) -> StoredState {
    let path = history_path(app);
    let Some(path) = path else {
        return StoredState::default();
    };

    let Ok(data) = fs::read(&path) else {
        return StoredState::default();
    };

    let mut stored: StoredState = serde_json::from_slice(&data).unwrap_or_default();
    stored.settings.max_items = stored.settings.max_items.clamp(1, MAX_HISTORY_LIMIT);
    stored.settings.theme = normalize_theme(&stored.settings.theme);
    stored.items.truncate(stored.settings.max_items);
    stored
}

fn persist_state(app: &AppHandle, data: &StoredState) {
    let Some(path) = history_path(app) else {
        return;
    };

    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    if let Ok(serialized) = serde_json::to_vec_pretty(data) {
        let _ = fs::write(path, serialized);
    }
}

fn history_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|dir| dir.join(HISTORY_FILE))
}

fn images_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|dir| dir.join(IMAGES_DIR))
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
