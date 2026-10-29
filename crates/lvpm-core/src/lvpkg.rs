//! The `.lvpkg`: lvpm's own package format (ADR 6).
//!
//! A deterministic zip of `RECORD`, `package.toml` and the payload, the
//! library's members at `<install-dir>/<relative path>`, in the layout of the
//! LabVIEW tree. `package.toml` carries the package's metadata with
//! `lvpm.toml`'s table and key names; `RECORD` lists every other entry with its
//! SHA-256, one `<sha256 hex>  <path>` line each.

use serde::{Serialize, Serializer};
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;

/// The entry that lists every other entry with its SHA-256.
pub const RECORD: &str = "RECORD";
/// The entry that carries the package's metadata.
pub const PACKAGE_TOML: &str = "package.toml";

/// What `package.toml` says about a package.
#[derive(Debug, Clone, PartialEq)]
pub struct Meta {
    pub name: String,
    /// As `lvpm.toml` has it: three parts stay three.
    pub version: String,
    /// The oldest LabVIEW, as `lvpm.toml` writes it: `">=2016"`.
    pub labview: String,
    pub vendor: String,
    pub display_name: String,
    /// Where the payload goes, below the LabVIEW folder, with forward slashes.
    pub install_dir: String,
    /// `[dependencies]` as `lvpm.toml` has them, in file order: name, constraint.
    pub dependencies: Vec<(String, String)>,
}

/// `package.toml`'s shape. The field order is the file's order: serde writes
/// the plain `format` first, then each table.
#[derive(Serialize)]
struct Wire<'a> {
    format: u32,
    project: WireProject<'a>,
    distribution: WireDistribution<'a>,
    #[serde(serialize_with = "as_table")]
    dependencies: &'a [(String, String)],
}

#[derive(Serialize)]
struct WireProject<'a> {
    name: &'a str,
    version: &'a str,
    labview: &'a str,
}

#[derive(Serialize)]
struct WireDistribution<'a> {
    kind: &'a str,
    vendor: &'a str,
    #[serde(rename = "display-name")]
    display_name: &'a str,
    #[serde(rename = "install-dir")]
    install_dir: &'a str,
}

fn as_table<S: Serializer>(deps: &&[(String, String)], s: S) -> Result<S::Ok, S::Error> {
    s.collect_map(deps.iter().map(|(name, constraint)| (name, constraint)))
}

impl Meta {
    /// `package.toml`'s text.
    pub fn to_toml(&self) -> String {
        toml::to_string(&Wire {
            format: 1,
            project: WireProject {
                name: &self.name,
                version: &self.version,
                labview: &self.labview,
            },
            distribution: WireDistribution {
                kind: "source",
                vendor: &self.vendor,
                display_name: &self.display_name,
                install_dir: &self.install_dir,
            },
            dependencies: &self.dependencies,
        })
        .expect("strings and string tables serialise")
    }
}

