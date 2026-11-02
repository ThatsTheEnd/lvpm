//! The wording of `lvpm build`.

use lvpm_core::ops::build::Built;

/// stderr of a build that ran with `--no-check`.
pub const SKIPPED: &str = "check skipped (--no-check)\n";

/// What `lvpm build` prints once the package is written.
pub fn built(built: &Built) -> String {
    format!("built {} ({} files)\n", built.path.display(), built.files)
}

/// stderr's last line of a build the check refused: `problems` findings.
pub fn refused(problems: usize) -> String {
    format!(
        "build refused: {problems} problem(s) in library members, nothing written; \
         fix them or pass --no-check\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_build_names_the_package_and_counts_its_files() {
        let b = Built {
            path: PathBuf::from("dist/demo-1.0.0.0.vip"),
            files: 4,
            library: None,
        };
        assert_eq!(built(&b), format!("built {} (4 files)\n", b.path.display()));
    }

    #[test]
    fn a_refusal_counts_the_problems_and_says_how_to_go_on() {
        assert_eq!(
            refused(2),
            "build refused: 2 problem(s) in library members, nothing written; \
             fix them or pass --no-check\n"
        );
    }
}
