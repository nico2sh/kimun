//! The server's newest release: what the manifest names, and the proof that a download is
//! the one the project published.
//!
//! The manifest (`latest.json`) is not signed, so nothing it says is trusted for the bytes.
//! The proof is the minisign signature of `checksums-sha256.txt`, checked against the key that
//! ships in this app. The archive's digest is read from that signed file, never from the
//! manifest (ADR-0059). `install-server.sh` does the same check when `minisign` is installed.

use std::collections::BTreeMap;
use std::io::Read;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::marker::BINARY;

/// The public key that signs releases: line 2 of `release-signing.pub`. A test keeps the two
/// in step.
pub const SIGNING_KEY: &str = "RWS64g2Q9vGQfwr3tC6fK3UgWdkKkLfz5KLmcTZhewdMmCHHkkfIgNf8";

/// The manifest layout this build understands.
const SCHEMA: u32 = 1;
/// The largest archive the app downloads. A bound against a rogue redirect.
const MAX_ARCHIVE: u64 = 256 * 1024 * 1024;
/// The largest binary the app unpacks from an archive.
const MAX_BINARY: u64 = 512 * 1024 * 1024;
/// The largest checksums file or signature the app reads.
const MAX_SMALL: u64 = 1024 * 1024;

/// The server release the manifest names, with the three files the app needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRelease {
    /// Plain `X.Y.Z`.
    pub version: String,
    /// The archive's file name, for example `kimun-server-0.5.1-linux-x64.tar.gz`.
    pub archive_name: String,
    pub archive_url: String,
    pub checksums_url: String,
    pub signature_url: String,
}

#[derive(Deserialize)]
struct Head {
    schema: u32,
}

#[derive(Deserialize)]
struct Manifest {
    server: Option<Series>,
}

#[derive(Deserialize)]
struct Series {
    version: String,
    checksums_url: Option<String>,
    signature_url: Option<String>,
    #[serde(default)]
    assets: BTreeMap<String, Asset>,
}

#[derive(Deserialize)]
struct Asset {
    url: String,
}

/// The archive name of the server release `version` for `platform`. Must match `build-server.yml`.
pub fn archive_name(version: &str, platform: &str) -> String {
    format!("kimun-server-{version}-{platform}.tar.gz")
}

/// The server release that the manifest `body` names for `platform`, or the reason it cannot be
/// used. Strict: a plain stable version, https URLs, and a signature that exists.
pub fn parse_manifest(body: &str, platform: &str) -> Result<ServerRelease, String> {
    let head: Head =
        serde_json::from_str(body).map_err(|e| format!("the manifest is not JSON: {e}"))?;
    if head.schema != SCHEMA {
        return Err(format!(
            "the manifest has schema {}, this app reads {SCHEMA}",
            head.schema
        ));
    }
    let manifest: Manifest =
        serde_json::from_str(body).map_err(|e| format!("the manifest is malformed: {e}"))?;
    let series = manifest
        .server
        .ok_or("the manifest names no server release")?;
    if crate::update::parse_version(&series.version).is_none() {
        return Err(format!(
            "{:?} is not a stable server version",
            series.version
        ));
    }
    let archive_name = archive_name(&series.version, platform);
    let archive = series
        .assets
        .get(&archive_name)
        .ok_or_else(|| format!("the release has no {archive_name}"))?;
    let checksums_url = series
        .checksums_url
        .ok_or("the release has no checksums file")?;
    let signature_url = series
        .signature_url
        .ok_or("the release is not signed, so the app will not install it")?;
    for url in [&archive.url, &checksums_url, &signature_url] {
        if !url.starts_with("https://") {
            return Err(format!("{url} is not an https URL"));
        }
    }
    Ok(ServerRelease {
        version: series.version,
        archive_name,
        archive_url: archive.url.clone(),
        checksums_url,
        signature_url,
    })
}

/// Check the signature `sig` of `checksums` against `key`.
pub fn verify_signature(key: &str, checksums: &[u8], sig: &str) -> Result<(), String> {
    let key = minisign_verify::PublicKey::from_base64(key)
        .map_err(|e| format!("bad signing key: {e}"))?;
    let sig =
        minisign_verify::Signature::decode(sig).map_err(|e| format!("bad signature file: {e}"))?;
    key.verify(checksums, &sig, false)
        .map_err(|e| format!("the signature of the checksums file is not valid: {e}"))
}