/// The package as bytes: `RECORD`, `package.toml`, then `files` (path below
/// `install-dir`, content) at `<install-dir>/<path>`, sorted byte-wise by path.
///
/// Nothing in an entry varies between two writes of the same input: fixed 1980
/// timestamps, one Deflate level, the zip crate's default permissions and no
/// extra fields.
///
/// shortcut: panics on a member of 4 GiB or more (a zip without zip64 cannot
/// hold it); no LabVIEW library comes near, upgrade when one does.
pub fn write(meta: &Meta, files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let toml = meta.to_toml();
    let mut payload: Vec<(String, &[u8])> = files
        .iter()
        .map(|(rel, bytes)| (format!("{}/{rel}", meta.install_dir), bytes.as_slice()))
        .collect();
    payload.sort_by(|a, b| a.0.cmp(&b.0));
    let mut entries = vec![(PACKAGE_TOML.to_string(), toml.as_bytes())];
    entries.extend(payload);

    let record: String = entries
        .iter()
        .map(|(path, bytes)| format!("{}  {path}\n", crate::cache::sha256_hex(bytes)))
        .collect();

    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6))
        .last_modified_time(zip::DateTime::default());
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut put = |path: &str, bytes: &[u8]| {
        zip.start_file(path, options).expect("a zip in memory");
        zip.write_all(bytes).expect("a zip in memory");
    };
    put(RECORD, record.as_bytes());
    for (path, bytes) in &entries {
        put(path, bytes);
    }
    zip.finish().expect("a zip in memory").into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> Meta {
        Meta {
            name: "demo_lib".into(),
            version: "1.2.3".into(),
            labview: ">=2016".into(),
            vendor: "V".into(),
            display_name: "D".into(),
            install_dir: "vi.lib/V/D".into(),
            dependencies: Vec::new(),
        }
    }

    fn files(names: &[&str]) -> Vec<(String, Vec<u8>)> {
        names
            .iter()
            .map(|n| (n.to_string(), format!("content of {n}").into_bytes()))
            .collect()
    }

    /// Every entry of `bytes` in archive order: name, time, content.
    fn read_all(bytes: &[u8]) -> Vec<(String, zip::DateTime, Vec<u8>)> {
        use std::io::Read;
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        (0..zip.len())
            .map(|i| {
                let mut f = zip.by_index(i).unwrap();
                let mut content = Vec::new();
                f.read_to_end(&mut content).unwrap();
                (f.name().to_string(), f.last_modified().unwrap(), content)
            })
            .collect()
    }

    fn names(bytes: &[u8]) -> Vec<String> {
        read_all(bytes).into_iter().map(|(n, ..)| n).collect()
    }

    #[test]
    fn entries_are_sorted_with_record_and_package_toml_first() {
        let bytes = write(&meta(), &files(&["b.vi", "a.vi"]));
        assert_eq!(
            names(&bytes),
            [
                "RECORD",
                "package.toml",
                "vi.lib/V/D/a.vi",
                "vi.lib/V/D/b.vi"
            ]
        );
    }

    #[test]
    fn payload_is_sorted_byte_wise_not_by_locale() {
        let bytes = write(
            &meta(),
            &files(&["b.vi", "B.vi", "a.vi", "sub/c.vi", "_x.vi"]),
        );
        assert_eq!(
            names(&bytes)[2..],
            [
                "vi.lib/V/D/B.vi",
                "vi.lib/V/D/_x.vi",
                "vi.lib/V/D/a.vi",
                "vi.lib/V/D/b.vi",
                "vi.lib/V/D/sub/c.vi"
            ]
        );
    }

    #[test]
    fn writing_is_deterministic_and_every_entry_is_dated_1980() {
        let a = write(&meta(), &files(&["b.vi", "a.vi"]));
        let b = write(&meta(), &files(&["a.vi", "b.vi"]));
        assert_eq!(a, b, "the order the files are given in does not matter");
        for (name, time, _) in read_all(&a) {
            assert_eq!(
                (
                    time.year(),
                    time.month(),
                    time.day(),
                    time.hour(),
                    time.minute(),
                    time.second()
                ),
                (1980, 1, 1, 0, 0, 0),
                "{name}"
            );
        }
    }

    #[test]
    fn package_toml_is_written_in_a_fixed_order_with_dependencies_as_given() {
        let mut m = meta();
        m.dependencies = vec![
            ("a".into(), "1.0".into()),
            ("b".into(), "*".into()),
            ("@o/c".into(), ">=2.0".into()),
        ];
        assert_eq!(
            m.to_toml(),
            "format = 1\n\
             \n\
             [project]\n\
             name = \"demo_lib\"\n\
             version = \"1.2.3\"\n\
             labview = \">=2016\"\n\
             \n\
             [distribution]\n\
             kind = \"source\"\n\
             vendor = \"V\"\n\
             display-name = \"D\"\n\
             install-dir = \"vi.lib/V/D\"\n\
             \n\
             [dependencies]\n\
             a = \"1.0\"\n\
             b = \"*\"\n\
             \"@o/c\" = \">=2.0\"\n"
        );
    }

    #[test]
    fn record_lists_every_entry_but_itself_in_entry_order_with_lower_case_sha256() {
        let mut f = files(&["b.vi"]);
        f.push(("a.vi".into(), b"abc".to_vec()));
        let bytes = write(&meta(), &f);
        let all = read_all(&bytes);
        let record = String::from_utf8(all[0].2.clone()).unwrap();
        let lines: Vec<&str> = record.lines().collect();
        assert_eq!(lines.len(), all.len() - 1);
        // SHA-256("abc"), from FIPS 180-2, not computed by the code under test.
        assert_eq!(
            lines[1],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  vi.lib/V/D/a.vi"
        );
        for (line, (name, _, content)) in lines.iter().zip(&all[1..]) {
            let (hash, path) = line.split_once("  ").unwrap();
            assert_eq!(path, name);
            assert_eq!(hash, crate::cache::sha256_hex(content), "{name}");
            assert!(hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        }
        assert!(record.ends_with('\n'));
    }
}
