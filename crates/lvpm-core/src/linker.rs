//! A VI read as bytes, with no LabVIEW: the blocks of its RSRC container and
//! the LabVIEW version it was saved in.

/// Why a file's bytes could not be read as an RSRC container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LinkerError {
    /// The bytes do not start with the `RSRC` header.
    #[error("not a LabVIEW resource file")]
    NotResource,
    /// A field lies past the end of the bytes, at byte `at`.
    #[error("file truncated at byte {at}")]
    Truncated { at: usize },
}

/// `at + by`, an error rather than a wrap where `usize` is 32 bits.
fn plus(at: usize, by: usize) -> Result<usize, LinkerError> {
    at.checked_add(by).ok_or(LinkerError::Truncated { at })
}

/// A bounds-checked big-endian cursor.
struct Cursor<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn at(b: &'a [u8], pos: usize) -> Self {
        Cursor { b, pos }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], LinkerError> {
        let s = plus(self.pos, n).and_then(|end| {
            self.b
                .get(self.pos..end)
                .ok_or(LinkerError::Truncated { at: self.pos })
        })?;
        self.pos += n;
        Ok(s)
    }

    fn u32(&mut self) -> Result<usize, LinkerError> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().expect("4 bytes")) as usize)
    }
}

/// One section of an RSRC file: a block's type, the section's id, and where
/// its data starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block<'a> {
    pub kind: [u8; 4],
    pub id: u32,
    /// The file offset of the section's `u32 size | bytes` record.
    pub offset: usize,
    /// The bytes, or None when the record's size reaches past the end of the
    /// file.
    pub data: Option<&'a [u8]>,
}

/// The blocks of an RSRC file, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rsrc<'a> {
    pub blocks: Vec<Block<'a>>,
}

impl<'a> Rsrc<'a> {
    /// List the blocks from the header's info and data offsets.
    pub fn read(b: &'a [u8]) -> Result<Self, LinkerError> {
        if !b.starts_with(b"RSRC\r\n") {
            return Err(LinkerError::NotResource);
        }
        let (info, data) = (Cursor::at(b, 16).u32()?, Cursor::at(b, 24).u32()?);
        let base = plus(info, Cursor::at(b, plus(info, 44)?).u32()?)?;
        let mut head = Cursor::at(b, base);
        let mut blocks = Vec::new();
        // Counts are stored less one. A huge one runs off the end of `b`.
        for _ in 0..=head.u32()? {
            let kind: [u8; 4] = head.take(4)?.try_into().expect("4 bytes");
            let (last, list) = (head.u32()?, head.u32()?);
            let mut entry = Cursor::at(b, plus(base, list)?);
            for _ in 0..=last {
                let id = entry.u32()? as u32;
                entry.take(8)?; // name and attribute offsets
                let offset = plus(data, entry.u32()?)?;
                entry.take(4)?;
                let mut record = Cursor::at(b, offset);
                let bytes = record.u32().ok().and_then(|size| record.take(size).ok());
                blocks.push(Block {
                    kind,
                    id,
                    offset,
                    data: bytes,
                });
            }
        }
        Ok(Rsrc { blocks })
    }

    /// The year of the LabVIEW this was saved in, from the first byte of the
    /// `vers` resource (id 4), binary-coded decimal: 0x26 is 26, so 2026.
    /// None for a major below 9, which is LabVIEW 8.x and older, not a year.
    pub fn save_year(&self) -> Option<u32> {
        let vers = self
            .blocks
            .iter()
            .find(|b| &b.kind == b"vers" && b.id == 4)?;
        let bcd = u32::from(*vers.data?.first()?);
        let major = (bcd >> 4) * 10 + (bcd & 0x0f);
        (major >= 9).then_some(2000 + major)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::path::Path;

    fn repo(path: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(path)
    }

    /// A VI saved in LabVIEW 2013, out of a pilot package.
    fn pilot_vi() -> Vec<u8> {
        let vip = std::fs::File::open(repo(
            "pilots/labview_open_source_lib_data_manipulation-1.6.0.16.vip",
        ))
        .unwrap();
        let mut zip = zip::ZipArchive::new(vip).unwrap();
        let mut bytes = Vec::new();
        zip.by_name(
            "File Group 0/vi.lib/LabVIEW Open Source Project/Data Manipulation/KVP Parser.vi",
        )
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
        bytes
    }

    #[test]
    fn a_pilot_vi_lists_its_blocks_and_was_saved_in_2013() {
        let bytes = pilot_vi();
        let rsrc = Rsrc::read(&bytes).unwrap();
        assert!(rsrc.blocks.iter().any(|b| &b.kind == b"LIvi"));
        assert!(rsrc.blocks.iter().all(|b| b.data.is_some()));
        assert_eq!(rsrc.save_year(), Some(2013));
    }

    #[test]
    fn good_vi_was_saved_in_2026() {
        let bytes = std::fs::read(repo("check/project/Good.vi")).unwrap();
        assert_eq!(Rsrc::read(&bytes).unwrap().save_year(), Some(2026));
    }

    #[test]
    fn text_that_is_not_a_resource_file_is_not_one() {
        assert_eq!(Rsrc::read(b"not a VI"), Err(LinkerError::NotResource));
    }

    #[test]
    fn a_truncated_header_is_an_error_not_a_panic() {
        let bytes = pilot_vi();
        for cut in 6..28 {
            assert!(
                matches!(
                    Rsrc::read(&bytes[..cut]),
                    Err(LinkerError::Truncated { .. })
                ),
                "cut at {cut}"
            );
        }
    }

    #[test]
    fn a_major_below_9_is_not_a_year() {
        let mut bytes = pilot_vi();
        let rsrc = Rsrc::read(&bytes).unwrap();
        let vers = rsrc.blocks.iter().find(|b| &b.kind == b"vers" && b.id == 4);
        let at = vers.unwrap().offset + 4;
        for (byte, year) in [(0x08, None), (0x09, Some(2009)), (0x26, Some(2026))] {
            bytes[at] = byte;
            assert_eq!(Rsrc::read(&bytes).unwrap().save_year(), year);
        }
    }
}
