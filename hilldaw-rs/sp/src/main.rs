//! HillDAW for the RG35XXSP: the Hill apps' SDL runner around `hilldaw_app::HillDaw`.

use hilldaw_app::status::{default_storage_path, spawn_poller, Status};
use hilldaw_app::HillDaw;
use std::sync::{Arc, Mutex};

fn main() {
    let status = Arc::new(Mutex::new(Status::default()));
    spawn_poller(status.clone(), default_storage_path());
    let result = hill_device::run_with("hilldaw", move |host| Some(Box::new(HillDaw::new(host, status))));
    if let Err(e) = result {
        eprintln!("hilldaw: {e}");
        std::process::exit(1);
    }
}
