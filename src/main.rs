fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(err) = openhuman_core::run_core_from_args(&args) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
