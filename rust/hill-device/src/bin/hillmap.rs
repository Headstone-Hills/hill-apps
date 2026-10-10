fn main() {
    if let Err(e) = hill_device::mapping_wizard() {
        eprintln!("hillmap: {e}");
        std::process::exit(1);
    }
}
