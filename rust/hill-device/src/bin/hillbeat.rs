fn main() {
    if let Err(e) = hill_device::run("hillbeat") {
        eprintln!("hillbeat: {e}");
        std::process::exit(1);
    }
}
