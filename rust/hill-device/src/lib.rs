//! SDL2 runner. One binary per app; each calls [`run`].

mod buttons;
mod host;

use buttons::{hat_buttons, Map};
use hill_core::fb::{self, Fb};
use hill_core::{apps, App, Btn, SAMPLE_RATE};
use host::{exe_dir, FsHost};
use sdl2::audio::{AudioCallback, AudioSpecDesired};
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::pixels::PixelFormatEnum;
use sdl2::video::Window;
use sdl2::render::Canvas;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Audio {
    app: Arc<Mutex<Box<dyn App>>>,
}

impl AudioCallback for Audio {
    type Channel = f32;
    fn callback(&mut self, out: &mut [f32]) {
        match self.app.lock() {
            Ok(mut app) => app.audio(out),
            Err(_) => out.iter_mut().for_each(|x| *x = 0.0),
        }
    }
}

fn open_canvas(sdl: &sdl2::Sdl, title: &str) -> Result<Canvas<Window>, String> {
    let video = sdl.video()?;
    let mut wb = video.window(title, 640, 480);
    wb.position_centered();
    if std::env::var("HILL_FULLSCREEN").is_ok() {
        wb.fullscreen_desktop();
    }
    let window = wb.build().map_err(|e| e.to_string())?;
    let mut canvas = match window.into_canvas().build() {
        Ok(c) => c,
        Err(_) => {
            // accelerated renderer unavailable: retry with software
            let window = video.window(title, 640, 480).position_centered().build().map_err(|e| e.to_string())?;
            window.into_canvas().software().build().map_err(|e| e.to_string())?
        }
    };
    canvas.set_logical_size(640, 480).map_err(|e| e.to_string())?;
    sdl.mouse().show_cursor(false);
    Ok(canvas)
}

fn present(canvas: &mut Canvas<Window>, tex: &mut sdl2::render::Texture, fb: &Fb) -> Result<(), String> {
    let bytes = unsafe { std::slice::from_raw_parts(fb.px.as_ptr() as *const u8, fb.px.len() * 4) };
    tex.update(None, bytes, 640 * 4).map_err(|e| e.to_string())?;
    canvas.clear();
    canvas.copy(tex, None, None)?;
    canvas.present();
    Ok(())
}

