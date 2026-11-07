fn main() {
    if let Err(e) = hill_device::run("hillsequencer") {
        eprintln!("hillsequencer: {e}");
        std::process::exit(1);
    }
}
