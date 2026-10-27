use super::FindingGroup;
use super::findings::is_audio;
use super::walk::Tree;
use crate::cancel::CancelToken;
use crate::cli::{Cli, MusicMode};
use crate::scan::FileEntry;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub(super) fn music_groups(
    cli: &Cli,
    tree: &Tree,
    notes: &mut Vec<String>,
    cancel: &CancelToken,
) -> Vec<FindingGroup> {
    let tracks: Vec<&FileEntry> = tree.files.iter().filter(|f| is_audio(&f.path)).collect();
    if tracks.is_empty() {
        return Vec::new();
    }
    if cli.music_by == MusicMode::Content {
        return crate::audio::music_groups(&tracks, cli.similarity / 100.0, notes, cancel);
    }
    let mut map: BTreeMap<String, (TrackTags, Vec<PathBuf>, u64)> = BTreeMap::new();
    for track in tracks {
        if cancel.is_cancelled() {
            break;
        }
        let Some(tags) = read_tags(&track.path) else {
            continue;
        };
        let title = normalize_tag(&tags.title);
        if title.is_empty() {
            continue;
        }
        let artist = normalize_tag(&tags.artist);
        let album = normalize_tag(&tags.album);
        let key = format!("{title}\u{1f}{artist}\u{1f}{album}");
        let entry = map
            .entry(key)
            .or_insert_with(|| (tags.clone(), Vec::new(), 0));
        entry.1.push(track.path.clone());
        entry.2 += track.size;
    }
    let mut out = Vec::new();
    for (_, (tags, members, bytes)) in map {
        if members.len() < 2 {
            continue;
        }
        let mut members_sized: Vec<(PathBuf, u64)> = members
            .into_iter()
            .map(|p| {
                let size = fs::metadata(&p).map_or(0, |m| m.len());
                (p, size)
            })
            .collect();
        members_sized.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let sizes: Vec<u64> = members_sized.iter().map(|(_, s)| *s).collect();
        let largest = sizes.first().copied().unwrap_or(0);
        let members_sorted: Vec<PathBuf> = members_sized.into_iter().map(|(p, _)| p).collect();
        let label = if tags.artist.trim().is_empty() {
            tags.title.trim().to_string()
        } else {
            format!("{} — {}", tags.artist.trim(), tags.title.trim())
        };
        out.push(FindingGroup {
            label,
            detail: if tags.album.trim().is_empty() {
                format!("{} tracks, same tags", members_sorted.len())
            } else {
                format!(
                    "{} tracks, album: {}",
                    members_sorted.len(),
                    tags.album.trim()
                )
            },
            reclaimable: bytes.saturating_sub(largest),
            sizes,
            bytes,
            members: members_sorted,
        });
    }
    out.sort_by_key(|g| std::cmp::Reverse(g.reclaimable));
    out
}

#[derive(Debug, Clone, Default)]

struct TrackTags {
    title: String,
    artist: String,
    album: String,
}

pub(super) fn normalize_tag(s: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
            space = false;
        } else if !space {
            out.push(' ');
            space = true;
        }
    }
    out.trim().to_string()
}

fn read_tags(path: &Path) -> Option<TrackTags> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "mp3" => read_mp3_tags(path),
        "flac" => read_flac_tags(path),
        "m4a" | "mp4" | "m4b" | "aac" => read_mp4_tags(path),
        "wav" => read_wav_tags(path),
        _ => None,
    }
    .filter(|t| !t.title.trim().is_empty())
}

fn synchsafe(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .fold(0usize, |acc, &b| (acc << 7) | (b & 0x7f) as usize)
}

