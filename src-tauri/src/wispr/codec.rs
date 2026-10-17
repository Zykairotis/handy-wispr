//! Converts Handy's 16 kHz mono `f32` capture into the frames the Wispr
//! voice-actions socket expects: 24 kHz mono PCM16 little-endian, base64.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

/// Raw bytes per `audio.append`. The Wispr app slices at 600 000 bytes; staying
/// far below keeps every frame valid under either reading of that limit.
pub const MAX_FRAME_BYTES: usize = 96_000;

/// Linear 16 kHz -> 24 kHz (exact 3:2), as the Wispr app does.
fn resample_16k_to_24k(samples: &[f32]) -> Vec<f32> {
    let Some(last) = samples.len().checked_sub(1) else {
        return Vec::new();
    };
    (0..samples.len() * 3 / 2)
        .map(|j| {
            let pos = j * 2;
            let (i, rem) = (pos / 3, (pos % 3) as f32 / 3.0);
            let a = samples[i];
            let b = samples[(i + 1).min(last)];
            a + (b - a) * rem
        })
        .collect()
}

fn to_pcm16_le(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|s| ((s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16).to_le_bytes())
        .collect()
}

/// Base64 frames of at most [`MAX_FRAME_BYTES`] raw bytes; empty input gives no frames.
pub fn encode_frames(samples_16k: &[f32]) -> Vec<String> {
    to_pcm16_le(&resample_16k_to_24k(samples_16k))
        .chunks(MAX_FRAME_BYTES)
        .map(|chunk| STANDARD.encode(chunk))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(frame: &str) -> Vec<u8> {
        STANDARD.decode(frame).unwrap()
    }

    #[test]
    fn resamples_three_to_two() {
        assert_eq!(resample_16k_to_24k(&[0.0; 160]).len(), 240);
        assert_eq!(resample_16k_to_24k(&[0.0; 3]).len(), 4);
        assert!(resample_16k_to_24k(&[]).is_empty());
    }

    #[test]
    fn resample_interpolates_linearly() {
        let out = resample_16k_to_24k(&[0.0, 0.3, 0.6]);
        let expected = [0.0, 0.2, 0.4, 0.6];
        for (got, want) in out.iter().zip(expected) {
            assert!((got - want).abs() < 1e-6, "{out:?}");
        }
    }

    #[test]
    fn quantizes_and_clamps_little_endian() {
        let bytes = to_pcm16_le(&[0.0, 1.0, -1.0, 2.0, -2.0]);
        let values: Vec<i16> = bytes
            .chunks(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(values, [0, 32767, -32767, 32767, -32767]);
    }

    #[test]
    fn empty_input_has_no_frames() {
        assert!(encode_frames(&[]).is_empty());
    }

    #[test]
    fn frames_are_bounded_and_lossless_in_length() {
        // 200 000 samples @16k -> 300 000 samples @24k -> 600 000 bytes.
        let frames = encode_frames(&vec![0.25; 200_000]);
        let sizes: Vec<usize> = frames.iter().map(|f| decode(f).len()).collect();
        assert!(sizes.iter().all(|&n| n <= MAX_FRAME_BYTES && n % 2 == 0));
        assert_eq!(sizes.iter().sum::<usize>(), 600_000);
        assert_eq!(frames.len(), 7);
    }
}
