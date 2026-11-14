// No console window in release builds; debug builds keep it for logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    contextflow_app::run();
}
