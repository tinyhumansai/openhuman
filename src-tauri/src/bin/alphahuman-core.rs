fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(err) = alphahuman::core_server::run_from_cli_args(&args) {
        eprintln!("alphahuman-core failed: {err}");
        std::process::exit(1);
    }
}