pub fn run(name: &str) -> Result<(), String> {
    let sdl = sdl2::init()?;
    let audio_sub = sdl.audio()?;
    let joy_sub = sdl.joystick()?;
    let mut canvas = open_canvas(&sdl, name)?;
    let creator = canvas.texture_creator();
    let mut tex = creator.create_texture_streaming(PixelFormatEnum::RGB888, 640, 480).map_err(|e| e.to_string())?;

    let host = FsHost::detect();
    let have_samples = host.sample_root.is_some();
    let app = apps::create(name, Box::new(host)).ok_or("unknown app")?;
    let app = Arc::new(Mutex::new(app));

    let desired = AudioSpecDesired { freq: Some(SAMPLE_RATE as i32), channels: Some(2), samples: Some(1024) };
    let device = audio_sub.open_playback(None, &desired, |spec| {
        if let Ok(mut a) = app.lock() {
            a.set_rate(spec.freq as f32);
        }
        Audio { app: app.clone() }
    })?;
    device.resume();

    let mut joysticks = Vec::new();
    for i in 0..joy_sub.num_joysticks().unwrap_or(0) {
        if let Ok(j) = joy_sub.open(i) {
            joysticks.push(j);
        }
    }
    let map = Map::load(&exe_dir());
    let mut pump = sdl.event_pump()?;
    let mut fb = Fb::new();
    let mut hat = [false; 4];
    let mut stick = [false; 4];
    let mut last = Instant::now();
    let mut since_save = 0u32;
    if !have_samples {
        eprintln!("{}: no sample folder found (set HILL_SAMPLES); using built-in sounds", name);
    }

    'main: loop {
        for ev in pump.poll_iter() {
            let mut out: Vec<(Btn, bool)> = Vec::new();
            match ev {
                Event::Quit { .. } => break 'main,
                Event::KeyDown { keycode: Some(Keycode::Escape), .. } => break 'main,
                Event::KeyDown { keycode: Some(k), repeat: false, .. } => {
                    if let Some(b) = buttons::key(k) {
                        out.push((b, true));
                    }
                }
                Event::KeyUp { keycode: Some(k), .. } => {
                    if let Some(b) = buttons::key(k) {
                        out.push((b, false));
                    }
                }
                Event::JoyButtonDown { button_idx, .. } => {
                    if let Some(b) = map.joy.get(&button_idx) {
                        out.push((*b, true));
                    }
                }
                Event::JoyButtonUp { button_idx, .. } => {
                    if let Some(b) = map.joy.get(&button_idx) {
                        out.push((*b, false));
                    }
                }
                Event::JoyHatMotion { state, .. } => {
                    let now = hat_buttons(state.to_raw());
                    for (i, b) in [Btn::Up, Btn::Down, Btn::Left, Btn::Right].into_iter().enumerate() {
                        if now[i] != hat[i] {
                            out.push((b, now[i]));
                        }
                    }
                    hat = now;
                }
                Event::JoyAxisMotion { axis_idx, value, .. } if map.stick && axis_idx < 2 => {
                    // left stick as an extra D-pad
                    let (neg, pos) = if axis_idx == 0 { (2, 3) } else { (0, 1) };
                    let n = value < -16000;
                    let p = value > 16000;
                    let pair = [(neg, n, [Btn::Up, Btn::Down, Btn::Left, Btn::Right][neg]), (pos, p, [Btn::Up, Btn::Down, Btn::Left, Btn::Right][pos])];
                    for (i, now, b) in pair {
                        if stick[i] != now {
                            stick[i] = now;
                            out.push((b, now));
                        }
                    }
                }
                _ => {}
            }
            if !out.is_empty() {
                if let Ok(mut a) = app.lock() {
                    for (b, down) in out {
                        a.event(b, down);
                    }
                }
            }
        }

        let now = Instant::now();
        let dt = now.duration_since(last).as_millis().min(100) as u32;
        last = now;
        since_save += dt;
        let quit = {
            let mut a = app.lock().map_err(|_| "poisoned")?;
            if since_save >= 30_000 {
                since_save = 0;
                a.save(); // autosave, in case the console is powered off
            }
            a.tick(dt);
            a.render(&mut fb);
            a.wants_quit()
        };
        present(&mut canvas, &mut tex, &fb)?;
        if quit {
            break;
        }
        let spent = now.elapsed();
        if spent < Duration::from_millis(16) {
            std::thread::sleep(Duration::from_millis(16) - spent);
        }
    }
    device.pause();
    if let Ok(mut a) = app.lock() {
        a.save();
    }
    Ok(())
}

