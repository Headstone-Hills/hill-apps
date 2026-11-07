//! HillDAW for the RG35XXSP: the Hill apps' SDL runner around `hilldaw_app::HillDaw`.

use hilldaw_app::fm1link::{Channels, Fm1Link};
use hilldaw_app::recorder::UsbRecorder;
use hilldaw_app::status::{default_storage_path, spawn_poller, Status};
use hilldaw_app::HillDaw;
use std::sync::{Arc, Mutex};

fn main() {
    let status = Arc::new(Mutex::new(Status::default()));
    spawn_poller(status.clone(), default_storage_path());
    // recordings live in <project folder>/audio/; the launcher sets HILL_DATA to the program's own folder
    let project_dir = std::env::var("HILL_PROJECTS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HILL_DATA").unwrap_or_else(|_| ".".into())).join("projects").join("song"));
    let result = hill_device::run_with("hilldaw", move |host| {
        let app = HillDaw::new(host, status)
            .with_fm1(Box::new(Fm1Link::spawn_usb(Channels::default())))
            .with_recorder(Box::new(UsbRecorder::new(project_dir)))
            .with_audio_out(Box::new(hilldaw_app::usbout::UsbAudioOut::new()));
        Some(Box::new(app))
    });
    if let Err(e) = result {
        eprintln!("hilldaw: {e}");
        std::process::exit(1);
    }
}
