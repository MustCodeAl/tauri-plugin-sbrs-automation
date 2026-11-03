// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[tauri_runtime_cef::cef_entry_point]
fn main() {
    tauri_profile_manager_lib::run()
}
