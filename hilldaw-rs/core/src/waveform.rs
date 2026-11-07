//! Waveform overviews for drawing: a (min, max) pair for every `BUCKET` frames of a recording, computed once when the audio is loaded, and
//! a function that squeezes them into one pair per screen pixel at any zoom. Drawing then costs a scan of a few buckets per pixel instead
//! of the audio itself, however long the take is.

/// Frames per overview bucket (about 11.6 ms at 44.1 kHz).
pub const BUCKET: usize = 512;

/// The lowest and highest sample in each bucket of `data`.
pub fn peaks(data: &[f32]) -> Vec<(f32, f32)> {
    data.chunks(BUCKET)
        .map(|c| c.iter().fold((f32::MAX, f32::MIN), |(lo, hi), s| (lo.min(*s), hi.max(*s))))
        .collect()
}

/// For each of `px` pixels, the (min, max) of the audio between `from + i * frames_per_px` and the next pixel's start (file frames),
/// or `None` where that lies outside the recording. `frames_per_px` below one bucket still works (a pixel then shows its bucket).
pub fn envelope(peaks: &[(f32, f32)], from: i64, frames_per_px: f64, px: usize) -> Vec<Option<(f32, f32)>> {
    let total = peaks.len() as i64 * BUCKET as i64;
    (0..px)
        .map(|i| {
            let a = from as f64 + i as f64 * frames_per_px;
            let b = from as f64 + (i + 1) as f64 * frames_per_px;
            let (a, b) = (a.floor() as i64, (b.ceil() as i64).max(a.floor() as i64 + 1));
            if b <= 0 || a >= total {
                return None;
            }
            let (first, last) = ((a.max(0) / BUCKET as i64) as usize, ((b.min(total) - 1) / BUCKET as i64) as usize);
            Some(peaks[first..=last.min(peaks.len() - 1)].iter().fold((f32::MAX, f32::MIN), |(lo, hi), (l, h)| (lo.min(*l), hi.max(*h))))
        })
        .collect()
}

/// Screen x for a timeline frame, for a view that starts at `start` and shows `span` frames across `w` pixels from `x0`. `None` outside.
pub fn frame_to_x(frame: u64, start: u64, span: u64, x0: i32, w: i32) -> Option<i32> {
    if span == 0 || frame < start || frame >= start + span {
        return None;
    }
    Some(x0 + ((frame - start) as f64 / span as f64 * w as f64) as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_are_the_extremes_of_each_bucket_and_cover_a_short_last_bucket() {
        let mut d = vec![0.0f32; BUCKET * 2 + 100];
        d[10] = 0.5;
        d[20] = -0.25;
        d[BUCKET + 7] = 0.9;
        d[BUCKET * 2 + 50] = -0.8;
        let p = peaks(&d);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0], (-0.25, 0.5));
        assert_eq!(p[1], (0.0, 0.9));
        assert_eq!(p[2], (-0.8, 0.0), "the 100-frame tail is a bucket too");
        assert!(peaks(&[]).is_empty());
    }

    #[test]
    fn envelope_combines_buckets_per_pixel_and_reports_what_is_outside_the_audio() {
        // a ramp: the max grows with position, so each pixel's max says where in the file it looked
        let d: Vec<f32> = (0..BUCKET * 10).map(|i| i as f32 / (BUCKET * 10) as f32).collect();
        let p = peaks(&d);
        // 5 pixels over the whole file: 2 buckets each
        let e = envelope(&p, 0, (BUCKET * 2) as f64, 5);
        assert!(e.iter().all(|x| x.is_some()));
        let maxes: Vec<f32> = e.iter().map(|x| x.unwrap().1).collect();
        assert!(maxes.windows(2).all(|w| w[1] > w[0]), "{maxes:?}");
        assert!((maxes[4] - 1.0).abs() < 1e-3, "the last pixel reaches the end of the ramp");
        // looking before the start and past the end gives None, and a view that overlaps the start shows the start
        let e = envelope(&p, -(BUCKET as i64) * 4, (BUCKET * 2) as f64, 5);
        assert_eq!((e[0], e[1]), (None, None));
        assert!(e[2].is_some());
        let e = envelope(&p, (BUCKET * 9) as i64, (BUCKET * 2) as f64, 3);
        assert!(e[0].is_some() && e[1].is_none() && e[2].is_none());
        // zoomed far in (less than a bucket per pixel) still shows something
        let e = envelope(&p, 0, 10.0, 20);
        assert!(e.iter().all(|x| x.is_some()));
        assert!(envelope(&[], 0, 100.0, 4).iter().all(|x| x.is_none()));
    }

    #[test]
    fn a_loud_burst_shows_up_at_the_right_pixel_at_any_zoom() {
        let mut d = vec![0.0f32; BUCKET * 100];
        d[BUCKET * 60 + 3] = 0.9;
        let p = peaks(&d);
        for fpp in [BUCKET as f64 / 4.0, BUCKET as f64, BUCKET as f64 * 10.0] {
            let n = (BUCKET as f64 * 100.0 / fpp).ceil() as usize;
            let e = envelope(&p, 0, fpp, n);
            let hit = e.iter().position(|x| x.is_some_and(|(_, hi)| hi > 0.5)).unwrap();
            let expect = ((BUCKET * 60 + 3) as f64 / fpp) as usize;
            assert!(hit.abs_diff(expect) <= 1, "fpp {fpp}: pixel {hit} vs {expect}");
        }
    }

    #[test]
    fn frames_map_to_pixels_inside_the_view_only() {
        assert_eq!(frame_to_x(1000, 0, 2000, 60, 400), Some(60 + 200));
        assert_eq!(frame_to_x(0, 0, 2000, 60, 400), Some(60));
        assert_eq!(frame_to_x(1999, 0, 2000, 60, 400), Some(60 + 399));
        assert_eq!(frame_to_x(2000, 0, 2000, 60, 400), None, "the end is outside");
        assert_eq!(frame_to_x(50, 100, 2000, 60, 400), None, "before the start");
        assert_eq!(frame_to_x(5, 0, 0, 60, 400), None);
    }
}
