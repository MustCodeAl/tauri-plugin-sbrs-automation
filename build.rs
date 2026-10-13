const COMMANDS: &[&str] = &["reply"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
