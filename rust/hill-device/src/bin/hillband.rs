fn main() {
    if let Err(e) = hill_device::run("hillband") {
        eprintln!("hillband: {e}");
        std::process::exit(1);
    }
}
