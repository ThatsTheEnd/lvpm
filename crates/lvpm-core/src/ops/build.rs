//! `build`: package the project's library as a `.vip` in `dist/`.
//!
//! Reads `lvpm.toml`'s `[distribution]` and the library's own member list,
//! and zips the members. No LabVIEW is involved. The package is written to a
//! temporary file and renamed over `dist/<name>-<version>.vip` only once it
//! is complete, so a failed build leaves the previous package as it was.

use super::env::OpEnv;
use crate::{distribution, lvlib, project, spec};
use anyhow::{Context, Result, bail};
use std::io::Write;
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

/// What a build wrote.
#[derive(Debug)]
pub struct Built {
    pub path: PathBuf,
    /// The members packaged, the library itself among them.
    pub files: usize,
}

pub fn build(env: &OpEnv) -> Result<Built> {
    let Some(manifest) = project::find_manifest(&env.cwd) else {
        bail!(
            "no {} at or above {}: lvpm build packages a project",
            project::FILE_NAME,
            env.cwd.display()
        );
    };
    let root = manifest.parent().context("the manifest has no folder")?;
    let d = distribution::validate(&project::read(&manifest)?, root)?;

    let library = root.join(&d.library);
    let folder = library.parent().context("the library has no folder")?;
    let mut members: Vec<(String, PathBuf)> = lvlib::members(&library)?
        .into_iter()
        .map(|m| {
            let m = lvlib::normalize(&m);
            let rel = m.strip_prefix(lvlib::normalize(folder)).map_err(|_| {
                anyhow::anyhow!(
                    "{} is a member of {} but lies outside its folder {}",
                    m.display(),
                    d.library.display(),
                    folder.display()
                )
            })?;
            let rel = rel.to_string_lossy().replace('\\', "/");
            Ok((format!("{}/{rel}", d.install_dir), m))
        })
        .collect::<Result<_>>()?;
    members.sort();

    let spec = spec::write(&d.spec(members.iter().map(|(f, _)| f.clone()).collect()));
    let dist = root.join("dist");
    std::fs::create_dir_all(&dist).with_context(|| format!("creating {}", dist.display()))?;
    let path = dist.join(d.file_name());
    let temp = dist.join(format!(".{}.{}.tmp", d.file_name(), std::process::id()));
    let written = write_zip(&temp, &spec, &members)
        .and_then(|()| std::fs::rename(&temp, &path).context("replacing the package"));
    if let Err(why) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(why.context(format!("writing {}", path.display())));
    }
    Ok(Built {
        path,
        files: members.len(),
    })
}

/// `spec`, then every member, with nothing in an entry that varies between
/// two builds of the same files: fixed 1980 timestamps, one Deflate level,
/// no permissions or extra fields.
fn write_zip(path: &Path, spec: &str, members: &[(String, PathBuf)]) -> Result<()> {
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6))
        .last_modified_time(zip::DateTime::default());
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path)?);
    zip.start_file("spec", options)?;
    zip.write_all(spec.as_bytes())?;
    for (entry, file) in members {
        let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
        zip.start_file(format!("File Group 0/{entry}"), options)?;
        zip.write_all(&bytes)?;
    }
    zip.finish()?.sync_all()?;
    Ok(())
}
