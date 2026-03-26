// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("core") {
        if let Err(err) = openhuman::run_core_from_args(&args[2..]) {
            eprintln!("core process failed: {err}");
            std::process::exit(1);
        }
        return;
    }

    openhuman::run()
}