/// The digest listed for `name` in a `sha256sum`-format file.
pub fn digest_for(checksums: &str, name: &str) -> Option<String> {
    checksums.lines().find_map(|line| {
        let (digest, file) = line.split_once(char::is_whitespace)?;
        // `sha256sum` marks a binary file with a leading `*`.
        (file.trim().trim_start_matches('*') == name).then(|| digest.trim().to_ascii_lowercase())
    })
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(64);
    for b in Sha256::digest(bytes) {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Check the downloaded `archive` against the signed `checksums`: the signature first, then the
/// digest that the signed file lists for the archive.
pub fn verify_archive(
    key: &str,
    release: &ServerRelease,
    archive: &[u8],
    checksums: &[u8],
    signature: &str,
) -> Result<(), String> {
    verify_signature(key, checksums, signature)?;
    let text =
        std::str::from_utf8(checksums).map_err(|_| "the checksums file is not text".to_string())?;
    let expected = digest_for(text, &release.archive_name).ok_or_else(|| {
        format!(
            "the signed checksums file has no entry for {}",
            release.archive_name
        )
    })?;
    let actual = sha256_hex(archive);
    if expected != actual {
        return Err(format!(
            "the download does not match the signed checksum (expected {expected}, got {actual})"
        ));
    }
    Ok(())
}

/// The server binary inside a `.tar.gz` archive.
pub fn extract_binary(archive: &[u8]) -> Result<Vec<u8>, String> {
    extract_binary_capped(archive, MAX_BINARY)
}

/// [`extract_binary`] with a size limit. A binary over the limit is an error, never a cut-off file.
fn extract_binary_capped(archive: &[u8], cap: u64) -> Result<Vec<u8>, String> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let entries = tar
        .entries()
        .map_err(|e| format!("the archive cannot be read: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("the archive cannot be read: {e}"))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let is_binary = entry
            .path()
            .ok()
            .and_then(|p| p.file_name().map(|n| n == BINARY))
            .unwrap_or(false);
        if !is_binary {
            continue;
        }
        let mut bytes = Vec::new();
        entry
            .take(cap + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("the archive cannot be unpacked: {e}"))?;
        if bytes.len() as u64 > cap {
            return Err(format!(
                "{BINARY} in the archive is larger than {} MB",
                cap / (1024 * 1024)
            ));
        }
        if bytes.is_empty() {
            return Err(format!("{BINARY} in the archive is empty"));
        }
        return Ok(bytes);
    }
    Err(format!("{BINARY} is not in the archive"))
}

/// Where the release files come from. A fake in tests, the network in the app.
pub trait Fetch: Send + Sync {
    /// The body at `url`. A body larger than `max` bytes is an error, never a cut-off body.
    fn get(&self, url: &str, max: u64) -> Result<Vec<u8>, String>;
}

/// The network, through `ureq`.
pub struct Network;

impl Fetch for Network {
    fn get(&self, url: &str, max: u64) -> Result<Vec<u8>, String> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(300))
            .build();
        let response = agent
            .get(url)
            .set("User-Agent", crate::update::USER_AGENT)
            .call()
            .map_err(|e| format!("could not download {url}: {e}"))?;
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(max + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("could not read {url}: {e}"))?;
        if bytes.len() as u64 > max {
            return Err(format!(
                "{url} is larger than {} MB",
                max.div_ceil(1024 * 1024)
            ));
        }
        Ok(bytes)
    }
}

/// Download `release`, verify it, and return the server binary inside. Nothing unverified leaves
/// this function.
pub fn fetch_verified_binary(
    fetch: &dyn Fetch,
    key: &str,
    release: &ServerRelease,
) -> Result<Vec<u8>, String> {
    let checksums = fetch.get(&release.checksums_url, MAX_SMALL)?;
    let signature = fetch.get(&release.signature_url, MAX_SMALL)?;
    let signature =
        String::from_utf8(signature).map_err(|_| "the signature file is not text".to_string())?;
    let archive = fetch.get(&release.archive_url, MAX_ARCHIVE)?;
    verify_archive(key, release, &archive, &checksums, &signature)?;
    extract_binary(&archive)
}

