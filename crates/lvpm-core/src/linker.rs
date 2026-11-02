//! A VI read as bytes, with no LabVIEW: the blocks of its RSRC container, the
//! LabVIEW version it was saved in, and the `PTH0` path records that its
//! linker tables and VI Server's wire format share.

use anyhow::{Result, bail, ensure};
use std::path::Path;

/// Why a file's bytes could not be read as an RSRC container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LinkerError {
    /// The bytes do not start with the `RSRC` header.
    #[error("not a LabVIEW resource file")]
    NotResource,
    /// A field lies past the end of the bytes, at byte `at`.
    #[error("truncated at byte {at}")]
    Truncated { at: usize },
    /// A section that should hold a table does not: it is missing (offset 0),
    /// runs past the end of the file, or holds a record that does not parse.
    /// `offset` is the file byte where that section or record starts.
    #[error("{block} not readable at byte {offset}")]
    Unreadable { block: &'static str, offset: usize },
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

    fn u16(&mut self) -> Result<u16, LinkerError> {
        Ok(u16::from_be_bytes(
            self.take(2)?.try_into().expect("2 bytes"),
        ))
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

    /// Whether the VI keeps its compiled code in a separate file, from bit
    /// 0x400 of the `LVSR` u32 at byte 8. Good.vi, saved with "Separate
    /// compiled code from source file" on, has the bit set (0x600);
    /// NotSeparated.vi, the same VI saved with the compiled code kept in the
    /// file, has it clear (0x200). None when there is no such `LVSR`.
    pub fn separate_compiled(&self) -> Option<bool> {
        let lvsr = self.blocks.iter().find(|b| &b.kind == b"LVSR")?;
        let flags = Cursor::at(lvsr.data?, 8).u32().ok()?;
        Some(flags & 0x400 != 0)
    }

    /// Every file a VI links to, from its three linker tables, once each. The
    /// empty paths LabVIEW writes after most links are not links.
    pub fn links(&self) -> Result<Vec<LinkPath>, LinkerError> {
        let mut links = Vec::new();
        for table in ["LIvi", "LIfp", "LIbd"] {
            for path in self.table_paths(table)? {
                if !links.contains(&path) {
                    links.push(path);
                }
            }
        }
        Ok(links)
    }

    /// The paths in one linker table. Every table is `u16 version | 4-char
    /// owner | u32 entry count | entries | u16 3`, but the entries differ per
    /// link type and are not documented, so this finds each `PTH0` record by
    /// its magic and checks the bounds of each one.
    // shortcut: reads PTH0 records by magic, not entry structure; upgrade once the entry layout is documented
    fn table_paths(&self, table: &'static str) -> Result<Vec<LinkPath>, LinkerError> {
        let missing = LinkerError::Unreadable {
            block: table,
            offset: 0,
        };
        let block = self
            .blocks
            .iter()
            .find(|b| b.kind == table.as_bytes())
            .ok_or(missing)?;
        let data = block.data.ok_or(LinkerError::Unreadable {
            block: table,
            offset: block.offset,
        })?;
        let mut paths = Vec::new();
        let mut at = 0;
        while let Some(found) = data[at..].windows(4).position(|w| w == b"PTH0") {
            let start = at + found;
            let bad = LinkerError::Unreadable {
                block: table,
                // the record's own bytes follow the section's 4-byte size
                offset: block.offset + 4 + start,
            };
            if Cursor::at(data, start + 4).u32().map_err(|_| bad)? == 0 {
                at = start + 8;
                continue;
            }
            let (path, used) = decode_pth0(&data[start..]).map_err(|_| bad)?;
            at = start + used;
            if path.kind == Kind::NotAPath || !path.components.is_empty() {
                paths.push(path);
            }
        }
        Ok(paths)
    }
}

/// What `lvpm check` needs from one VI file, read once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViFacts {
    pub save_year: Option<u32>,
    pub separate_compiled: bool,
    pub links: Vec<LinkPath>,
}

