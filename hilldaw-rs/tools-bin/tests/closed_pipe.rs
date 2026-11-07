//! Regression: `hdtool drums list | head` used to die with "failed printing to stdout: Broken pipe" (exit 101, or an
//! abort in the release build) because `println!` panics when the reader goes away. A tool whose output is meant to be
//! piped must just stop quietly.

use std::process::{Command, Stdio};

#[test]
fn drums_list_stops_quietly_when_the_reader_closes_the_pipe() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_hdtool"))
        .args(["drums", "list"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start hdtool");
    // close our end before the tool has finished starting up, so its first write has nobody to go to
    drop(child.stdout.take());
    let out = child.wait_with_output().expect("wait");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "exit {:?}, stderr: {err}", out.status.code());
    assert!(!err.contains("panicked"), "stderr: {err}");
}
