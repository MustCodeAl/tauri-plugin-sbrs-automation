use std::path::PathBuf;
use std::sync::Arc;

use seleniumbase_rs::init_tracing;
use tauri::{AppHandle, Manager, Runtime, State, command, generate_context, generate_handler};
use tauri_runtime_cef::AsCefWebviewDispatcher;
use tracing::{error, info};

mod api;
mod app_window;
mod models;
mod passphrase;
mod session;
mod storage;
mod store;

use app_window::ProfileWindow;
use models::{NewProfile, Profile, RandomizeRequest, Randomized, SessionInfo, StorageStatus};
use seleniumbase_rs::OsType;
use session::Session;
use store::{AppState, next_api_port};

/// Whether the profile vault opened, for the window to explain when it did not.
#[command]
fn get_storage_status(state: State<'_, Arc<AppState>>) -> StorageStatus {
    state.storage_status()
}

#[command]
async fn list_profiles(state: State<'_, Arc<AppState>>) -> Result<Vec<Profile>, String> {
    state.ensure_storage().map_err(|e| e.to_string())?;
    let profiles = state.profiles.lock().await.clone();
    info!(count = profiles.len(), "listed profiles");
    Ok(profiles)
}

#[command]
async fn create_profile(
    state: State<'_, Arc<AppState>>,
    new: NewProfile,
) -> Result<Profile, String> {
    let profile = Profile::from_new(uuid::Uuid::new_v4().to_string(), new)?;
    state
        .add_profile(profile.clone())
        .await
        .map_err(|e| e.to_string())?;
    info!(profile_id = %profile.id, name = %profile.name, "created profile");
    Ok(profile)
}

#[command]
async fn delete_profile(state: State<'_, Arc<AppState>>, id: String) -> Result<(), String> {
    state.delete_profile(&id).await.map_err(|e| e.to_string())?;
    info!(profile_id = %id, "deleted profile");
    Ok(())
}

/// Gives a profile a new randomized identity; see
/// [`AppState::randomize_fingerprint`].
#[command]
async fn randomize_fingerprint(
    state: State<'_, Arc<AppState>>,
    id: String,
    os: Option<OsType>,
    seed: Option<u64>,
) -> Result<Randomized, String> {
    let randomized = state
        .randomize_fingerprint(&id, RandomizeRequest { os, seed })
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Profile not found".to_string())?;
    info!(profile_id = %id, os = ?randomized.os, "randomized fingerprint");
    Ok(randomized)
}

#[command]
async fn launch_profile(
    state: State<'_, Arc<AppState>>,
    id: String,
    start_url: Option<String>,
) -> Result<SessionInfo, String> {
    let profile = {
        let profiles = state.profiles.lock().await;
        profiles
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| "Profile not found".to_string())?
    };

    let session = Session::launch(&profile, start_url.as_deref())
        .await
        .inspect_err(
            |e| tracing::warn!(profile_id = %id, error = %e, "could not launch the profile"),
        )
        .map_err(|e| e.to_string())?;

    let session_id = store::make_session_id();
    let info = SessionInfo {
        session_id: session_id.clone(),
        profile_id: profile.id,
        profile_name: profile.name,
        container_url: profile.container_url,
        engine: profile.engine,
    };

    state
        .sessions
        .lock()
        .await
        .insert(session_id.clone(), session);
    state
        .session_info
        .lock()
        .await
        .insert(session_id, info.clone());
    info!(
        session_id = %info.session_id,
        profile_id = %info.profile_id,
        engine = ?info.engine,
        "launched profile"
    );
    Ok(info)
}

/// Opens a profile in a window of the app itself, with its fingerprint's
/// script in every page, through the plugin's stealth (see [`app_window`]).
#[command]
async fn open_profile_window<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, Arc<AppState>>,
    id: String,
    url: Option<String>,
) -> Result<(), String>
where
    R::WebviewDispatcher: AsCefWebviewDispatcher,
{
    let profile = state
        .profiles
        .lock()
        .await
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .ok_or_else(|| "Profile not found".to_string())?;
    // A profile with no fingerprint gets one drawn for this machine's
    // system and saved first, as `Randomize` does, so the window never shows
    // the real browser and the profile keeps the same identity next time.
    let profile = if store::build_config(&profile).fingerprint.is_some() {
        profile
    } else {
        let randomized = state
            .randomize_fingerprint(
                &id,
                RandomizeRequest {
                    os: None,
                    seed: None,
                },
            )
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Profile not found".to_string())?;
        info!(profile_id = %id, os = ?randomized.os, "drew a fingerprint for the app window");
        randomized.profile
    };
    let window = ProfileWindow::plan(&profile, url.as_deref()).inspect_err(
        |e| tracing::warn!(profile_id = %id, error = %e, "could not open the profile in an app window"),
    )?;
    let label = window.label.clone();
    window.open(&app).inspect_err(
        |e| tracing::warn!(profile_id = %id, error = %e, "could not open the profile in an app window"),
    )?;
    info!(profile_id = %profile.id, window = %label, "opened profile in an app window");
    Ok(())
}

