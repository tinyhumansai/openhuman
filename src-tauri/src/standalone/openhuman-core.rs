fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(err) = openhuman::core_server::run_from_cli_args(&args) {
        eprintln!("openhuman-core failed: {err}");
        std::process::exit(1);
    }
}