impl ViFacts {
    pub fn read(bytes: &[u8]) -> Result<Self, LinkerError> {
        let rsrc = Rsrc::read(bytes)?;
        let separate_compiled = rsrc.separate_compiled().ok_or(LinkerError::Unreadable {
            block: "LVSR",
            offset: 0,
        })?;
        Ok(ViFacts {
            save_year: rsrc.save_year(),
            separate_compiled,
            links: rsrc.links()?,
        })
    }
}

/// The names of the VIs in an `.llb`, from its `STR#` section 1, a Mac string
/// list: `u16 count | count pascal names`, with nothing after the last.
// shortcut: layout read from one single-member .llb (Removed.vi was deleted from it); check it against a multi-member .llb when one is a fixture
pub fn llb_members(bytes: &[u8]) -> Result<Vec<String>, LinkerError> {
    let rsrc = Rsrc::read(bytes)?;
    let unreadable = |offset| LinkerError::Unreadable {
        block: "STR#",
        offset,
    };
    let list = rsrc
        .blocks
        .iter()
        .find(|b| &b.kind == b"STR#" && b.id == 1)
        .ok_or(unreadable(0))?;
    let data = list.data.ok_or(unreadable(list.offset))?;
    let mut cursor = Cursor::at(data, 0);
    let names = (|| {
        let mut names = Vec::new();
        for _ in 0..cursor.u16()? {
            let len = cursor.take(1)?[0] as usize;
            names.push(String::from_utf8_lossy(cursor.take(len)?).into_owned());
        }
        Ok::<_, LinkerError>(names)
    })();
    match names {
        Ok(names) if cursor.pos == data.len() => Ok(names),
        _ => Err(unreadable(list.offset)),
    }
}

/// Where a link's path starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Absolute,
    Relative,
    /// Under a LabVIEW search root such as `<vilib>`; the root is not among
    /// the components.
    Symbolic(Root),
    /// LabVIEW's "not a path".
    NotAPath,
}

/// A symbolic path's root, as LabVIEW writes it between angle brackets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Root {
    ViLib,
    UserLib,
    InstrLib,
    /// Any other `<...>`, as written, brackets included.
    Other(String),
}

