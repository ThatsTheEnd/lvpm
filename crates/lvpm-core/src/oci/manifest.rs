//! The calls on manifests and blobs: existence checks, reads that check the
//! digest of what came back, and the monolithic blob upload.

use super::reference::valid_digest;
use super::{
    BLOB_TIMEOUT, Error, MANIFEST_TYPE, MAX_BLOB_BYTES, MAX_MANIFEST_BYTES, Oci, Repo, base_url,
    digest_of, expect, location, read_limited,
};
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap};

const DIGEST_HEADER: &str = "Docker-Content-Digest";
const OCTET_STREAM: &str = "application/octet-stream";

/// A manifest read from a registry: its bytes, which hash to its digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub digest: String,
    pub bytes: Vec<u8>,
}

impl Oci {
    /// Whether the repository has a manifest under `reference`, a tag or a
    /// digest: `404` is no, any other failure is an error.
    pub fn manifest_exists(&self, repo: &Repo, reference: &str) -> Result<bool, Error> {
        let url = manifest_url(repo, reference);
        let request = format!("HEAD {}:{reference}", repo.name);
        let response = self.send(repo, |client| {
            client.head(&url).header(ACCEPT, MANIFEST_TYPE)
        })?;
        match response.status().as_u16() {
            404 => Ok(false),
            _ => expect(repo, response, &request, &[200]).map(|_| true),
        }
    }

    /// The manifest under `reference`, its bytes checked against the digest
    /// (the reference's own, when it is one, and the header's).
    pub fn get_manifest(&self, repo: &Repo, reference: &str) -> Result<Manifest, Error> {
        let url = manifest_url(repo, reference);
        let request = format!("GET {}:{reference}", repo.name);
        let response = self.send(repo, |client| {
            client.get(&url).header(ACCEPT, MANIFEST_TYPE)
        })?;
        let response = expect(repo, response, &request, &[200])?;
        let announced = announced_digest(response.headers());
        let bytes = read_limited(repo.registry, response, MAX_MANIFEST_BYTES)?;
        let digest = digest_of(&bytes);
        let wanted = valid_digest(reference).then_some(reference);
        for expected in wanted.into_iter().chain(announced.as_deref()) {
            agree(repo, expected, &digest)?;
        }
        Ok(Manifest { digest, bytes })
    }

    /// Store `bytes` as the manifest under `reference`; its digest.
    pub fn put_manifest(
        &self,
        repo: &Repo,
        reference: &str,
        bytes: &[u8],
    ) -> Result<String, Error> {
        let url = manifest_url(repo, reference);
        let request = format!("PUT {}:{reference}", repo.name);
        let response = self.send(repo, |client| {
            client
                .put(&url)
                .header(CONTENT_TYPE, MANIFEST_TYPE)
                .body(bytes.to_vec())
        })?;
        let response = expect(repo, response, &request, &[200, 201])?;
        let digest = digest_of(bytes);
        if let Some(announced) = announced_digest(response.headers()) {
            agree(repo, &announced, &digest)?;
        }
        Ok(digest)
    }

    /// Whether the repository holds the blob: `404` is no.
    pub fn blob_exists(&self, repo: &Repo, digest: &str) -> Result<bool, Error> {
        let url = blob_url(repo, digest);
        let request = format!("HEAD {}@{digest}", repo.name);
        let response = self.send(repo, |client| client.head(&url))?;
        match response.status().as_u16() {
            404 => Ok(false),
            _ => expect(repo, response, &request, &[200]).map(|_| true),
        }
    }

    /// Upload `bytes` as one blob, in one POST and one PUT; its digest.
    pub fn upload_blob(&self, repo: &Repo, bytes: &[u8]) -> Result<String, Error> {
        let digest = digest_of(bytes);
        let start = format!(
            "{}/v2/{}/blobs/uploads/",
            base_url(repo.registry),
            repo.name
        );
        let request = format!("POST {} blob upload", repo.name);
        let started = self.send(repo, |client| client.post(&start))?;
        let started = expect(repo, started, &request, &[202])?;
        let mut url = location(repo, &start, &started)?;
        url.query_pairs_mut().append_pair("digest", &digest);
        let request = format!("PUT {}@{digest}", repo.name);
        let response = self.send(repo, |client| {
            client
                .put(url.clone())
                .header(CONTENT_TYPE, OCTET_STREAM)
                .timeout(BLOB_TIMEOUT)
                .body(bytes.to_vec())
        })?;
        expect(repo, response, &request, &[201])?;
        Ok(digest)
    }

    /// The blob, its bytes checked against `digest`.
    pub fn get_blob(&self, repo: &Repo, digest: &str) -> Result<Vec<u8>, Error> {
        let url = blob_url(repo, digest);
        let request = format!("GET {}@{digest}", repo.name);
        let response = self.send(repo, |client| client.get(&url).timeout(BLOB_TIMEOUT))?;
        let response = expect(repo, response, &request, &[200])?;
        let bytes = read_limited(repo.registry, response, MAX_BLOB_BYTES)?;
        agree(repo, digest, &digest_of(&bytes))?;
        Ok(bytes)
    }
}

fn manifest_url(repo: &Repo, reference: &str) -> String {
    format!(
        "{}/v2/{}/manifests/{reference}",
        base_url(repo.registry),
        repo.name
    )
}

fn blob_url(repo: &Repo, digest: &str) -> String {
    format!(
        "{}/v2/{}/blobs/{digest}",
        base_url(repo.registry),
        repo.name
    )
}

/// The digest a reply says its body has, when it says one.
fn announced_digest(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(DIGEST_HEADER)?.to_str().ok()?;
    valid_digest(value).then(|| value.to_string())
}

/// Ok when the bytes hashed to `actual` are the ones `expected` names.
fn agree(repo: &Repo, expected: &str, actual: &str) -> Result<(), Error> {
    match expected == actual {
        true => Ok(()),
        false => Err(Error::DigestMismatch {
            registry: repo.registry.to_string(),
            digest: expected.to_string(),
        }),
    }
}
