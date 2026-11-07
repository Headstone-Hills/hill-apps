fn main() {
    if let Err(e) = hill_device::run("hillchord") {
        eprintln!("hillchord: {e}");
        std::process::exit(1);
    }
}
