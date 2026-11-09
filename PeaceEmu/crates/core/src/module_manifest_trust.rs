#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Trust-policy boundary for signed module manifests.
//!
//! Cryptographic verification is injected through a narrow trait. Production
//! callers must provide a reviewed Ed25519 (or stronger approved) implementation.
//! The test verifier below is not cryptographic and must never be used in production.

use crate::module_manifest::{ManifestError, ModuleManifest};
use core::fmt;

/// Signed bytes and detached signature received from a catalogue publisher.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedManifestEnvelope {
    /// Publisher key identifier; must match an explicitly trusted key.
    pub key_id: String,
    /// Exact bytes covered by the detached signature.
    pub payload: Vec<u8>,
    /// Signature bytes produced by the trusted publisher.
    pub signature: Vec<u8>,
}

/// Cryptographic signature-verification provider supplied by the application.
pub trait ManifestSignatureVerifier {
    /// Verifies a signature over the exact payload bytes for a public key.
    ///
    /// Implementations must use a reviewed cryptographic library, reject malformed
    /// keys/signatures, and return success only for a valid signature.
    fn verify(
        &self,
        public_key: &[u8],
        payload: &[u8],
        signature: &[u8],
    ) -> Result<(), SignatureVerificationError>;
}

/// Signature provider rejected the key or signature.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SignatureVerificationError;
impl fmt::Display for SignatureVerificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("manifest signature verification failed")
    }
}
impl std::error::Error for SignatureVerificationError {}

/// Trusted publisher key pinned by the application.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustedManifestKey {
    /// Stable key identifier used by signed envelopes.
    pub key_id: String,
    /// Raw public key bytes in the encoding expected by the verifier.
    pub public_key: Vec<u8>,
}

/// Policy state for accepting signed catalogues and preventing revision rollback.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestTrustPolicy {
    trusted_keys: Vec<TrustedManifestKey>,
    highest_accepted_revision: u64,
}

/// Failure while applying the trust policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManifestTrustError {
    /// The envelope names a key not pinned by the application.
    UntrustedKey(String),
    /// A key ID is empty or repeated in the trust configuration.
    InvalidTrustConfiguration,
    /// Signature verification failed.
    InvalidSignature,
    /// Payload could not be parsed as a module manifest.
    InvalidPayload,
    /// Parsed manifest failed structural validation.
    InvalidManifest(ManifestError),
    /// Manifest revision is not newer than the previously accepted revision.
    RevisionRollback {
        /// Revision supplied by the candidate manifest.
        received: u64,
        /// Highest revision previously accepted by this policy instance.
        highest_accepted: u64,
    },
}
impl fmt::Display for ManifestTrustError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UntrustedKey(id) => write!(f, "manifest signing key '{id}' is not trusted"),
            Self::InvalidTrustConfiguration => {
                f.write_str("trusted manifest keys contain an empty or duplicate key ID")
            },
            Self::InvalidSignature => f.write_str("manifest signature is invalid"),
            Self::InvalidPayload => f.write_str("signed manifest payload could not be parsed"),
            Self::InvalidManifest(error) => write!(f, "signed manifest is structurally invalid: {error}"),
            Self::RevisionRollback {
                received,
                highest_accepted,
            } => write!(
                f,
                "manifest revision {received} is not newer than accepted revision {highest_accepted}"
            ),
        }
    }
}
impl std::error::Error for ManifestTrustError {}

impl ManifestTrustPolicy {
    /// Creates a trust policy from application-pinned keys and a persisted revision floor.
    pub fn new(
        trusted_keys: Vec<TrustedManifestKey>,
        highest_accepted_revision: u64,
    ) -> Result<Self, ManifestTrustError> {
        for (index, key) in trusted_keys.iter().enumerate() {
            if key.key_id.trim().is_empty()
                || key.public_key.is_empty()
                || trusted_keys[..index]
                    .iter()
                    .any(|prior| prior.key_id == key.key_id)
            {
                return Err(ManifestTrustError::InvalidTrustConfiguration);
            }
        }
        Ok(Self {
            trusted_keys,
            highest_accepted_revision,
        })
    }

    /// Returns the highest manifest revision previously accepted by this policy.
    pub const fn highest_accepted_revision(&self) -> u64 {
        self.highest_accepted_revision
    }