/// Interactive wizard that writes `buttons.cfg` next to the executable.
pub fn mapping_wizard() -> Result<(), String> {
    let sdl = sdl2::init()?;
    let joy_sub = sdl.joystick()?;
    let mut canvas = open_canvas(&sdl, "hillmap")?;
    let creator = canvas.texture_creator();
    let mut tex = creator.create_texture_streaming(PixelFormatEnum::RGB888, 640, 480).map_err(|e| e.to_string())?;
    let mut joysticks = Vec::new();
    for i in 0..joy_sub.num_joysticks().unwrap_or(0) {
        if let Ok(j) = joy_sub.open(i) {
            joysticks.push(j);
        }
    }
    let names: Vec<String> = joysticks.iter().map(|j| j.name()).collect();

    // (label, logical, optional)
    let steps: Vec<(&str, Btn, bool)> = vec![
        ("A", Btn::A, false),
        ("B", Btn::B, false),
        ("X", Btn::X, false),
        ("Y", Btn::Y, false),
        ("L1", Btn::L1, false),
        ("R1", Btn::R1, false),
        ("L2", Btn::L2, false),
        ("R2", Btn::R2, false),
        ("SELECT", Btn::Select, false),
        ("START", Btn::Start, false),
        ("FUNCTION", Btn::Fn, false),
        ("L3 (stick click)", Btn::L3, true),
        ("R3 (stick click)", Btn::R3, true),
        ("D-pad UP", Btn::Up, false),
        ("D-pad DOWN", Btn::Down, false),
        ("D-pad LEFT", Btn::Left, false),
        ("D-pad RIGHT", Btn::Right, false),
    ];
    let mut done: Vec<(Btn, u8)> = Vec::new();
    let mut step = 0usize;
    let mut dpad_is_hat = false;
    let mut start_idx: Option<u8> = None;
    let mut fbuf = Fb::new();
    let mut pump = sdl.event_pump()?;
    let mut finished = false;
    let mut stick_ask = false;
    let mut stick = false;
    let mut last_seen = String::from("-");
    let path = std::env::var("HILL_BUTTONS").map(std::path::PathBuf::from).unwrap_or_else(|_| exe_dir().join("buttons.cfg"));

    'main: loop {
        for ev in pump.poll_iter() {
            match ev {
                Event::Quit { .. } | Event::KeyDown { keycode: Some(Keycode::Escape), .. } => break 'main,
                Event::JoyButtonDown { button_idx, .. } => {
                    last_seen = format!("button {}", button_idx);
                    if finished {
                        break 'main; // any button closes the "Saved" screen
                    } else if stick_ask {
                        // A (just mapped) = yes, anything else = no
                        let a = done.iter().find(|(b, _)| *b == Btn::A).map(|(_, i)| *i);
                        stick = Some(button_idx) == a;
                        std::fs::write(&path, Map::render(&done, stick)).map_err(|e| e.to_string())?;
                        finished = true;
                        stick_ask = false;
                    } else if !finished && step < steps.len() {
                        let (_, logical, optional) = steps[step];
                        let used = done.iter().any(|(_, i)| *i == button_idx);
                        if optional && Some(button_idx) == start_idx {
                            step += 1; // START skips optional buttons
                        } else if !used {
                            done.push((logical, button_idx));
                            if logical == Btn::Start {
                                start_idx = Some(button_idx);
                            }
                            step += 1;
                        }
                    }
                }
                Event::JoyHatMotion { state, .. } => {
                    last_seen = format!("hat {}", state.to_raw());
                    if !finished && !stick_ask && step < steps.len() && steps[step].1 == Btn::Up && state.to_raw() != 0 {
                        dpad_is_hat = true;
                        step = steps.len();
                    }
                }
                _ => {}
            }
            if step >= steps.len() && !finished && !stick_ask {
                stick_ask = true;
            }
        }

        fbuf.clear(fb::BG);
        fbuf.text(20, 20, "HillMap: button setup", 3, fb::ACCENT);
        let dev = if names.is_empty() { "NO JOYSTICK FOUND".to_string() } else { names.join(", ") };
        fbuf.text(20, 64, &fb::clip(&dev, 50), 1, fb::DIM);
        fbuf.text(20, 84, &format!("last input: {}", last_seen), 1, fb::DIM);
        if finished {
            fbuf.text(20, 160, "Saved buttons.cfg", 3, fb::WHITE);
            fbuf.text(20, 220, &fb::clip(&path.to_string_lossy(), 50), 1, fb::DIM);
            fbuf.text(20, 260, &format!("D-pad: {}   left stick as D-pad: {}", if dpad_is_hat { "hat" } else { "buttons" }, if stick { "yes" } else { "no" }), 1, fb::TEXT);
            fbuf.text(20, 300, "Press any button to close.", 1, fb::DIM);
        } else if stick_ask {
            fbuf.text(20, 160, "Use the left stick as a", 3, fb::WHITE);
            fbuf.text(20, 200, "second D-pad?", 3, fb::WHITE);
            fbuf.text(20, 260, "Press A for yes, any other button for no", 2, fb::DIM);
        } else if step < steps.len() {
            let (label, _, optional) = steps[step];
            fbuf.text(20, 150, "Press", 3, fb::DIM);
            fbuf.text(20, 190, label, 4, fb::WHITE);
            if optional {
                fbuf.text(20, 260, "No such button? press START to skip", 2, fb::DIM);
            } else if label.starts_with("D-pad") {
                fbuf.text(20, 260, "(if the D-pad is a hat, press UP once)", 2, fb::DIM);
            }
            fbuf.text(20, 440, &format!("step {}/{}", step + 1, steps.len()), 1, fb::DIM);
        }
        present(&mut canvas, &mut tex, &fbuf)?;
        std::thread::sleep(Duration::from_millis(16));
    }
    Ok(())
}
