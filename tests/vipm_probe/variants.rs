//! The `.vip` variants the probe installs, and the ID one of them carries.

use md5::{Digest, Md5};

/// A package ID derived from what the package holds: MD5 over
/// `name\0version\0`, then `path\0md5(content)\n` for every entry but `spec`,
/// sorted by path (`/` separators). `spec` is left out because it will hold
/// the ID itself, which would make the ID circular. Two builds of the same
/// files get the same ID, so builds stay byte-identical.
pub fn package_id(name: &str, version: &str, entries: &[(String, Vec<u8>)]) -> String {
    let mut files: Vec<(String, &[u8])> = entries
        .iter()
        .filter(|(path, _)| path != "spec")
        .map(|(path, bytes)| (path.replace('\\', "/"), bytes.as_slice()))
        .collect();
    files.sort();
    let mut hash = Md5::new();
    hash.update(format!("{name}\0{version}\0"));
    for (path, bytes) in files {
        hash.update(format!("{path}\0{}\n", hex(&Md5::digest(bytes))));
    }
    hex(&hash.finalize())
}

/// `md-5` 0.11 returns an array with no `LowerHex`, so format the bytes here.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Vec<(String, Vec<u8>)> {
        vec![
            ("spec".into(), b"[Package]".to_vec()),
            ("File Group 0/vi.lib/a.vi".into(), b"aaa".to_vec()),
            ("File Group 0/vi.lib/b.vi".into(), b"bbb".to_vec()),
        ]
    }

    #[test]
    fn the_same_input_gives_the_same_id() {
        assert_eq!(
            package_id("p", "1.0.0.1", &entries()),
            package_id("p", "1.0.0.1", &entries())
        );
    }

    #[test]
    fn the_id_is_32_lowercase_hex_digits() {
        let id = package_id("p", "1.0.0.1", &entries());
        assert_eq!(id.len(), 32);
        assert!(
            id.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
            "{id}"
        );
    }

    #[test]
    fn one_changed_member_changes_the_id() {
        let mut changed = entries();
        changed[2].1 = b"bbc".to_vec();
        assert_ne!(
            package_id("p", "1.0.0.1", &entries()),
            package_id("p", "1.0.0.1", &changed)
        );
    }

    #[test]
    fn the_order_of_the_entries_does_not_matter() {
        let mut reversed = entries();
        reversed.reverse();
        assert_eq!(
            package_id("p", "1.0.0.1", &entries()),
            package_id("p", "1.0.0.1", &reversed)
        );
    }

    #[test]
    fn a_changed_version_or_name_changes_the_id() {
        let id = package_id("p", "1.0.0.1", &entries());
        assert_ne!(id, package_id("p", "1.0.0.2", &entries()));
        assert_ne!(id, package_id("q", "1.0.0.1", &entries()));
    }

    #[test]
    fn a_changed_spec_entry_does_not_change_the_id() {
        let mut changed = entries();
        changed[0].1 = b"[Package]\nID=\"x\"".to_vec();
        assert_eq!(
            package_id("p", "1.0.0.1", &entries()),
            package_id("p", "1.0.0.1", &changed)
        );
    }

    #[test]
    fn a_renamed_member_changes_the_id() {
        let mut renamed = entries();
        renamed[1].0 = "File Group 0/vi.lib/c.vi".into();
        assert_ne!(
            package_id("p", "1.0.0.1", &entries()),
            package_id("p", "1.0.0.1", &renamed)
        );
    }
}
