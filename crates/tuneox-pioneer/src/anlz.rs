//! rekordbox analysis files: `ANLZ0000.DAT`, `.EXT` and `.2EX`.
//!
//! Layout follows Deep Symmetry's DJ Link analysis documentation. A file is a
//! `PMAI` header followed by tagged sections. Every section starts with a
//! fourcc, its header length and its total length (big-endian u32s), so
//! sections we don't understand can be skipped and kept verbatim.

use tuneox_core::{Beat, BeatGrid};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a rekordbox analysis file (missing PMAI header)")]
    NotAnlz,
    #[error("analysis file is truncated or corrupt at byte {0}")]
    Corrupt(usize),
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct AnlzFile {
    data: Vec<u8>,
    /// (start, header length, total length) of each section.
    sections: Vec<(usize, usize, usize)>,
}

/// One tagged section of an analysis file.
pub struct Section<'a> {
    pub fourcc: [u8; 4],
    /// The section header, starting with the fourcc.
    pub header: &'a [u8],
    /// Everything after the header.
    pub body: &'a [u8],
}

impl AnlzFile {
    pub fn parse(data: Vec<u8>) -> Result<Self> {
        if data.get(..4) != Some(b"PMAI") {
            return Err(Error::NotAnlz);
        }
        let mut pos = be_u32(&data, 4).ok_or(Error::Corrupt(4))? as usize;
        let mut sections = Vec::new();
        while pos + 12 <= data.len() {
            let header_len = be_u32(&data, pos + 4).ok_or(Error::Corrupt(pos))? as usize;
            let total_len = be_u32(&data, pos + 8).ok_or(Error::Corrupt(pos))? as usize;
            if header_len < 12 || total_len < header_len || pos + total_len > data.len() {
                return Err(Error::Corrupt(pos));
            }
            sections.push((pos, header_len, total_len));
            pos += total_len;
        }
        Ok(Self { data, sections })
    }

    pub fn sections(&self) -> impl Iterator<Item = Section<'_>> {
        self.sections.iter().map(|&(start, header_len, total_len)| {
            let bytes = &self.data[start..start + total_len];
            Section {
                fourcc: bytes[..4].try_into().unwrap(),
                header: &bytes[..header_len],
                body: &bytes[header_len..],
            }
        })
    }

    pub fn section(&self, fourcc: &[u8; 4]) -> Option<Section<'_>> {
        self.sections().find(|s| &s.fourcc == fourcc)
    }

    /// The beat grid from the `PQTZ` section (in `.DAT` files).
    pub fn beat_grid(&self) -> Result<Option<BeatGrid>> {
        let Some(section) = self.section(b"PQTZ") else {
            return Ok(None);
        };
        // Header: fourcc, lengths, two unknown u32s, beat count.
        let count = be_u32(section.header, 20).ok_or(Error::Corrupt(0))? as usize;
        let beats = section
            .body
            .as_chunks::<8>()
            .0
            .iter()
            .take(count)
            .map(|entry| Beat {
                bar_position: u16::from_be_bytes([entry[0], entry[1]]) as u8,
                bpm_centi: u16::from_be_bytes([entry[2], entry[3]]),
                time_us: u32::from_be_bytes([entry[4], entry[5], entry[6], entry[7]]) as i64 * 1000,
            })
            .collect::<Vec<_>>();
        if beats.len() != count {
            return Err(Error::Corrupt(0));
        }
        Ok(Some(BeatGrid { beats }))
    }

    /// Cues from the extended `PCO2` sections (`.EXT`), falling back to the
    /// legacy `PCOB` sections when there are none.
    pub fn cues(&self) -> Result<Vec<AnlzCue>> {
        let extended = self.cue_lists(b"PCO2", parse_pcp2)?;
        if !extended.is_empty() {
            return Ok(extended);
        }
        self.cue_lists(b"PCOB", parse_pcpt)
    }

    fn cue_lists(
        &self,
        fourcc: &[u8; 4],
        parse_entry: fn(&[u8]) -> Option<AnlzCue>,
    ) -> Result<Vec<AnlzCue>> {
        let mut cues = Vec::new();
        for section in self.sections().filter(|s| &s.fourcc == fourcc) {
            // Both list headers keep the cue count at byte 16 (PCO2) or 18 (PCOB).
            let count_at = if fourcc == b"PCO2" { 16 } else { 18 };
            let count = be_u16(section.header, count_at).ok_or(Error::Corrupt(0))? as usize;
            let mut pos = 0;
            for _ in 0..count {
                let entry_len = be_u32(section.body, pos + 8).ok_or(Error::Corrupt(pos))? as usize;
                let entry = section
                    .body
                    .get(pos..pos + entry_len)
                    .ok_or(Error::Corrupt(pos))?;
                cues.push(parse_entry(entry).ok_or(Error::Corrupt(pos))?);
                pos += entry_len;
            }
        }
        Ok(cues)
    }
}

/// A cue as stored in an analysis file. Times are milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnlzCue {
    /// 0 for a memory cue, 1 = A … 8 = H.
    pub hot_cue: u32,
    pub time_ms: u32,
    /// `None` unless this cue is a loop.
    pub loop_end_ms: Option<u32>,
    pub comment: Option<String>,
}

/// Legacy `PCPT` entry.
fn parse_pcpt(entry: &[u8]) -> Option<AnlzCue> {
    if entry.get(..4)? != b"PCPT" {
        return None;
    }
    let is_loop = *entry.get(28)? == 2;
    Some(AnlzCue {
        hot_cue: be_u32(entry, 12)?,
        time_ms: be_u32(entry, 32)?,
        loop_end_ms: is_loop.then(|| be_u32(entry, 36)).flatten(),
        comment: None,
    })
}

