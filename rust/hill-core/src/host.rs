use std::collections::BTreeMap;

/// What the platform provides to an app: the sample files it can see and a
/// place to keep state.
pub trait Host: Send {
    /// Relative sample paths (forward slashes), e.g. `Kit/Kick/kick1.wav`.
    fn list_samples(&self) -> Vec<String>;
    fn read_sample(&self, name: &str) -> Option<Vec<u8>>;
    fn load_state(&self, app: &str) -> Option<String>;
    fn save_state(&mut self, app: &str, data: &str);
}

/// In-memory host: used by tests and by the browser build, where JS pushes
/// files in and pulls saved state out.
#[derive(Default)]
pub struct MemHost {
    pub samples: BTreeMap<String, Vec<u8>>,
    pub state: BTreeMap<String, String>,
    pub state_dirty: bool,
}

impl Host for MemHost {
    fn list_samples(&self) -> Vec<String> {
        self.samples.keys().cloned().collect()
    }
    fn read_sample(&self, name: &str) -> Option<Vec<u8>> {
        self.samples.get(name).cloned()
    }
    fn load_state(&self, app: &str) -> Option<String> {
        self.state.get(app).cloned()
    }
    fn save_state(&mut self, app: &str, data: &str) {
        self.state.insert(app.to_string(), data.to_string());
        self.state_dirty = true;
    }
}