fn read_mp3_tags(path: &Path) -> Option<TrackTags> {
    let mut file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let mut header = [0u8; 10];
    if file.read_exact(&mut header).is_err() {
        return None;
    }
    if &header[0..3] == b"ID3" {
        let version = header[3];
        let flags = header[5];
        let tag_size = synchsafe(&header[6..10]).min(32 * 1024 * 1024);
        let mut data = vec![0u8; tag_size];
        file.read_exact(&mut data).ok()?;
        if flags & 0x80 != 0 {
            data = deunsync(&data);
        }
        let mut tags = TrackTags::default();
        let mut pos = 0usize;
        while pos + 10 <= data.len() {
            let id = &data[pos..pos + 4];
            if id.iter().all(|&b| b == 0) {
                break;
            }
            let frame_size = if version >= 4 {
                synchsafe(&data[pos + 4..pos + 8])
            } else {
                u32::from_be_bytes(data[pos + 4..pos + 8].try_into().ok()?) as usize
            };
            let body_start = pos + 10;
            if frame_size == 0 || body_start + frame_size > data.len() {
                break;
            }
            let body = &data[body_start..body_start + frame_size];
            match id {
                b"TIT2" => tags.title = decode_id3_text(body),
                b"TPE1" => tags.artist = decode_id3_text(body),
                b"TALB" => tags.album = decode_id3_text(body),
                _ => {}
            }
            pos = body_start + frame_size;
        }
        if !tags.title.trim().is_empty() {
            return Some(tags);
        }
    }
    if size >= 128 {
        file.seek(SeekFrom::End(-128)).ok()?;
        let mut tail = [0u8; 128];
        file.read_exact(&mut tail).ok()?;
        if &tail[0..3] == b"TAG" {
            let text = |s: &[u8]| {
                String::from_utf8_lossy(s)
                    .trim_matches('\0')
                    .trim()
                    .to_string()
            };
            let tags = TrackTags {
                title: text(&tail[3..33]),
                artist: text(&tail[33..63]),
                album: text(&tail[63..93]),
            };
            if !tags.title.is_empty() {
                return Some(tags);
            }
        }
    }
    None
}

fn deunsync(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        out.push(data[i]);
        if data[i] == 0xFF && i + 1 < data.len() && data[i + 1] == 0x00 {
            i += 1;
        }
        i += 1;
    }
    out
}

fn decode_id3_text(body: &[u8]) -> String {
    let Some((&encoding, text)) = body.split_first() else {
        return String::new();
    };
    let trim = |s: String| s.trim_matches('\0').trim().to_string();
    match encoding {
        0 => trim(text.iter().map(|&b| b as char).collect()),
        1 => {
            let s = if text.len() >= 2 {
                let le = text[0] == 0xFF && text[1] == 0xFE;
                let be = text[0] == 0xFE && text[1] == 0xFF;
                let start = if le || be { 2 } else { 0 };
                let mut units = Vec::new();
                let mut i = start;
                while i + 1 < text.len() {
                    let u = if le {
                        u16::from_le_bytes([text[i], text[i + 1]])
                    } else {
                        u16::from_be_bytes([text[i], text[i + 1]])
                    };
                    units.push(u);
                    i += 2;
                }
                String::from_utf16_lossy(&units)
            } else {
                String::new()
            };
            trim(s)
        }
        2 => {
            let mut units = Vec::new();
            let mut i = 0;
            while i + 1 < text.len() {
                units.push(u16::from_be_bytes([text[i], text[i + 1]]));
                i += 2;
            }
            trim(String::from_utf16_lossy(&units))
        }
        _ => trim(String::from_utf8_lossy(text).into_owned()),
    }
}

fn read_flac_tags(path: &Path) -> Option<TrackTags> {
    let mut file = File::open(path).ok()?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).ok()?;
    if &magic != b"fLaC" {
        return None;
    }
    let mut tags = TrackTags::default();
    loop {
        let mut header = [0u8; 4];
        if file.read_exact(&mut header).is_err() {
            break;
        }
        let last = header[0] & 0x80 != 0;
        let kind = header[0] & 0x7f;
        let len = ((header[1] as usize) << 16) | ((header[2] as usize) << 8) | header[3] as usize;
        if kind == 4 {
            let mut data = vec![0u8; len];
            file.read_exact(&mut data).ok()?;
            let mut pos = 0usize;
            if data.len() < 8 {
                break;
            }
            let vendor = u32::from_le_bytes(data[pos..pos + 4].try_into().ok()?) as usize;
            pos += 4 + vendor;
            if pos + 4 > data.len() {
                break;
            }
            let count = u32::from_le_bytes(data[pos..pos + 4].try_into().ok()?) as usize;
            pos += 4;
            for _ in 0..count {
                if pos + 4 > data.len() {
                    break;
                }
                let clen = u32::from_le_bytes(data[pos..pos + 4].try_into().ok()?) as usize;
                pos += 4;
                if pos + clen > data.len() {
                    break;
                }
                let comment = String::from_utf8_lossy(&data[pos..pos + clen]);
                pos += clen;
                if let Some((key, value)) = comment.split_once('=') {
                    match key.to_ascii_uppercase().as_str() {
                        "TITLE" => tags.title = value.to_string(),
                        "ARTIST" => tags.artist = value.to_string(),
                        "ALBUM" => tags.album = value.to_string(),
                        _ => {}
                    }
                }
            }
        } else {
            file.seek(SeekFrom::Current(len as i64)).ok()?;
        }
        if last {
            break;
        }
    }
    Some(tags)
}

