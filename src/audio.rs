use crate::cancel::CancelToken;
use crate::modes::FindingGroup;
use crate::scan::FileEntry;
use std::io::Read;
use std::path::Path;
use std::process::Stdio;

const SAMPLE_RATE: u32 = 8_000;
const FRAME_SAMPLES: usize = 4_000;
const BANDS: [f64; 8] = [180.0, 250.0, 350.0, 500.0, 700.0, 1_000.0, 1_500.0, 2_200.0];

#[derive(Debug, Clone)]
pub struct AudioFp {
    pub bits: Vec<u8>,
}

impl AudioFp {
    pub fn frames(&self) -> usize {
        self.bits.len()
    }
}

pub fn fingerprint(path: &Path) -> Option<AudioFp> {
    let mut child = crate::util::quiet_command("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-vn", "-ac", "1", "-ar", "8000", "-t", "900", "-f", "s16le", "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let mut data = Vec::new();
    stdout.read_to_end(&mut data).ok()?;
    let _ = child.wait();
    if data.len() < FRAME_SAMPLES * 2 / 2 {
        return None;
    }
    let samples: Vec<f64> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| i16::from_le_bytes(*c) as f64 / 32768.0)
        .collect();
    let mut bits = Vec::with_capacity(samples.len() / FRAME_SAMPLES + 1);
    for chunk in samples.chunks(FRAME_SAMPLES) {
        if chunk.len() < FRAME_SAMPLES / 2 {
            break;
        }
        bits.push(frame_bits(chunk));
    }
    if bits.is_empty() {
        None
    } else {
        Some(AudioFp { bits })
    }
}

fn frame_bits(samples: &[f64]) -> u8 {
    let mut energies = [0.0f64; BANDS.len()];
    for (i, &freq) in BANDS.iter().enumerate() {
        energies[i] = goertzel(samples, freq);
    }
    let mean = energies.iter().sum::<f64>() / energies.len() as f64;
    let mut bits = 0u8;
    for (i, &e) in energies.iter().enumerate() {
        if e > mean {
            bits |= 1 << i;
        }
    }
    bits
}

fn goertzel(samples: &[f64], freq: f64) -> f64 {
    let k = (0.5 + samples.len() as f64 * freq / SAMPLE_RATE as f64).floor();
    let omega = 2.0 * std::f64::consts::PI * k / samples.len() as f64;
    let coeff = 2.0 * omega.cos();
    let mut s0;
    let mut s1 = 0.0;
    let mut s2 = 0.0;
    for &x in samples {
        s0 = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0)
}

/// Best frame-agreement score over every alignment of the shorter fingerprint
/// inside the longer one, so partial clips and delayed intros still match.
pub fn similarity(a: &AudioFp, b: &AudioFp) -> f64 {
    if a.bits.is_empty() || b.bits.is_empty() {
        return 0.0;
    }
    let (short, long) = if a.bits.len() <= b.bits.len() {
        (&a.bits, &b.bits)
    } else {
        (&b.bits, &a.bits)
    };
    let max_off = long.len() - short.len();
    let mut best = 0.0f64;
    for off in 0..=max_off {
        let same: u32 = short
            .iter()
            .zip(&long[off..off + short.len()])
            .map(|(x, y)| (x ^ y).count_ones())
            .sum();
        let total = (short.len() * 8) as f64;
        let score = 1.0 - same as f64 / total;
        if score > best {
            best = score;
        }
    }
    best
}

pub fn music_groups(
    tracks: &[&FileEntry],
    threshold: f64,
    notes: &mut Vec<String>,
    cancel: &CancelToken,
) -> Vec<FindingGroup> {
    let mut prints: Vec<(AudioFp, &FileEntry)> = Vec::new();
    for track in tracks {
        if cancel.is_cancelled() {
            break;
        }
        match fingerprint(&track.path) {
            Some(fp) => prints.push((fp, track)),
            None => notes.push(format!(
                "cannot fingerprint audio: {} (is ffmpeg installed?)",
                track.path.display()
            )),
        }
    }
    prints.sort_by_key(|(fp, _)| fp.frames());
    let mut clusters: Vec<(AudioFp, Vec<&FileEntry>, u64)> = Vec::new();
    for (fp, entry) in prints {
        let mut matched = None;
        for (i, (rep, _, _)) in clusters.iter().enumerate() {
            let ratio =
                fp.frames().min(rep.frames()) as f64 / fp.frames().max(rep.frames()).max(1) as f64;
            if ratio < 0.35 {
                continue;
            }
            if similarity(&fp, rep) >= threshold {
                matched = Some(i);
                break;
            }
        }
        match matched {
            Some(i) => {
                clusters[i].1.push(entry);
                clusters[i].2 += entry.size;
            }
            None => clusters.push((fp, vec![entry], entry.size)),
        }
    }
    let mut out = Vec::new();
    for (_, members, bytes) in clusters {
        if members.len() < 2 {
            continue;
        }
        let mut members: Vec<&FileEntry> = members;
        members.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
        let sizes: Vec<u64> = members.iter().map(|m| m.size).collect();
        let largest = sizes.first().copied().unwrap_or(0);
        let paths: Vec<_> = members.iter().map(|m| m.path.clone()).collect();
        let label = paths
            .first()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Audio match".into());
        out.push(FindingGroup {
            label,
            detail: format!(
                "{} tracks, audio fingerprint ≥ {:.0}%",
                paths.len(),
                threshold * 100.0
            ),
            reclaimable: bytes.saturating_sub(largest),
            sizes,
            bytes,
            members: paths,
        });
    }
    out.sort_by_key(|b| std::cmp::Reverse(b.reclaimable));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(bits: Vec<u8>) -> AudioFp {
        AudioFp { bits }
    }

    #[test]
    fn identical_fingerprints_score_one() {
        let a = fp(vec![1, 2, 3, 4]);
        assert!((similarity(&a, &a) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn partial_clip_matches_inside_a_longer_track() {
        let short = fp(vec![9, 7, 1]);
        let long = fp(vec![0, 0, 9, 7, 1, 0]);
        assert!((similarity(&short, &long) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn unrelated_fingerprints_score_low() {
        let a = fp(vec![0x00, 0x00]);
        let b = fp(vec![0xFF, 0xFF]);
        assert!(similarity(&a, &b) < 0.1);
    }
}
