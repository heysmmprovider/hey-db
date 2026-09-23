use crate::model::Profile;
use std::{fs, path::PathBuf};
use tauri::Manager;

const SERVICE: &str = "app.heydb.desktop";
static STORAGE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|_| "Cannot locate application settings.")?;
    fs::create_dir_all(&dir).map_err(|_| "Cannot create application settings directory.")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
            .map_err(|_| "Cannot protect settings directory.")?;
    }
    Ok(dir.join("connections.json"))
}
fn read(app: &tauri::AppHandle) -> Result<Vec<Profile>, String> {
    let file = path(app)?;
    if !file.exists() {
        return Ok(vec![]);
    }
    serde_json::from_slice(&fs::read(file).map_err(|_| "Cannot read saved connections.")?).map_err(
        |_| "Saved connections could not be read. Your settings have not been overwritten.".into(),
    )
}
fn write(app: &tauri::AppHandle, profiles: &[Profile]) -> Result<(), String> {
    let file = path(app)?;
    let temp = file.with_extension("json.tmp");
    fs::write(
        &temp,
        serde_json::to_vec_pretty(profiles).map_err(|_| "Cannot encode settings.")?,
    )
    .map_err(|_| "Cannot save connections.")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))
            .map_err(|_| "Cannot protect saved connections.")?;
    }
    fs::rename(temp, file).map_err(|_| "Cannot finish saving connections.".into())
}
pub fn password(id: &str) -> Result<String, String> {
    keyring::Entry::new(SERVICE, id)
        .map_err(|_| "Credential storage is unavailable.")?
        .get_password()
        .map_err(|_| "The saved password is unavailable. Enter it again to connect.".into())
}
#[tauri::command]
pub fn list_profiles(app: tauri::AppHandle) -> Result<Vec<Profile>, String> {
    let _guard = STORAGE_LOCK.lock().map_err(|_| "Settings are busy.")?;
    read(&app)
}
#[tauri::command]
pub fn save_profile(
    app: tauri::AppHandle,
    profile: Profile,
    password: Option<String>,
) -> Result<(), String> {
    let _guard = STORAGE_LOCK.lock().map_err(|_| "Settings are busy.")?;
    profile.validate()?;
    let mut profiles = read(&app)?;
    if profile.remember_password {
        if let Some(secret) = password {
            let entry = keyring::Entry::new(SERVICE, &profile.id)
                .map_err(|_| "Credential storage is unavailable.")?;
            entry.set_password(&secret).map_err(|_| {
                "Could not store the password in the operating system's credential store."
            })?;
        }
    } else if profiles
        .iter()
        .any(|p| p.id == profile.id && p.remember_password)
    {
        let entry = keyring::Entry::new(SERVICE, &profile.id)
            .map_err(|_| "Credential storage is unavailable.")?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(_) => {
                return Err("Could not remove the saved password from credential storage.".into())
            }
        }
    }
    profiles.retain(|p| p.id != profile.id);
    profiles.push(profile);
    write(&app, &profiles)
}
#[tauri::command]
pub fn delete_profile(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let _guard = STORAGE_LOCK.lock().map_err(|_| "Settings are busy.")?;
    uuid::Uuid::parse_str(&id).map_err(|_| "Invalid connection ID.")?;
    let mut profiles = read(&app)?;
    if profiles.iter().any(|p| p.id == id && p.remember_password) {
        let entry =
            keyring::Entry::new(SERVICE, &id).map_err(|_| "Credential storage is unavailable.")?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(_) => return Err("Could not remove the saved password.".into()),
        }
    }
    profiles.retain(|p| p.id != id);
    write(&app, &profiles)
}
