#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Production Ed25519 verification adapter for signed module catalogue envelopes.
//!
//! This verifies signatures against a public key supplied by the caller. It does
//! not decide which keys are trusted; callers must pin approved public keys in the
//! application and must not accept keys fetched from the same untrusted catalogue.

use ed25519_dalek::{Signature, VerifyingKey};
use emulation_core::module_manifest::ModuleManifest;
use emulation_core::module_manifest_trust::{
    ManifestSignatureVerifier, ManifestTrustError, ManifestTrustPolicy, SignatureVerificationError,
    SignedManifestEnvelope,
};
use serde::Deserialize;

/// Ed25519 signature verifier backed by the reviewed ed25519-dalek implementation.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ed25519ManifestVerifier;

impl ManifestSignatureVerifier for Ed25519ManifestVerifier {
    fn verify(
        &self,
        public_key: &[u8],
        payload: &[u8],
        signature: &[u8],
    ) -> Result<(), SignatureVerificationError> {
        let key_bytes: &[u8; 32] = public_key
            .try_into()
            .map_err(|_| SignatureVerificationError)?;
        let key = VerifyingKey::from_bytes(key_bytes).map_err(|_| SignatureVerificationError)?;
        let signature = Signature::from_slice(signature).map_err(|_| SignatureVerificationError)?;
        key.verify_strict(payload, &signature)
            .map_err(|_| SignatureVerificationError)
    }
}

/// Verifies and parses a signed manifest using the production Ed25519 verifier.
///
/// The trust policy must be constructed from public keys pinned by the application,
/// not from key material supplied by the catalogue. The parser receives only the
/// exact payload bytes covered by the signature.
pub fn verify_signed_manifest<F>(
    policy: &mut ManifestTrustPolicy,
    envelope: &SignedManifestEnvelope,
    parse_verified_payload: F,
) -> Result<ModuleManifest, ManifestTrustError>
where
    F: FnOnce(&[u8]) -> Result<ModuleManifest, ()>,
{
    policy.verify_and_parse(envelope, &Ed25519ManifestVerifier, parse_verified_payload)
}

