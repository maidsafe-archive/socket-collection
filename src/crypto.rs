// Copyright 2018 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under the MIT license <LICENSE-MIT
// http://opensource.org/licenses/MIT> or the Modified BSD license <LICENSE-BSD
// https://opensource.org/licenses/BSD-3-Clause>, at your option. This file may not be copied,
// modified, or distributed except according to those terms. Please review the Licences for the
// specific language governing permissions and limitations relating to use of the SAFE Network
// Software.

use maidsafe_utilities::serialisation::{deserialise, serialise};
use safe_crypto::{PublicEncryptKey, SecretEncryptKey, SharedSecretKey};
use serde::de::DeserializeOwned;
use serde::Serialize;

/// Simplifies encryption by holding the necessary context - encryption keys.
/// Allows "null" encryption where data is only serialized. See: null object pattern.
#[derive(Clone, Debug)]
pub enum EncryptContext {
    /// No encryption.
    Null,
    /// Encryption + authentication
    Authenticated { shared_key: SharedSecretKey },
    /// No message authentication. Only encrypt operation is allowed.
    AnonymousEncrypt {
        /// Their public key.
        their_pk: PublicEncryptKey,
    },
}

impl Default for EncryptContext {
    /// Default is "null" encryption.
    fn default() -> Self {
        Self::null()
    }
}

impl EncryptContext {
    /// Contructs "null" encryption context which actually does no encryption.
    /// In this case data is simply serialized but not encrypted.
    pub fn null() -> Self {
        EncryptContext::Null
    }

    /// Construct crypto context that encrypts and authenticate messages.
    pub fn authenticated(shared_key: SharedSecretKey) -> Self {
        EncryptContext::Authenticated { shared_key }
    }

    /// Constructs crypto context that is only meant for unauthenticated encryption.
    pub fn anonymous_encrypt(their_pk: PublicEncryptKey) -> Self {
        EncryptContext::AnonymousEncrypt { their_pk }
    }

    /// Serialize given structure and encrypt it.
    pub fn encrypt<T: Serialize>(&self, msg: &T) -> ::Res<Vec<u8>> {
        Ok(match *self {
            EncryptContext::Null => serialise(msg)?,
            EncryptContext::Authenticated { ref shared_key } => shared_key.encrypt(msg)?,
            EncryptContext::AnonymousEncrypt { ref their_pk } => {
                their_pk.anonymously_encrypt(msg)?
            }
        })
    }
}

/// Simplifies decryption by holding the necessary context - keys to decrypt data.
/// Allows "null" decryption where data is only deserialized. See: null object pattern.
#[derive(Clone, Debug)]
pub enum DecryptContext {
    /// No encryption.
    Null,
    /// Encryption + authentication
    Authenticated { shared_key: SharedSecretKey },
    /// No message authentication. Only decrypt operation is allowed.
    AnonymousDecrypt {
        /// Our private key.
        our_pk: PublicEncryptKey,
        /// Our secret key.
        our_sk: SecretEncryptKey,
    },
}

impl Default for DecryptContext {
    /// Default is "null" encryption.
    fn default() -> Self {
        Self::null()
    }
}

impl DecryptContext {
    /// Contructs "null" encryption context which actually does no encryption.
    /// In this case data is simply serialized but not encrypted.
    pub fn null() -> Self {
        DecryptContext::Null
    }

    /// Construct crypto context that encrypts and authenticate messages.
    pub fn authenticated(shared_key: SharedSecretKey) -> Self {
        DecryptContext::Authenticated { shared_key }
    }

    /// Constructs crypto context that is only meant for unauthenticated decryption.
    pub fn anonymous_decrypt(our_pk: PublicEncryptKey, our_sk: SecretEncryptKey) -> Self {
        DecryptContext::AnonymousDecrypt { our_pk, our_sk }
    }

    /// Decrypt given buffer and deserialize into structure.
    pub fn decrypt<T>(&self, msg: &[u8]) -> ::Res<T>
    where
        T: Serialize + DeserializeOwned,
    {
        Ok(match *self {
            DecryptContext::Null => deserialise(msg)?,
            DecryptContext::Authenticated { ref shared_key } => shared_key.decrypt(msg)?,
            DecryptContext::AnonymousDecrypt {
                ref our_pk,
                ref our_sk,
            } => our_sk.anonymously_decrypt(msg, our_pk)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use safe_crypto::gen_encrypt_keypair;

    #[test]
    fn null_encryption_serializes_and_deserializes_data() {
        let enc_ctx = EncryptContext::null();
        let dec_ctx = DecryptContext::null();

        let encrypted = unwrap!(enc_ctx.encrypt(b"test123"));
        let decrypted: [u8; 7] = unwrap!(dec_ctx.decrypt(&encrypted[..]));

        assert_eq!(&decrypted, b"test123");
    }

    #[test]
    fn authenticated_encryption_encrypts_and_decrypts_data() {
        let (pk1, sk1) = gen_encrypt_keypair();
        let (pk2, sk2) = gen_encrypt_keypair();
        let enc_ctx = EncryptContext::authenticated(sk1.shared_secret(&pk2));
        let dec_ctx = DecryptContext::authenticated(sk2.shared_secret(&pk1));

        let encrypted = unwrap!(enc_ctx.encrypt(b"test123"));
        let decrypted: [u8; 7] = unwrap!(dec_ctx.decrypt(&encrypted[..]));

        assert_eq!(&decrypted, b"test123");
    }

    #[test]
    fn anonymous_encryption() {
        let (pk1, sk1) = gen_encrypt_keypair();
        let enc_ctx = EncryptContext::anonymous_encrypt(pk1);
        let dec_ctx = DecryptContext::anonymous_decrypt(pk1, sk1);

        let encrypted = unwrap!(enc_ctx.encrypt(b"test123"));
        let decrypted: [u8; 7] = unwrap!(dec_ctx.decrypt(&encrypted[..]));

        assert_eq!(&decrypted, b"test123");
    }
}
