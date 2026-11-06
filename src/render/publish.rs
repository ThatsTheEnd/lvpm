//! The wording of `lvpm publish`.

use super::{Renderer, Stream};
use lvpm_core::events::Event;
use lvpm_core::ops::publish::{Published, SbomReferrer};

/// The wording of the publish module's events.
pub(super) fn render(r: &mut Renderer, event: &Event) {
    match event {
        Event::OciUploading {
            repository,
            tag,
            registry,
        } => r.line(
            Stream::Err,
            &format!("uploading {repository}:{tag} to {registry}"),
        ),
        _ => unreachable!("routed here by Renderer::render: {event:?}"),
    }
}

/// What `lvpm publish` prints once the package is in the feed. For an OCI
/// registry, the digest a signature of the package signs comes second, then
/// what became of the SBOM. An SBOM that was not attached is no line here but
/// [`sbom_failure`], so the package's lines come first and are never lost.
pub fn published(published: &Published) -> String {
    match published {
        Published::Folder { path } => format!("published {}\n", path.display()),
        Published::Oci {
            name,
            version,
            reference,
            digest,
            sbom,
        } => {
            let sbom = match sbom {
                SbomReferrer::Attached => "  SBOM attached\n",
                SbomReferrer::NoSbom => "  no SBOM in the package\n",
                SbomReferrer::Failed(_) => "",
            };
            format!(
                "published {name} {version} to {reference}\n  OCI manifest digest {digest}\n{sbom}"
            )
        }
    }
}

/// The warning for a package that is published but whose SBOM is not attached,
/// which also fails the command: a rerun is refused, so it says so.
pub fn sbom_failure(published: &Published) -> Option<String> {
    match published {
        Published::Oci {
            sbom: SbomReferrer::Failed(cause),
            ..
        } => Some(format!(
            "SBOM could not be attached: {cause}; the package is published and a rerun will be \
             refused\n"
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::renderer;
    use super::*;
    use lvpm_core::oci::Reference;
    use std::path::PathBuf;

    #[test]
    fn a_publish_names_the_file_it_wrote() {
        let p = Published::Folder {
            path: PathBuf::from("demo-1.0.0.0.vip"),
        };
        assert_eq!(published(&p), "published demo-1.0.0.0.vip\n");
    }

    #[test]
    fn an_oci_publish_names_the_reference_and_the_manifest_digest() {
        let p = Published::Oci {
            name: "alpha".into(),
            version: "1.0.0".into(),
            reference: Reference::tagged("ghcr.io", "org/alpha", "1.0.0"),
            digest: "sha256:ab".into(),
            sbom: SbomReferrer::NoSbom,
        };
        assert_eq!(
            published(&p),
            "published alpha 1.0.0 to oci://ghcr.io/org/alpha:1.0.0\n  OCI manifest digest sha256:ab\n  no SBOM in the package\n"
        );
        assert_eq!(sbom_failure(&p), None);
    }

    fn oci_with(sbom: SbomReferrer) -> Published {
        Published::Oci {
            name: "alpha".into(),
            version: "1.0.0".into(),
            reference: Reference::tagged("ghcr.io", "org/alpha", "1.0.0"),
            digest: "sha256:ab".into(),
            sbom,
        }
    }

    #[test]
    fn an_attached_sbom_is_the_last_line_and_a_failed_one_is_a_warning_after_the_package() {
        let attached = published(&oci_with(SbomReferrer::Attached));
        assert!(attached.ends_with("  OCI manifest digest sha256:ab\n  SBOM attached\n"));
        let failed = oci_with(SbomReferrer::Failed(
            "registry r:1 answered 403 to PUT t/a".into(),
        ));
        assert_eq!(
            published(&failed),
            "published alpha 1.0.0 to oci://ghcr.io/org/alpha:1.0.0\n  OCI manifest digest sha256:ab\n"
        );
        assert_eq!(
            sbom_failure(&failed).as_deref(),
            Some(
                "SBOM could not be attached: registry r:1 answered 403 to PUT t/a; the package is \
                 published and a rerun will be refused\n"
            )
        );
        let folder = Published::Folder {
            path: PathBuf::from("a.lvpkg"),
        };
        assert_eq!(sbom_failure(&folder), None);
    }

    #[test]
    fn an_upload_is_announced_on_stderr() {
        let (mut renderer, out, err) = renderer();
        renderer.render(&Event::OciUploading {
            repository: "team/alpha".into(),
            tag: "1.0.0".into(),
            registry: "ghcr.io".into(),
        });
        assert_eq!(
            err.text(),
            "uploading team/alpha:1.0.0 to ghcr.io
"
        );
        assert_eq!(out.text(), "");
    }
}
