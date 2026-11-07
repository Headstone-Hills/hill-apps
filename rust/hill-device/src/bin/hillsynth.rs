fn main() {
    if let Err(e) = hill_device::run("hillsynth") {
        eprintln!("hillsynth: {e}");
        std::process::exit(1);
    }
}
