pub mod db;
pub mod edits;
pub mod model;
mod storage;

use model::*;
use tauri::State;

#[tauri::command]
async fn connect(
    state: State<'_, db::Database>,
    profile: Profile,
    password: Option<String>,
) -> Result<(), String> {
    profile.validate()?;
    let secret = if let Some(value) = password {
        value
    } else if profile.remember_password {
        storage::password(&profile.id)?
    } else {
        String::new()
    };
    state.connect(profile, secret).await
}
#[tauri::command]
async fn disconnect(state: State<'_, db::Database>, connection_id: String) -> Result<(), String> {
    state.disconnect(&connection_id).await
}
#[tauri::command]
async fn run_query(
    state: State<'_, db::Database>,
    connection_id: String,
    sql: String,
    operation_id: String,
) -> Result<QueryResult, String> {
    state.query(&connection_id, &sql, &operation_id).await
}
#[tauri::command]
async fn cancel_query(
    state: State<'_, db::Database>,
    connection_id: String,
    operation_id: String,
) -> Result<(), String> {
    state.cancel(&connection_id, &operation_id).await
}
#[tauri::command]
async fn list_tables(
    state: State<'_, db::Database>,
    connection_id: String,
) -> Result<Vec<TableInfo>, String> {
    state.tables(&connection_id).await
}
#[tauri::command]
async fn table_details(
    state: State<'_, db::Database>,
    connection_id: String,
    oid: u32,
) -> Result<TableDetails, String> {
    state.details(&connection_id, oid).await
}
#[tauri::command]
async fn preview_edits(
    state: State<'_, db::Database>,
    connection_id: String,
    result_id: String,
    edits: Vec<CellEdit>,
) -> Result<Vec<PlannedUpdate>, String> {
    state.preview(&connection_id, &result_id, &edits).await
}
#[tauri::command]
async fn apply_edits(
    state: State<'_, db::Database>,
    connection_id: String,
    result_id: String,
    edits: Vec<CellEdit>,
) -> Result<usize, String> {
    state.apply(&connection_id, &result_id, &edits).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(db::Database::default())
        .invoke_handler(tauri::generate_handler![
            connect,
            disconnect,
            run_query,
            cancel_query,
            list_tables,
            table_details,
            preview_edits,
            apply_edits,
            storage::list_profiles,
            storage::save_profile,
            storage::delete_profile
        ])
        .run(tauri::generate_context!())
        .expect("Could not start hey db");
}
