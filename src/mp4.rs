//! Small, bounded timing metadata reader. Windows' MP4 source can omit an
//! initial empty audio edit; decoding/encoding still belongs to Media Foundation.
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};
#[derive(Clone, Copy)]
struct Atom {
    kind: [u8; 4],
    body: u64,
    end: u64,
}
fn atoms(file: &mut File, start: u64, end: u64) -> Option<Vec<Atom>> {
    let mut p = start;
    let mut result = vec![];
    while p + 8 <= end {
        if result.len() > 100_000 {
            return None;
        }
        file.seek(SeekFrom::Start(p)).ok()?;
        let mut h = [0u8; 8];
        file.read_exact(&mut h).ok()?;
        let small = u32::from_be_bytes(h[..4].try_into().ok()?);
        let (size, head) = match small {
            0 => (end - p, 8),
            1 => {
                let mut large = [0u8; 8];
                file.read_exact(&mut large).ok()?;
                (u64::from_be_bytes(large), 16)
            }
            n => (n as u64, 8),
        };
        let next = p.checked_add(size)?;
        if size < head || next > end {
            return None;
        }
        result.push(Atom {
            kind: h[4..8].try_into().ok()?,
            body: p + head,
            end: next,
        });
        p = next;
    }
    Some(result)
}
fn child(f: &mut File, parent: Atom, name: &[u8; 4]) -> Option<Atom> {
    atoms(f, parent.body, parent.end)?
        .into_iter()
        .find(|a| &a.kind == name)
}
fn bytes(f: &mut File, a: Atom, max: usize) -> Option<Vec<u8>> {
    let len = (a.end - a.body) as usize;
    if len > max {
        return None;
    }
    let mut b = vec![0; len];
    f.seek(SeekFrom::Start(a.body)).ok()?;
    f.read_exact(&mut b).ok()?;
    Some(b)
}
fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], i: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(i..i + 8)?.try_into().ok()?))
}
pub fn audio_delay(path: &Path) -> f64 {
    if !path.extension().and_then(|p| p.to_str()).is_some_and(|e| {
        ["mp4", "mov", "m4v", "m4a"]
            .iter()
            .any(|known| e.eq_ignore_ascii_case(known))
    }) {
        return 0.0;
    }
    parse(path).unwrap_or(0.0)
}
fn parse(path: &Path) -> Option<f64> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let moov = atoms(&mut f, 0, len)?
        .into_iter()
        .find(|a| a.kind == *b"moov")?;
    let mvhd = child(&mut f, moov, b"mvhd")?;
    let b = bytes(&mut f, mvhd, 4096)?;
    let scale = u32_at(&b, if b[0] == 1 { 20 } else { 12 })? as f64;
    if scale == 0.0 {
        return None;
    }
    for track in atoms(&mut f, moov.body, moov.end)?
        .into_iter()
        .filter(|a| a.kind == *b"trak")
    {
        let Some(mdia) = child(&mut f, track, b"mdia") else {
            continue;
        };
        let Some(handler) = child(&mut f, mdia, b"hdlr") else {
            continue;
        };
        let Some(h) = bytes(&mut f, handler, 65536) else {
            continue;
        };
        if h.get(8..12) != Some(b"soun") {
            continue;
        }
        let edts = child(&mut f, track, b"edts")?;
        let elst = child(&mut f, edts, b"elst")?;
        let b = bytes(&mut f, elst, 65536)?;
        let v = *b.first()?;
        if v > 1 {
            return None;
        }
        let count = u32_at(&b, 4)?;
        if count > 4096 {
            return None;
        }
        let mut offset = 8;
        let mut delay = 0u64;
        for _ in 0..count {
            let (duration, media, width) = if v == 1 {
                (u64_at(&b, offset)?, u64_at(&b, offset + 8)? as i64, 20)
            } else {
                (
                    u32_at(&b, offset)? as u64,
                    u32_at(&b, offset + 4)? as i32 as i64,
                    12,
                )
            };
            if media != -1 {
                break;
            }
            delay = delay.checked_add(duration)?;
            offset += width;
        }
        return Some(delay as f64 / scale);
    }
    None
}