impl Root {
    fn text(&self) -> &str {
        match self {
            Root::ViLib => "<vilib>",
            Root::UserLib => "<userlib>",
            Root::InstrLib => "<instrlib>",
            Root::Other(text) => text,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Component {
    Name(String),
    /// One folder up: an empty component of a relative path.
    Up,
}

/// The path a `PTH0` record holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkPath {
    pub kind: Kind,
    pub components: Vec<Component>,
}

impl LinkPath {
    /// The path as this machine writes it. A drive letter, which LabVIEW
    /// stores as a bare component, gets its colon back; a Linux path has one
    /// root and no drives, so an absolute one is rooted again.
    pub fn to_path_string(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        match &self.kind {
            Kind::NotAPath => return "<Not A Path>".to_string(),
            Kind::Symbolic(root) => parts.push(root.text().to_string()),
            Kind::Absolute | Kind::Relative => {}
        }
        parts.extend(self.components.iter().map(|c| match c {
            Component::Name(name) => name.clone(),
            Component::Up => "..".to_string(),
        }));
        #[cfg(windows)]
        {
            if self.kind == Kind::Absolute
                && let Some(first) = parts.first_mut()
                && first.len() == 1
                && first.chars().all(|c| c.is_ascii_alphabetic())
            {
                first.push(':');
            }
            parts.join("\\")
        }
        #[cfg(target_os = "linux")]
        {
            let rooted = matches!(self.kind, Kind::Absolute | Kind::Symbolic(_));
            format!("{}{}", if rooted { "/" } else { "" }, parts.join("/"))
        }
    }
}

/// Read one flattened `PTH0` record from the start of `b`, and how many bytes
/// it took. The inverse of [`encode_pth0`]:
/// `"PTH0" | u32 length | u16 kind | u16 levels | pascal components`, where
/// the components carry no padding. `kind` is 0 for an absolute path
/// (symbolic when the first component is `<...>`), 1 for a relative one and 2
/// for LabVIEW's "not a path".
pub fn decode_pth0(b: &[u8]) -> Result<(LinkPath, usize)> {
    let mut record = Cursor::at(b, 0);
    let magic = record.take(4)?;
    ensure!(magic == b"PTH0", "expected a PTH0 record, got {magic:02x?}");
    let len = record.u32()?;
    ensure!(len >= 4, "PTH0 length {len} is too small");
    let mut body = Cursor::at(record.take(len)?, 0);
    let (kind, levels) = (body.u16()?, body.u16()?);
    let used = record.pos;
    if kind == 2 {
        let path = LinkPath {
            kind: Kind::NotAPath,
            components: Vec::new(),
        };
        return Ok((path, used));
    }
    let mut names: Vec<String> = Vec::with_capacity(levels as usize);
    for _ in 0..levels {
        let n = body.take(1)?[0] as usize;
        names.push(String::from_utf8_lossy(body.take(n)?).into_owned());
    }
    // shortcut: any kind but 0 and 2 reads as relative, as it always did here
    // (3 is a UNC path); give it a Kind of its own when a link uses one.
    let (kind, names) = match (kind, names.split_first()) {
        (0, Some((root, rest))) if root.starts_with('<') && root.ends_with('>') => {
            let root = match root.as_str() {
                "<vilib>" => Root::ViLib,
                "<userlib>" => Root::UserLib,
                "<instrlib>" => Root::InstrLib,
                other => Root::Other(other.to_string()),
            };
            (Kind::Symbolic(root), rest.to_vec())
        }
        (0, _) => (Kind::Absolute, names),
        _ => (Kind::Relative, names),
    };
    let relative = kind == Kind::Relative;
    let components = names
        .into_iter()
        .map(|name| match name.is_empty() && relative {
            true => Component::Up,
            false => Component::Name(name),
        })
        .collect();
    Ok((LinkPath { kind, components }, used))
}

/// Encode a path as LabVIEW's `PTH0` record.
///
/// The same structure appears inside a VI's `LIvi`/`LIbd` linker blocks, so this
/// encoder serves both the wire protocol and any future offline path rewriting:
/// `"PTH0" | u32 byte length | u32 component count | pascal components`, with an
/// absolute Windows path contributing its drive letter as the first component.
/// A Linux path is the same record without one: `/usr/local` is `usr`,
/// `local` (verified against LabVIEW 2026 Q3 on Linux).
pub(crate) fn encode_pth0(path: &Path) -> Result<Vec<u8>> {
    let s = path.to_string_lossy().replace('/', "\\");
    let mut parts: Vec<String> = Vec::new();
    for (i, comp) in s.split('\\').filter(|c| !c.is_empty()).enumerate() {
        // "C:" becomes the component "C".
        if i == 0 && comp.len() == 2 && comp.ends_with(':') {
            parts.push(comp[..1].to_string());
        } else {
            parts.push(comp.to_string());
        }
    }
    if parts.is_empty() {
        bail!("cannot encode empty path");
    }

    let mut components = Vec::new();
    for p in &parts {
        let bytes = p.as_bytes();
        if bytes.len() > 255 {
            bail!("path component too long for PTH0: {p:?}");
        }
        components.push(bytes.len() as u8);
        components.extend_from_slice(bytes);
    }

    let mut out = Vec::with_capacity(components.len() + 16);
    out.extend_from_slice(b"PTH0");
    // Length covers the component count field plus the components themselves.
    out.extend_from_slice(&((components.len() + 4) as u32).to_be_bytes());
    out.extend_from_slice(&(parts.len() as u32).to_be_bytes());
    out.extend_from_slice(&components);
    Ok(out)
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

    /// A `PTH0` record of `kind` with `parts` as its components.
    fn record(kind: u16, parts: &[&str]) -> Vec<u8> {
        let mut body = [kind.to_be_bytes(), (parts.len() as u16).to_be_bytes()].concat();
        for p in parts {
            body.push(p.len() as u8);
            body.extend(p.as_bytes());
        }
        [
            b"PTH0".as_slice(),
            &(body.len() as u32).to_be_bytes(),
            &body,
        ]
        .concat()
    }

    fn names(parts: &[&str]) -> Vec<Component> {
        parts
            .iter()
            .map(|p| Component::Name(p.to_string()))
            .collect()
    }

    #[test]
    fn a_record_decodes_to_its_kind_and_components_and_says_how_much_it_took() {
        let mut bytes = record(0, &["C", "Git", "x.vi"]);
        let took = bytes.len();
        bytes.extend(b"more");
        let (path, used) = decode_pth0(&bytes).unwrap();
        assert_eq!(used, took);
        assert_eq!(path.kind, Kind::Absolute);
        assert_eq!(path.components, names(&["C", "Git", "x.vi"]));
    }

    #[test]
    fn a_symbolic_root_is_not_a_component() {
        for (root, want) in [
            ("<vilib>", Root::ViLib),
            ("<userlib>", Root::UserLib),
            ("<instrlib>", Root::InstrLib),
            ("<resource>", Root::Other("<resource>".into())),
        ] {
            let (path, _) = decode_pth0(&record(0, &[root, "Check Fixture", "Dep.vi"])).unwrap();
            assert_eq!(path.kind, Kind::Symbolic(want));
            assert_eq!(path.components, names(&["Check Fixture", "Dep.vi"]));
        }
    }

    #[test]
    fn an_empty_component_of_a_relative_path_is_up() {
        let (path, _) = decode_pth0(&record(1, &["", "Sub", "x.vi"])).unwrap();
        assert_eq!(path.kind, Kind::Relative);
        assert_eq!(
            path.components,
            [
                Component::Up,
                Component::Name("Sub".into()),
                Component::Name("x.vi".into())
            ]
        );
    }

    #[test]
    fn not_a_path_has_no_components() {
        let (path, used) = decode_pth0(b"PTH0\x00\x00\x00\x04\x00\x02\x00\x00").unwrap();
        assert_eq!(
            (path.kind, path.components, used),
            (Kind::NotAPath, vec![], 12)
        );
    }

    #[test]
    fn a_record_that_is_not_one_or_is_cut_short_is_an_error() {
        assert!(decode_pth0(b"XXXX\x00\x00\x00\x04\x00\x00\x00\x00").is_err());
        let whole = record(0, &["C", "x.vi"]);
        for cut in 0..whole.len() {
            assert!(decode_pth0(&whole[..cut]).is_err(), "cut at {cut}");
        }
    }

    #[test]
    fn a_path_is_written_the_way_this_machine_does() {
        let sep = if cfg!(windows) { "\\" } else { "/" };
        let root = if cfg!(windows) { "" } else { "/" };
        let written = |kind, parts: &[&str]| {
            decode_pth0(&record(kind, parts))
                .unwrap()
                .0
                .to_path_string()
        };
        assert_eq!(
            written(0, &["<vilib>", "Dep.vi"]),
            format!("{root}<vilib>{sep}Dep.vi")
        );
        assert_eq!(written(1, &["", "x.vi"]), format!("..{sep}x.vi"));
        assert_eq!(written(2, &[]), "<Not A Path>");
    }

    #[test]
    fn the_compiled_code_flag_tells_the_minimal_pair_apart() {
        let flag = |name| Rsrc::read(&project_vi(name)).unwrap().separate_compiled();
        assert_eq!(flag("Good.vi"), Some(true));
        assert_eq!(flag("NotSeparated.vi"), Some(false));
    }

    #[test]
    fn an_llb_lists_the_vis_it_still_holds() {
        let llb = project_vi("Archive.llb");
        assert_eq!(llb_members(&llb).unwrap(), ["InLlb.vi"]);
    }

    #[test]
    fn an_llb_list_that_does_not_add_up_is_unreadable() {
        let mut llb = project_vi("Archive.llb");
        let rsrc = Rsrc::read(&llb).unwrap();
        let list = rsrc.blocks.iter().find(|b| &b.kind == b"STR#" && b.id == 1);
        let offset = list.unwrap().offset;
        llb[offset + 5] = 2; // two names are claimed, one is there
        assert_eq!(
            llb_members(&llb),
            Err(LinkerError::Unreadable {
                block: "STR#",
                offset
            })
        );
        assert_eq!(
            llb_members(&project_vi("Good.vi")),
            Err(LinkerError::Unreadable {
                block: "STR#",
                offset: 0
            })
        );
    }

    #[test]
    fn a_vi_is_read_once_into_its_facts() {
        let facts = ViFacts::read(&project_vi("Good.vi")).unwrap();
        assert_eq!(facts.save_year, Some(2026));
        assert!(facts.separate_compiled);
        assert_eq!(facts.links, [rel(&["Lib.lvlib"]), rel(&["Helper.vi"])]);
        assert_eq!(ViFacts::read(b"not a VI"), Err(LinkerError::NotResource));
    }

    fn project_vi(name: &str) -> Vec<u8> {
        std::fs::read(repo(&format!("check/project/{name}"))).unwrap()
    }

    fn links_of(bytes: &[u8]) -> Vec<LinkPath> {
        Rsrc::read(bytes).unwrap().links().unwrap()
    }

    fn link(kind: Kind, parts: &[&str]) -> LinkPath {
        LinkPath {
            kind,
            components: names(parts),
        }
    }

    /// A link LabVIEW stores relative to the VI file itself: the first
    /// component is Up, from the file to its folder.
    fn rel(parts: &[&str]) -> LinkPath {
        let mut path = link(Kind::Relative, parts);
        path.components.insert(0, Component::Up);
        path
    }

    #[test]
    fn good_vi_links_to_its_library_and_helper_once_each() {
        assert_eq!(
            links_of(&project_vi("Good.vi")),
            [rel(&["Lib.lvlib"]), rel(&["Helper.vi"])]
        );
        assert_eq!(
            links_of(&project_vi("Broken.vi")),
            [rel(&["Sub", "Gone.vi"])]
        );
    }

    #[test]
    fn a_vilib_link_is_symbolic() {
        assert!(links_of(&project_vi("UsesDep.vi")).contains(&link(
            Kind::Symbolic(Root::ViLib),
            &["Check Fixture", "Dep.vi"]
        )));
    }

    #[test]
    fn links_through_an_llb_keep_the_llb_as_a_component() {
        assert_eq!(
            links_of(&project_vi("UsesLlb.vi")),
            [
                rel(&["Archive.llb", "InLlb.vi"]),
                rel(&["Archive.llb", "Removed.vi"])
            ]
        );
    }

    /// `bytes` with the size of its `kind` section set past the end of the
    /// file, and the file offset of that section.
    fn size_past_the_end(mut bytes: Vec<u8>, kind: &[u8; 4]) -> (Vec<u8>, usize) {
        let rsrc = Rsrc::read(&bytes).unwrap();
        let at = rsrc.blocks.iter().find(|b| &b.kind == kind).unwrap().offset;
        bytes[at..at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        (bytes, at)
    }

    #[test]
    fn a_table_whose_size_runs_past_the_file_is_unreadable_at_its_offset() {
        for block in ["LIvi", "LIfp", "LIbd"] {
            let (bytes, offset) =
                size_past_the_end(project_vi("Good.vi"), block.as_bytes().try_into().unwrap());
            assert_eq!(
                Rsrc::read(&bytes).unwrap().links(),
                Err(LinkerError::Unreadable { block, offset })
            );
        }
    }

    #[test]
    fn a_path_record_that_overruns_its_table_is_unreadable_where_it_starts() {
        let mut bytes = project_vi("Good.vi");
        let rsrc = Rsrc::read(&bytes).unwrap();
        let livi = rsrc.blocks.iter().find(|b| &b.kind == b"LIvi").unwrap();
        let magic = livi
            .data
            .unwrap()
            .windows(4)
            .position(|w| w == b"PTH0")
            .unwrap();
        let record = livi.offset + 4 + magic;
        bytes[record + 4..record + 8].copy_from_slice(&1000u32.to_be_bytes());
        assert_eq!(
            Rsrc::read(&bytes).unwrap().links(),
            Err(LinkerError::Unreadable {
                block: "LIvi",
                offset: record
            })
        );
    }

    #[test]
    fn viserver_has_no_pth0_codec_of_its_own() {
        let viserver = include_str!("viserver.rs");
        assert!(!viserver.contains("fn encode_pth0"));
        assert!(!viserver.contains("fn decode_pth0"));
    }
}