#[command]
async fn list_sessions(state: State<'_, Arc<AppState>>) -> Result<Vec<SessionInfo>, String> {
    Ok(state.session_info.lock().await.values().cloned().collect())
}

#[command]
async fn navigate_session(
    state: State<'_, Arc<AppState>>,
    session_id: String,
    url: String,
) -> Result<(), String> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get_mut(&session_id)
        .ok_or_else(|| "Session not found".to_string())?;
    session.open(&url).await
}

#[command]
async fn take_screenshot(
    state: State<'_, Arc<AppState>>,
    session_id: String,
) -> Result<PathBuf, String> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get_mut(&session_id)
        .ok_or_else(|| "Session not found".to_string())?;
    session.screenshot().await
}

#[command]
async fn set_session_geolocation(
    state: State<'_, Arc<AppState>>,
    session_id: String,
    latitude: f64,
    longitude: f64,
    accuracy: Option<f64>,
) -> Result<(), String> {
    let mut sessions = state.sessions.lock().await;
    let session = sessions
        .get_mut(&session_id)
        .ok_or_else(|| "Session not found".to_string())?;
    session
        .set_geolocation(latitude, longitude, accuracy.unwrap_or(100.0))
        .await
}

#[command]
async fn close_session(state: State<'_, Arc<AppState>>, session_id: String) -> Result<(), String> {
    // Take the session out first, so closing a slow browser does not hold up
    // every other session.
    let mut session = state
        .sessions
        .lock()
        .await
        .remove(&session_id)
        .ok_or_else(|| "Session not found".to_string())?;
    state.session_info.lock().await.remove(&session_id);
    session.quit().await?;
    info!(session_id = %session_id, "closed session");
    Ok(())
}

#[command]
async fn get_api_base() -> Result<String, String> {
    Ok(format!("http://127.0.0.1:{}", next_api_port()))
}

/// A button or the address bar of a profile window's toolbar. The caller's
/// own label says which window it belongs to, so only a toolbar can use it.
#[command]
async fn control_profile_window<R: Runtime>(
    webview: tauri::Webview<R>,
    windows: State<'_, app_window::OpenWindows>,
    action: Option<app_window::PageAction>,
    address: Option<String>,
) -> Result<(), String> {
    app_window::control(&windows, webview.label(), action, address.as_deref()).await
}

/// Hands this run's API token to the app window.
///
/// Reaching the token requires Tauri IPC, which is available only to the
/// app's own frontend. A web page can send requests to the loopback API but
/// has no way to obtain the token they must carry.
#[command]
async fn get_api_token(state: State<'_, Arc<AppState>>) -> Result<String, String> {
    Ok(state.api_token.clone())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();

    let builder = tauri::Builder::default()
        // Tauri 3 runs no runtime unless the app names one; without this the
        // app stops at once with `RuntimeNotConfigured`.
        .runtime(tauri_runtime_cef::Cef::default())
        .plugin(tauri_plugin_opener::init())
        .manage(app_window::OpenWindows::default());
    // A WebDriver endpoint for the window, for `tests/e2e_automation.rs`.
    #[cfg(feature = "e2e")]
    let builder = builder.plugin(tauri_plugin_sbrs_automation::init());

    builder
        .setup(|app| {
            // Open the vault before anything can ask for a profile. This takes
            // a fraction of a second, because deriving the key is meant to be
            // slow.
            let data_dir = app.path().app_data_dir()?;
            let opened = tauri::async_runtime::block_on(storage::open_default(&data_dir));
            let state = Arc::new(AppState::from_opened(opened));
            app.manage(state.clone());

            std::thread::spawn(move || {
                actix_web::rt::System::new().block_on(async move {
                    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], next_api_port()));
                    if let Err(e) = api::start_server(state, addr).await {
                        error!(error = %e, "external profile API server stopped");
                    }
                });
            });
            Ok(())
        })
        .invoke_handler(generate_handler![
            get_storage_status,
            list_profiles,
            create_profile,
            delete_profile,
            randomize_fingerprint,
            launch_profile,
            open_profile_window,
            control_profile_window,
            list_sessions,
            navigate_session,
            take_screenshot,
            set_session_geolocation,
            close_session,
            get_api_base,
            get_api_token,
        ])
        .run(generate_context!())
        .expect("error while running tauri application");
}
