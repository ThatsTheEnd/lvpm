//! The wording of `lvpm publish`.

use super::{Renderer, Stream};
use lvpm_core::events::Event;
use lvpm_core::ops::publish::Published;

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
/// registry, the digest a signature of the package signs comes second.
pub fn published(published: &Published) -> String {
    match published {
        Published::Folder { path } => format!("published {}\n", path.display()),
        Published::Oci {
            name,
            version,
            reference,
            digest,
        } => format!("published {name} {version} to {reference}\n  OCI manifest digest {digest}\n"),
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
        };
        assert_eq!(
            published(&p),
            "published alpha 1.0.0 to oci://ghcr.io/org/alpha:1.0.0\n  OCI manifest digest sha256:ab\n"
        );
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