fn read_wav_tags(path: &Path) -> Option<TrackTags> {
    let mut file = File::open(path).ok()?;
    let mut riff = [0u8; 12];
    file.read_exact(&mut riff).ok()?;
    if &riff[0..4] != b"RIFF" || &riff[8..12] != b"WAVE" {
        return None;
    }
    let mut tags = TrackTags::default();
    loop {
        let mut header = [0u8; 8];
        if file.read_exact(&mut header).is_err() {
            break;
        }
        let len = u32::from_le_bytes(header[4..8].try_into().ok()?) as usize;
        if &header[0..4] == b"LIST" {
            let mut data = vec![0u8; len.min(1024 * 1024)];
            file.read_exact(&mut data).ok()?;
            let mut pos = 0usize;
            while pos + 8 <= data.len() {
                let id = &data[pos..pos + 4];
                let clen = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().ok()?) as usize;
                pos += 8;
                if pos + clen > data.len() {
                    break;
                }
                let value = String::from_utf8_lossy(&data[pos..pos + clen])
                    .trim_matches('\0')
                    .trim()
                    .to_string();
                match id {
                    b"INAM" => tags.title = value,
                    b"IART" => tags.artist = value,
                    b"IPRD" => tags.album = value,
                    _ => {}
                }
                pos += clen + (clen & 1);
            }
        } else {
            file.seek(SeekFrom::Current(len as i64 + (len as i64 & 1)))
                .ok()?;
        }
    }
    Some(tags)
}

fn read_mp4_tags(path: &Path) -> Option<TrackTags> {
    let mut file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let ilst = find_mp4_atom(&mut file, 0, size, b"ilst")?;
    read_ilst_tags(&mut file, ilst.0, ilst.1)
}

fn find_mp4_atom(file: &mut File, start: u64, end: u64, target: &[u8; 4]) -> Option<(u64, u64)> {
    let mut pos = start;
    while pos + 8 <= end {
        file.seek(SeekFrom::Start(pos)).ok()?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header).ok()?;
        let mut atom_size = u64::from(u32::from_be_bytes(header[0..4].try_into().ok()?));
        let kind = &header[4..8];
        let header_len = if atom_size == 1 {
            let mut ext = [0u8; 8];
            file.read_exact(&mut ext).ok()?;
            atom_size = u64::from_be_bytes(ext);
            16
        } else {
            8
        };
        if atom_size == 0 {
            atom_size = end - pos;
        }
        if atom_size < header_len || pos + atom_size > end {
            return None;
        }
        if kind == target {
            return Some((pos + header_len, pos + atom_size));
        }
        if (kind == b"moov"
            || kind == b"udta"
            || kind == b"trak"
            || kind == b"mdia"
            || kind == b"minf"
            || kind == b"stbl")
            && let Some(found) = find_mp4_atom(file, pos + header_len, pos + atom_size, target)
        {
            return Some(found);
        }
        if kind == b"meta"
            && let Some(found) = find_mp4_atom(file, pos + header_len + 4, pos + atom_size, target)
        {
            return Some(found);
        }
        pos += atom_size;
    }
    None
}

fn read_ilst_tags(file: &mut File, start: u64, end: u64) -> Option<TrackTags> {
    let mut tags = TrackTags::default();
    let mut pos = start;
    while pos + 8 <= end {
        file.seek(SeekFrom::Start(pos)).ok()?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header).ok()?;
        let atom_size = u64::from(u32::from_be_bytes(header[0..4].try_into().ok()?));
        let kind = &header[4..8];
        if atom_size < 8 || pos + atom_size > end {
            break;
        }
        if kind == b"\xa9nam" || kind == b"\xa9ART" || kind == b"\xa9alb" {
            file.seek(SeekFrom::Start(pos + 8)).ok()?;
            let mut data_header = [0u8; 16];
            if file.read_exact(&mut data_header).is_ok() && &data_header[4..8] == b"data" {
                let data_size = u64::from(u32::from_be_bytes(data_header[0..4].try_into().ok()?));
                if data_size >= 16 && data_size <= atom_size {
                    let mut payload = vec![0u8; (data_size - 16) as usize];
                    file.read_exact(&mut payload).ok()?;
                    let value = String::from_utf8_lossy(&payload).trim().to_string();
                    if kind == b"\xa9nam" {
                        tags.title = value;
                    } else if kind == b"\xa9ART" {
                        tags.artist = value;
                    } else {
                        tags.album = value;
                    }
                }
            }
        }
        pos += atom_size;
    }
    Some(tags)
}