/// Extended `PCP2` entry, which adds colors and a UTF-16 comment.
fn parse_pcp2(entry: &[u8]) -> Option<AnlzCue> {
    if entry.get(..4)? != b"PCP2" {
        return None;
    }
    let is_loop = *entry.get(16)? == 2;
    let comment_len = be_u32(entry, 40).unwrap_or(0) as usize;
    let comment = entry
        .get(44..44 + comment_len)
        .map(|bytes| {
            let units: Vec<u16> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&c| u16::from_be_bytes(c))
                .take_while(|&u| u != 0)
                .collect();
            String::from_utf16_lossy(&units)
        })
        .filter(|c| !c.is_empty());
    Some(AnlzCue {
        hot_cue: be_u32(entry, 12)?,
        time_ms: be_u32(entry, 20)?,
        loop_end_ms: is_loop.then(|| be_u32(entry, 24)).flatten(),
        comment,
    })
}

fn be_u32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

fn be_u16(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(fourcc: &[u8; 4], header_fields: &[u8], body: &[u8]) -> Vec<u8> {
        let header_len = 12 + header_fields.len();
        let mut out = fourcc.to_vec();
        out.extend((header_len as u32).to_be_bytes());
        out.extend(((header_len + body.len()) as u32).to_be_bytes());
        out.extend(header_fields);
        out.extend(body);
        out
    }

    fn file(sections: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = sections.concat();
        let mut out = b"PMAI".to_vec();
        out.extend(28u32.to_be_bytes());
        out.extend((28 + body.len() as u32).to_be_bytes());
        out.extend([0; 16]);
        out.extend(body);
        out
    }

    fn pqtz(beats: &[(u16, u16, u32)]) -> Vec<u8> {
        let mut header = vec![0; 4];
        header.extend(0x80000u32.to_be_bytes());
        header.extend((beats.len() as u32).to_be_bytes());
        let body: Vec<u8> = beats
            .iter()
            .flat_map(|&(bar, bpm, ms)| {
                [bar.to_be_bytes(), bpm.to_be_bytes()]
                    .concat()
                    .into_iter()
                    .chain(ms.to_be_bytes())
            })
            .collect();
        section(b"PQTZ", &header, &body)
    }

    fn pcp2(hot_cue: u32, time_ms: u32, loop_end_ms: Option<u32>, comment: &str) -> Vec<u8> {
        let comment: Vec<u8> = comment
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_be_bytes)
            .collect();
        let mut entry = b"PCP2".to_vec();
        entry.extend(16u32.to_be_bytes());
        entry.extend(0u32.to_be_bytes()); // total length, patched below
        entry.extend(hot_cue.to_be_bytes());
        entry.push(if loop_end_ms.is_some() { 2 } else { 1 });
        entry.extend([0; 3]);
        entry.extend(time_ms.to_be_bytes());
        entry.extend(loop_end_ms.unwrap_or(u32::MAX).to_be_bytes());
        entry.extend([0; 12]);
        entry.extend((comment.len() as u32).to_be_bytes());
        entry.extend(&comment);
        entry.extend([0; 4]);
        let len = entry.len() as u32;
        entry[8..12].copy_from_slice(&len.to_be_bytes());
        entry
    }

    fn pco2(hot: bool, entries: &[Vec<u8>]) -> Vec<u8> {
        let mut header = (hot as u32).to_be_bytes().to_vec();
        header.extend((entries.len() as u16).to_be_bytes());
        header.extend([0; 2]);
        section(b"PCO2", &header, &entries.concat())
    }

    #[test]
    fn reads_beat_grid() {
        let anlz = AnlzFile::parse(file(&[
            section(b"PPTH", &[0; 4], &[]),
            pqtz(&[(1, 12500, 16), (2, 12500, 496)]),
        ]))
        .unwrap();
        let grid = anlz.beat_grid().unwrap().unwrap();
        assert_eq!(grid.beats.len(), 2);
        assert_eq!(grid.beats[1].time_us, 496_000);
        assert_eq!(grid.beats[1].bar_position, 2);
        assert_eq!(grid.constant_bpm(), Some(125.0));
    }

    #[test]
    fn reads_extended_cues() {
        let anlz = AnlzFile::parse(file(&[
            pco2(
                true,
                &[pcp2(1, 24, None, "Drop"), pcp2(2, 15_263, Some(17_000), "")],
            ),
            pco2(false, &[pcp2(0, 4, None, "")]),
        ]))
        .unwrap();
        let cues = anlz.cues().unwrap();
        assert_eq!(cues.len(), 3);
        assert_eq!(cues[0].comment.as_deref(), Some("Drop"));
        assert_eq!(cues[1].loop_end_ms, Some(17_000));
        assert_eq!((cues[2].hot_cue, cues[2].time_ms), (0, 4));
    }

    #[test]
    fn keeps_unknown_sections_and_rejects_garbage() {
        let anlz = AnlzFile::parse(file(&[section(b"PVDI", &[1, 2], &[3, 4, 5])])).unwrap();
        let pvdi = anlz.section(b"PVDI").unwrap();
        assert_eq!(pvdi.body, &[3, 4, 5]);
        assert!(anlz.beat_grid().unwrap().is_none());
        assert!(matches!(
            AnlzFile::parse(b"nope".to_vec()),
            Err(Error::NotAnlz)
        ));
        let mut truncated = file(&[pqtz(&[(1, 12500, 16)])]);
        truncated.truncate(truncated.len() - 3);
        assert!(matches!(AnlzFile::parse(truncated), Err(Error::Corrupt(_))));
    }
}
