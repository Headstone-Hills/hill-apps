//! FLAC and MP3 export: the WAV is rendered first, then converted by an encoder program on the device (`flac`, `lame`, with `ffmpeg` as a
//! fallback). The WAV is deleted only after the encoder succeeded; if no encoder works the WAV is kept and the report says so.

use hilldaw_core::project::ExportFormat;
use std::path::Path;
use std::process::{Command, Stdio};

/// An encoder invocation: program and arguments.
pub type Candidate = (String, Vec<String>);

/// The encoders to try, in order, to turn `wav` into `out`.
pub fn candidates(fmt: ExportFormat, wav: &str, out: &str) -> Vec<Candidate> {
    let c = |p: &str, a: &[&str]| (p.to_string(), a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    match fmt {
        ExportFormat::Wav => vec![],
        ExportFormat::Flac => vec![c("flac", &["-s", "--force", "-o", out, wav]), c("ffmpeg", &["-y", "-loglevel", "error", "-i", wav, out])],
        ExportFormat::Mp3 => vec![
            c("lame", &["--silent", "-V2", wav, out]),
            c("ffmpeg", &["-y", "-loglevel", "error", "-i", wav, "-codec:a", "libmp3lame", "-q:a", "2", out]),
        ],
    }
}

fn target(wav: &str, fmt: ExportFormat) -> String {
    Path::new(wav).with_extension(fmt.ext()).display().to_string()
}

/// Try each candidate; on the first that succeeds and produces the file, delete the WAV and return the new path.
pub fn convert_with(wav: &str, fmt: ExportFormat, cands: Vec<Candidate>) -> Result<String, String> {
    let out = target(wav, fmt);
    let mut last = format!("no {} encoder found", fmt.ext().to_uppercase());
    for (prog, args) in cands {
        match Command::new(&prog).args(&args).stdout(Stdio::null()).stderr(Stdio::null()).status() {
            Ok(st) if st.success() && Path::new(&out).exists() => {
                let _ = std::fs::remove_file(wav);
                return Ok(out);
            }
            Ok(_) => last = format!("{prog} failed"),
            Err(_) => {} // not installed: try the next one
        }
    }
    Err(format!("{last}: kept the WAV"))
}

/// The real thing: the encoders on this device.
pub fn convert(wav: &str, fmt: ExportFormat) -> Result<String, String> {
    let out = target(wav, fmt);
    convert_with(wav, fmt, candidates(fmt, wav, &out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_name_the_right_programs_and_files() {
        let c = candidates(ExportFormat::Flac, "/x/song.wav", "/x/song.flac");
        assert_eq!(c[0].0, "flac");
        assert!(c[0].1.contains(&"/x/song.wav".to_string()) && c[0].1.contains(&"/x/song.flac".to_string()) && c[0].1.contains(&"--force".to_string()));
        assert_eq!(c[1].0, "ffmpeg");
        let m = candidates(ExportFormat::Mp3, "/x/song.wav", "/x/song.mp3");
        assert_eq!((m[0].0.as_str(), m[1].0.as_str()), ("lame", "ffmpeg"));
        assert!(m[1].1.contains(&"libmp3lame".to_string()));
        assert!(candidates(ExportFormat::Wav, "a", "b").is_empty());
        assert_eq!(target("/x/song-stem-1.wav", ExportFormat::Mp3), "/x/song-stem-1.mp3");
    }

    #[test]
    fn a_working_encoder_replaces_the_wav_and_a_missing_one_keeps_it() {
        let d = std::env::temp_dir().join(format!("hilldaw-enc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let wav = d.join("a.wav").display().to_string();
        let out = d.join("a.flac").display().to_string();
        std::fs::write(&wav, b"RIFFfake").unwrap();
        // no such programs: an error, and the WAV is untouched
        let none = vec![("definitely-not-an-encoder-xyz".to_string(), vec![wav.clone()])];
        let e = convert_with(&wav, ExportFormat::Flac, none).unwrap_err();
        assert!(e.contains("no FLAC encoder") && e.contains("kept the WAV"), "{e}");
        assert!(Path::new(&wav).exists());
        // `cp` stands in for an encoder that works
        let ok = vec![("definitely-not-an-encoder-xyz".to_string(), vec![]), ("cp".to_string(), vec![wav.clone(), out.clone()])];
        assert_eq!(convert_with(&wav, ExportFormat::Flac, ok).unwrap(), out);
        assert!(!Path::new(&wav).exists() && Path::new(&out).exists(), "the WAV is removed after a successful conversion");
        // an encoder that fails (`false`) is reported by name and keeps the WAV
        std::fs::write(&wav, b"RIFFfake").unwrap();
        let bad = vec![("false".to_string(), vec![])];
        let e = convert_with(&wav, ExportFormat::Mp3, bad).unwrap_err();
        assert!(e.contains("false failed") && Path::new(&wav).exists(), "{e}");
    }
}
