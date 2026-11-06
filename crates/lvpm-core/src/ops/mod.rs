//! Operations: what a front end asks core to do. Each takes an [`OpEnv`] and
//! reports through its sink; none prints, reads the environment or knows which
//! front end called it.

pub mod add_remove;
mod attest;
pub mod build;
pub mod build_app;
pub mod check;
pub mod community;
pub mod env;
mod github_index;
pub mod hook_policy;
pub mod hooks;
pub mod init;
pub mod install;
pub mod launch_cmd;
pub mod lock;
pub mod publish;
pub mod query;
pub mod refresh;
pub mod relink;
pub mod resave;
pub mod route;
pub mod session;
pub mod snapshot;
pub mod staging;
pub mod start;
pub mod sync;
#[cfg(test)]
pub(crate) mod testing;
pub mod uninstall;
pub mod venv_cmd;

pub use env::{Destination, DestinationFlags, OpEnv};

use anyhow::Context;

/// The project's root and the project itself, for `lvpm <command>`.
pub(super) fn project_of(
    env: &OpEnv,
    command: &str,
) -> anyhow::Result<(std::path::PathBuf, crate::project::Project)> {
    let Some((manifest, p)) = env.manifest_for(None, None)? else {
        anyhow::bail!(
            "no {} at or above {}: lvpm {command} works on a project",
            crate::project::FILE_NAME,
            env.cwd.display()
        );
    };
    let root = manifest.parent().context("the manifest has no folder")?;
    Ok((root.to_path_buf(), p))
}

/// The project's root, its validated `[distribution]` and the project itself,
/// for `lvpm <command>`.
pub(super) fn packaged(
    env: &OpEnv,
    command: &str,
) -> anyhow::Result<(
    std::path::PathBuf,
    crate::distribution::Distribution,
    crate::project::Project,
)> {
    let (root, p) = project_of(env, command)?;
    let d = crate::distribution::validate(&p, &root)?;
    Ok((root, d, p))
}

/// Where `lvpm build` puts, and `lvpm publish` finds, the current version's package.
pub(super) fn package_path(
    root: &std::path::Path,
    d: &crate::distribution::Distribution,
) -> std::path::PathBuf {
    root.join("dist").join(d.file_name())
}