const MAX_SIGNED_CATALOGUE_BYTES: usize = 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonManifest {
    schema_version: u32,
    revision: u64,
    modules: Vec<JsonManifestModule>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonManifestModule {
    id: String,
    version: String,
    min_app_version: String,
    max_app_version_exclusive: Option<String>,
    targets: Vec<JsonModuleTarget>,
    readiness: String,
    dependencies: Vec<String>,
    artifact: Option<JsonPackageArtifact>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonModuleTarget {
    os: String,
    arch: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonPackageArtifact {
    url: String,
    size_bytes: u64,
    sha256: String,
    signing_key_id: String,
}

/// Verifies a signed JSON catalogue and parses the exact authenticated bytes.
///
/// The JSON schema is strict: unknown fields, malformed IDs or versions, and
/// unknown platform/readiness values are rejected. The 1 MiB cap is checked
/// before parsing. This function does not download packages or install modules.
pub fn verify_signed_manifest_json(
    policy: &mut ManifestTrustPolicy,
    envelope: &SignedManifestEnvelope,
) -> Result<ModuleManifest, ManifestTrustError> {
    verify_signed_manifest(policy, envelope, parse_manifest_json)
}

fn parse_manifest_json(payload: &[u8]) -> Result<ModuleManifest, ()> {
    use emulation_core::module_management::{ModuleId, ModuleReadiness};
    use emulation_core::module_manifest::{
        ManifestModule, ModuleTarget, PackageArtifact, ReleaseVersion, TargetArch, TargetOs,
    };

    if payload.len() > MAX_SIGNED_CATALOGUE_BYTES {
        return Err(());
    }
    let source: JsonManifest = serde_json::from_slice(payload).map_err(|_| ())?;
    let modules = source
        .modules
        .into_iter()
        .map(|module| {
            let readiness = match module.readiness.as_str() {
                "installable" => ModuleReadiness::Installable,
                "experimental" => ModuleReadiness::Experimental,
                "in_development" => ModuleReadiness::InDevelopment,
                _ => return Err(()),
            };
            let targets = module
                .targets
                .into_iter()
                .map(|target| {
                    let os = match target.os.as_str() {
                        "windows" => TargetOs::Windows,
                        "macos" => TargetOs::Macos,
                        "linux" => TargetOs::Linux,
                        _ => return Err(()),
                    };
                    let arch = match target.arch.as_str() {
                        "x86_64" => TargetArch::X86_64,
                        "aarch64" => TargetArch::Aarch64,
                        _ => return Err(()),
                    };
                    Ok(ModuleTarget { os, arch })
                })
                .collect::<Result<Vec<_>, ()>>()?;
            let dependencies = module
                .dependencies
                .into_iter()
                .map(|id| ModuleId::new(id).map_err(|_| ()))
                .collect::<Result<Vec<_>, ()>>()?;
            let artifact = module.artifact.map(|artifact| PackageArtifact {
                url: artifact.url,
                size_bytes: artifact.size_bytes,
                sha256: artifact.sha256,
                signing_key_id: artifact.signing_key_id,
            });
            Ok(ManifestModule {
                id: ModuleId::new(module.id).map_err(|_| ())?,
                version: ReleaseVersion::parse(&module.version).map_err(|_| ())?,
                min_app_version: ReleaseVersion::parse(&module.min_app_version).map_err(|_| ())?,
                max_app_version_exclusive: module
                    .max_app_version_exclusive
                    .as_deref()
                    .map(ReleaseVersion::parse)
                    .transpose()
                    .map_err(|_| ())?,
                targets,
                readiness,
                dependencies,
                artifact,
            })
        })
        .collect::<Result<Vec<_>, ()>>()?;
    let manifest = ModuleManifest {
        schema_version: source.schema_version,
        revision: source.revision,
        modules,
    };
    // Parsing authenticated JSON is not enough: apply the core's semantic checks
    // before any caller can treat the catalogue as usable metadata.
    manifest.validate().map_err(|_| ())?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_hex<const N: usize>(input: &str) -> [u8; N] {
        assert_eq!(input.len(), N * 2);
        let mut output = [0_u8; N];
        for (index, pair) in input.as_bytes().chunks_exact(2).enumerate() {
            let digit = |byte: u8| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                _ => None,
            };
            output[index] = (digit(pair[0]).unwrap() << 4) | digit(pair[1]).unwrap();
        }
        output
    }

    // RFC 8032, section 7.1, test vector 1 (empty message).
    const PUBLIC_KEY: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
    const SIGNATURE: &str = concat!(
        "e5564300c360ac729086e2cc806e828a",
        "84877f1eb8e5d974d873e06522490155",
        "5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
    );

    #[test]
    fn accepts_rfc8032_valid_signature() {
        let key = decode_hex::<32>(PUBLIC_KEY);
        let signature = decode_hex::<64>(SIGNATURE);
        assert!(Ed25519ManifestVerifier
            .verify(&key, b"", &signature)
            .is_ok());
    }

    #[test]
    fn rejects_modified_payload() {
        let key = decode_hex::<32>(PUBLIC_KEY);
        let signature = decode_hex::<64>(SIGNATURE);
        assert!(Ed25519ManifestVerifier
            .verify(&key, b"tampered", &signature)
            .is_err());
    }

    #[test]
    fn rejects_malformed_key_and_signature_lengths() {
        assert!(Ed25519ManifestVerifier
            .verify(b"short", b"", &[0; 64])
            .is_err());
        assert!(Ed25519ManifestVerifier
            .verify(&[0; 32], b"", &[0; 63])
            .is_err());
    }

    #[test]
    fn strict_json_parser_validates_manifest_and_rejects_unknown_fields() {
        let valid = br#"{
            "schema_version": 1,
            "revision": 1,
            "modules": [{
                "id": "sample-module",
                "version": "1.0.0",
                "min_app_version": "0.1.0",
                "max_app_version_exclusive": "2.0.0",
                "targets": [{"os": "windows", "arch": "x86_64"}],
                "readiness": "installable",
                "dependencies": [],
                "artifact": {
                    "url": "https://packages.example.invalid/module.zip",
                    "size_bytes": 128,
                    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "signing_key_id": "release"
                }
            }]
        }"#;
        let parsed = parse_manifest_json(valid).unwrap();
        assert_eq!(parsed.revision, 1);
        assert_eq!(parsed.modules[0].id.as_str(), "sample-module");
        parsed.validate().unwrap();

        let unknown_field = br#"{"schema_version":1,"revision":1,"modules":[],"unexpected":true}"#;
        assert!(parse_manifest_json(unknown_field).is_err());
        assert!(parse_manifest_json(&vec![b' '; MAX_SIGNED_CATALOGUE_BYTES + 1]).is_err());

        // The parser must also reject semantically invalid but syntactically valid
        // manifests; downstream code must never need to remember this extra step.
        let invalid_digest = br#"{
            "schema_version": 1,
            "revision": 1,
            "modules": [{
                "id": "sample-module",
                "version": "1.0.0",
                "min_app_version": "0.1.0",
                "max_app_version_exclusive": "2.0.0",
                "targets": [{"os": "windows", "arch": "x86_64"}],
                "readiness": "installable",
                "dependencies": [],
                "artifact": {
                    "url": "https://packages.example.invalid/module.zip",
                    "size_bytes": 128,
                    "sha256": "NOT-A-SHA256",
                    "signing_key_id": "release"
                }
            }]
        }"#;
        assert!(parse_manifest_json(invalid_digest).is_err());

        let invalid_revision = br#"{"schema_version":1,"revision":0,"modules":[]}"#;
        assert!(parse_manifest_json(invalid_revision).is_err());
    }

    #[test]
    fn production_adapter_uses_ed25519_before_accepting_manifest() {
        use emulation_core::module_management::{ModuleId, ModuleReadiness};
        use emulation_core::module_manifest::{
            ManifestModule, ModuleTarget, PackageArtifact, ReleaseVersion, TargetArch, TargetOs,
        };
        use emulation_core::module_manifest_trust::{
            ManifestTrustPolicy, SignedManifestEnvelope, TrustedManifestKey,
        };

        use ed25519_dalek::{Signer, SigningKey};

        // This seed is test-only; never use it for a production catalogue.
        let test_seed =
            decode_hex::<32>("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
        let signing_key = SigningKey::from_bytes(&test_seed);
        let key = signing_key.verifying_key().to_bytes();
        let payload = b"manifest-test-payload".to_vec();
        let signature = signing_key.sign(&payload).to_bytes();
        let mut policy = ManifestTrustPolicy::new(
            vec![TrustedManifestKey {
                key_id: "test-rfc8032-key".to_owned(),
                public_key: key.to_vec(),
            }],
            0,
        )
        .unwrap();
        let envelope = SignedManifestEnvelope {
            key_id: "test-rfc8032-key".to_owned(),
            payload,
            signature: signature.to_vec(),
        };
        let parsed = verify_signed_manifest(&mut policy, &envelope, |payload| {
            if payload != b"manifest-test-payload" {
                return Err(());
            }
            Ok(ModuleManifest {
                schema_version: 1,
                revision: 1,
                modules: vec![ManifestModule {
                    id: ModuleId::new("sample-module").unwrap(),
                    version: ReleaseVersion::parse("1.0.0").unwrap(),
                    min_app_version: ReleaseVersion::parse("0.1.0").unwrap(),
                    max_app_version_exclusive: Some(ReleaseVersion::parse("2.0.0").unwrap()),
                    targets: vec![ModuleTarget {
                        os: TargetOs::Linux,
                        arch: TargetArch::X86_64,
                    }],
                    readiness: ModuleReadiness::Installable,
                    dependencies: vec![],
                    artifact: Some(PackageArtifact {
                        url: "https://packages.example.invalid/module.tar".into(),
                        size_bytes: 1,
                        sha256: "a".repeat(64),
                        signing_key_id: "release".into(),
                    }),
                }],
            })
        })
        .unwrap();

        assert_eq!(parsed.revision, 1);
        assert_eq!(policy.highest_accepted_revision(), 1);
        let mut tampered = envelope;
        tampered.payload = b"tampered".to_vec();
        assert!(verify_signed_manifest(&mut policy, &tampered, |_| {
            panic!("parser must not run after a signature failure")
        })
        .is_err());
    }
}