    /// Verifies the signature before parsing, validates the manifest, and rejects rollback.
    ///
    /// The parser must parse the provided bytes directly; it must not load a different
    /// path, fetch another document, or accept separately supplied manifest fields.
    /// Persist the returned revision durably before trusting it across restarts.
    pub fn verify_and_parse<V, F>(
        &mut self,
        envelope: &SignedManifestEnvelope,
        verifier: &V,
        parse_verified_payload: F,
    ) -> Result<ModuleManifest, ManifestTrustError>
    where
        V: ManifestSignatureVerifier,
        F: FnOnce(&[u8]) -> Result<ModuleManifest, ()>,
    {
        let key = self
            .trusted_keys
            .iter()
            .find(|key| key.key_id == envelope.key_id)
            .ok_or_else(|| ManifestTrustError::UntrustedKey(envelope.key_id.clone()))?;

        verifier
            .verify(&key.public_key, &envelope.payload, &envelope.signature)
            .map_err(|_| ManifestTrustError::InvalidSignature)?;

        let manifest = parse_verified_payload(&envelope.payload)
            .map_err(|_| ManifestTrustError::InvalidPayload)?;
        manifest
            .validate()
            .map_err(ManifestTrustError::InvalidManifest)?;

        if manifest.revision <= self.highest_accepted_revision {
            return Err(ManifestTrustError::RevisionRollback {
                received: manifest.revision,
                highest_accepted: self.highest_accepted_revision,
            });
        }
        self.highest_accepted_revision = manifest.revision;
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module_management::{ModuleId, ModuleReadiness};
    use crate::module_manifest::{
        ManifestModule, ModuleManifest, ModuleTarget, PackageArtifact, ReleaseVersion, TargetArch,
        TargetOs,
    };

    struct TestVerifier;
    impl ManifestSignatureVerifier for TestVerifier {
        fn verify(
            &self,
            public_key: &[u8],
            payload: &[u8],
            signature: &[u8],
        ) -> Result<(), SignatureVerificationError> {
            // Test-only deterministic sentinel. This is NOT a cryptographic signature.
            if public_key == b"test-public-key" && signature == payload {
                Ok(())
            } else {
                Err(SignatureVerificationError)
            }
        }
    }

    fn manifest(revision: u64) -> ModuleManifest {
        ModuleManifest {
            schema_version: 1,
            revision,
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
        }
    }

    fn policy(revision: u64) -> ManifestTrustPolicy {
        ManifestTrustPolicy::new(
            vec![TrustedManifestKey {
                key_id: "release".into(),
                public_key: b"test-public-key".to_vec(),
            }],
            revision,
        )
        .unwrap()
    }

    fn envelope(revision: u64) -> SignedManifestEnvelope {
        let payload = revision.to_string().into_bytes();
        SignedManifestEnvelope {
            key_id: "release".into(),
            signature: payload.clone(),
            payload,
        }
    }

    #[test]
    fn verifies_before_parsing_and_advances_revision_floor() {
        let mut trust = policy(0);
        let parsed = trust
            .verify_and_parse(&envelope(1), &TestVerifier, |_| Ok(manifest(1)))
            .unwrap();
        assert_eq!(parsed.revision, 1);
        assert_eq!(trust.highest_accepted_revision(), 1);
    }

    #[test]
    fn rejects_unknown_keys_and_bad_signatures_before_parser_runs() {
        let mut trust = policy(0);
        let mut unknown = envelope(1);
        unknown.key_id = "untrusted".into();
        assert!(matches!(
            trust.verify_and_parse(&unknown, &TestVerifier, |_| panic!("must not parse")),
            Err(ManifestTrustError::UntrustedKey(_))
        ));

        let mut invalid = envelope(1);
        invalid.signature = b"tampered".to_vec();
        assert!(matches!(
            trust.verify_and_parse(&invalid, &TestVerifier, |_| panic!("must not parse")),
            Err(ManifestTrustError::InvalidSignature)
        ));
        // Rejected envelopes must never poison the anti-rollback floor.
        assert_eq!(trust.highest_accepted_revision(), 0);
    }

    #[test]
    fn rejects_rollback_and_invalid_parsed_manifest() {
        let mut trust = policy(4);
        assert!(matches!(
            trust.verify_and_parse(&envelope(4), &TestVerifier, |_| Ok(manifest(4))),
            Err(ManifestTrustError::RevisionRollback { .. })
        ));
        assert!(matches!(
            trust.verify_and_parse(&envelope(5), &TestVerifier, |_| Err(())),
            Err(ManifestTrustError::InvalidPayload)
        ));
        assert_eq!(trust.highest_accepted_revision(), 4);
        assert!(matches!(
            trust.verify_and_parse(&envelope(5), &TestVerifier, |_| Ok(manifest(0))),
            Err(ManifestTrustError::InvalidManifest(_))
        ));
        assert_eq!(trust.highest_accepted_revision(), 4);
    }

    #[test]
    fn rejects_duplicate_trusted_key_ids() {
        let key = TrustedManifestKey {
            key_id: "release".into(),
            public_key: vec![1],
        };
        assert!(matches!(
            ManifestTrustPolicy::new(vec![key.clone(), key], 0),
            Err(ManifestTrustError::InvalidTrustConfiguration)
        ));
    }
}