/// Helpers that build releases for tests: a tar.gz archive, and a minisign key with a signature.
#[cfg(test)]
pub(super) mod tests_support {
    pub fn make_archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, data) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append_data(&mut header, name, *data).unwrap();
        }
        let tar_bytes = builder.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &tar_bytes).unwrap();
        gz.finish().unwrap()
    }

    /// A fresh key, and the signature of `data` under it: `(public key in base64, signature file text)`.
    pub fn sign_for_test(data: &[u8]) -> (String, String) {
        let pair = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let sig = minisign::sign(
            Some(&pair.pk),
            &pair.sk,
            std::io::Cursor::new(data),
            Some("test"),
            Some("test"),
        )
        .unwrap();
        (pair.pk.to_base64(), sig.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::*;
    use super::*;

    const H: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn manifest(extra: &str) -> String {
        format!(
            r#"{{"schema": 1,
 "app": {{"version": "0.24.8", "assets": {{}}}},
 "server": {{"version": "0.5.1", "checksums_url": "https://dl.example/sums", {extra}
   "assets": {{"kimun-server-0.5.1-linux-x64.tar.gz": {{"url": "https://dl.example/a.tgz", "sha256": "{H}"}}}}}}}}"#
        )
    }

    #[test]
    fn the_manifest_names_the_server_release_for_a_platform() {
        let r = parse_manifest(
            &manifest(r#""signature_url": "https://dl.example/sig","#),
            "linux-x64",
        )
        .unwrap();
        assert_eq!(r.version, "0.5.1");
        assert_eq!(r.archive_name, "kimun-server-0.5.1-linux-x64.tar.gz");
        assert_eq!(r.archive_url, "https://dl.example/a.tgz");
        assert_eq!(r.checksums_url, "https://dl.example/sums");
        assert_eq!(r.signature_url, "https://dl.example/sig");
    }

    #[test]
    fn a_release_that_is_not_signed_is_refused() {
        let err = parse_manifest(&manifest(""), "linux-x64").unwrap_err();
        assert!(err.contains("not signed"), "{err}");
    }

    #[test]
    fn a_platform_with_no_archive_is_refused() {
        let body = manifest(r#""signature_url": "https://dl.example/sig","#);
        let err = parse_manifest(&body, "macos-arm64").unwrap_err();
        assert!(
            err.contains("no kimun-server-0.5.1-macos-arm64.tar.gz"),
            "{err}"
        );
    }

    #[test]
    fn a_manifest_this_build_cannot_trust_is_refused() {
        let ok = manifest(r#""signature_url": "https://dl.example/sig","#);
        assert!(parse_manifest("<html>captive portal</html>", "linux-x64").is_err());
        assert!(
            parse_manifest(r#"{"schema": 2}"#, "linux-x64")
                .unwrap_err()
                .contains("schema 2")
        );
        assert!(
            parse_manifest(r#"{"schema": 1}"#, "linux-x64")
                .unwrap_err()
                .contains("no server release")
        );
        assert!(parse_manifest(&ok.replace("0.5.1", "0.5.1-beta.1"), "linux-x64").is_err());
        assert!(
            parse_manifest(
                &ok.replace("https://dl.example/sig", "http://dl.example/sig"),
                "linux-x64"
            )
            .is_err()
        );
        assert!(
            parse_manifest(
                &ok.replace("https://dl.example/a.tgz", "file:///etc/passwd"),
                "linux-x64"
            )
            .is_err()
        );
    }

    #[test]
    fn a_digest_is_found_by_exact_file_name() {
        let sums = format!(
            "{H}  kimun-server-0.5.1-linux-x64.tar.gz\n{H}  kimun-server-0.5.1-linux-x64.tar.gz.old\n"
        );
        assert_eq!(
            digest_for(&sums, "kimun-server-0.5.1-linux-x64.tar.gz").as_deref(),
            Some(H)
        );
        assert_eq!(digest_for(&sums, "kimun-server-0.5.1-linux"), None);
        let binary = format!("{H} *kimun-server-0.5.1-linux-x64.tar.gz\n");
        assert_eq!(
            digest_for(&binary, "kimun-server-0.5.1-linux-x64.tar.gz").as_deref(),
            Some(H)
        );
    }

    #[test]
    fn the_binary_is_found_in_an_archive_laid_out_like_the_releases() {
        let archive = make_archive(&[("./kimun-server", b"binary")]);
        assert_eq!(extract_binary(&archive).unwrap(), b"binary");
        let nested = make_archive(&[
            ("./README", b"x"),
            ("kimun-server-0.5.1/kimun-server", b"binary2"),
        ]);
        assert_eq!(extract_binary(&nested).unwrap(), b"binary2");
    }

    #[test]
    fn a_binary_over_the_size_limit_is_an_error_not_a_cut_off_file() {
        let archive = make_archive(&[("./kimun-server", &[7u8; 100])]);
        assert!(extract_binary_capped(&archive, 100).is_ok());
        let err = extract_binary_capped(&archive, 99).unwrap_err();
        assert!(err.contains("larger than"), "{err}");
    }

    #[test]
    fn an_archive_without_the_binary_is_refused() {
        let archive = make_archive(&[("./other", b"x")]);
        assert!(
            extract_binary(&archive)
                .unwrap_err()
                .contains("not in the archive")
        );
        assert!(extract_binary(b"not gzip").is_err());
        let empty = make_archive(&[("./kimun-server", b"")]);
        assert!(extract_binary(&empty).unwrap_err().contains("empty"));
    }

    #[test]
    fn a_signature_checks_only_the_text_it_signed_under_its_own_key() {
        let (key, sig) = sign_for_test(b"the checksums");
        assert!(verify_signature(&key, b"the checksums", &sig).is_ok());
        assert!(verify_signature(&key, b"the checksums, edited", &sig).is_err());
        let (other_key, _) = sign_for_test(b"x");
        assert!(verify_signature(&other_key, b"the checksums", &sig).is_err());
        assert!(verify_signature(&key, b"the checksums", "not a signature").is_err());
        assert!(verify_signature("not a key", b"the checksums", &sig).is_err());
    }

    #[test]
    fn the_pinned_key_is_a_valid_minisign_key() {
        assert!(minisign_verify::PublicKey::from_base64(SIGNING_KEY).is_ok());
    }
}
