//! Password protection of saved files: AES-256 (PDF 2.0 security handler, revision 6).

use std::collections::BTreeMap;
use std::sync::Arc;

use lopdf::encryption::crypt_filters::{Aes256CryptFilter, CryptFilter};
use lopdf::{EncryptionState, EncryptionVersion, Permissions};

/// What a reader may do without the owner password.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allowed {
    pub print: bool,
    pub copy: bool,
    /// Change pages and content.
    pub modify: bool,
    /// Add annotations and fill forms.
    pub annotate: bool,
}

impl Default for Allowed {
    fn default() -> Self {
        Allowed { print: true, copy: true, modify: true, annotate: true }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Protection {
    /// Needed to open the file; empty opens it without a password, with the restrictions.
    pub user_password: String,
    /// Lifts the restrictions; a random one is used when empty, so nobody can lift them.
    pub owner_password: String,
    pub allowed: Allowed,
}

/// How the saved file is encrypted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Security {
    /// As the opened file: encrypted again with its password, or not encrypted.
    #[default]
    Keep,
    /// Not encrypted (the opened file was unlocked with its password).
    Remove,
    Protect(Protection),
}

#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    #[error("no random numbers available: {0}")]
    Random(String),
    #[error("cannot set up the encryption: {0}")]
    Setup(#[from] lopdf::Error),
}

fn random(bytes: &mut [u8]) -> Result<(), SecurityError> {
    getrandom::fill(bytes).map_err(|error| SecurityError::Random(error.to_string()))
}

impl Protection {
    /// The encryption state to apply to `document` before saving.
    pub fn state(&self) -> Result<EncryptionState, SecurityError> {
        let mut key = [0u8; 32];
        random(&mut key)?;
        let owner = if self.owner_password.is_empty() {
            let mut bytes = [0u8; 16];
            random(&mut bytes)?;
            bytes.iter().map(|byte| format!("{byte:02x}")).collect()
        } else {
            self.owner_password.clone()
        };
        let mut permissions = Permissions::all();
        let allowed = self.allowed;
        for (allow, flags) in [
            (allowed.print, Permissions::PRINTABLE | Permissions::PRINTABLE_IN_HIGH_QUALITY),
            (allowed.copy, Permissions::COPYABLE),
            (allowed.modify, Permissions::MODIFIABLE | Permissions::ASSEMBLABLE),
            (allowed.annotate, Permissions::ANNOTABLE | Permissions::FILLABLE),
        ] {
            if !allow {
                permissions.remove(flags);
            }
        }
        let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
        let version = EncryptionVersion::V5 {
            encrypt_metadata: true,
            crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
            file_encryption_key: &key,
            stream_filter: b"StdCF".to_vec(),
            string_filter: b"StdCF".to_vec(),
            owner_password: &owner,
            user_password: &self.user_password,
            permissions,
        };
        Ok(EncryptionState::try_from(version)?)
    }
}

/// What a reader may do with `document` (all of it when it is not encrypted).
pub fn allowed(document: &lopdf::Document) -> Allowed {
    let Ok(dictionary) = document.get_encrypted() else { return Allowed::default() };
    let Ok(bits) = dictionary.get(b"P").and_then(lopdf::Object::as_i64) else { return Allowed::default() };
    let permissions = Permissions::from_bits_truncate(bits as u32 as u64);
    Allowed {
        print: permissions.contains(Permissions::PRINTABLE),
        copy: permissions.contains(Permissions::COPYABLE),
        modify: permissions.contains(Permissions::MODIFIABLE),
        annotate: permissions.contains(Permissions::ANNOTABLE),
    }
}
