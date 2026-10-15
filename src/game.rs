//! Finds the Deadlock install from Steam's library folders.

use std::path::PathBuf;

pub(crate) const DEADLOCK_APP_ID: u32 = 1422450;

/// The game's install folder, from whichever Steam library holds it.
pub(crate) fn install_dir() -> Option<PathBuf> {
    let steam_dir = steamlocate::SteamDir::locate().ok()?;
    let (app, library) = steam_dir.find_app(DEADLOCK_APP_ID).ok()??;
    Some(library.resolve_app_dir(&app))
}

/// Where the game writes its console when launched with `-condebug`.
pub(crate) fn console_log_path() -> Option<PathBuf> {
    Some(
        install_dir()?
            .join("game")
            .join("citadel")
            .join("console.log"),
    )
}
