pub mod alg;
pub mod enc;
pub mod zip;

use std::cmp::Eq;
use std::collections::{BTreeSet, BTreeMap, HashMap};
use std::fmt::{Debug, Display};
use std::io;
use std::convert::Into;
use std::borrow::Cow;

use anyhow::bail;
use once_cell::sync::Lazy;
use openssl::rand;
use serde_json::{Map, Value};

use crate::jose::{JoseError, JoseHeader};
use crate::jwk::Jwk;
use crate::util::{self, SourceValue};

pub use crate::jwe::alg::direct::DirectJweAlgorithm::Dir;

pub use crate::jwe::alg::ecdh_es::EcdhEsJweAlgorithm::EcdhEs;

pub use crate::jwe::alg::aes::AesJweAlgorithm::A128Kw;
pub use crate::jwe::alg::aes::AesJweAlgorithm::A192Kw;
pub use crate::jwe::alg::aes::AesJweAlgorithm::A256Kw;

pub use crate::jwe::alg::aes_gcm::AesGcmJweAlgorithm::A128GcmKw;
pub use crate::jwe::alg::aes_gcm::AesGcmJweAlgorithm::A192GcmKw;
pub use crate::jwe::alg::aes_gcm::AesGcmJweAlgorithm::A256GcmKw;

pub use crate::jwe::alg::ecdh_es_aes::EcdhEsAesJweAlgorithm::EcdhEsA128Kw;
pub use crate::jwe::alg::ecdh_es_aes::EcdhEsAesJweAlgorithm::EcdhEsA192Kw;
pub use crate::jwe::alg::ecdh_es_aes::EcdhEsAesJweAlgorithm::EcdhEsA256Kw;

pub use crate::jwe::alg::pbes2_hmac_aes::Pbes2HmacAesJweAlgorithm::Pbes2HS256A128Kw;
pub use crate::jwe::alg::pbes2_hmac_aes::Pbes2HmacAesJweAlgorithm::Pbes2HS384A192Kw;
pub use crate::jwe::alg::pbes2_hmac_aes::Pbes2HmacAesJweAlgorithm::Pbes2HS512A256Kw;

#[allow(deprecated)]
pub use crate::jwe::alg::rsaes::RsaesJweAlgorithm::Rsa1_5;
pub use crate::jwe::alg::rsaes::RsaesJweAlgorithm::RsaOaep;
pub use crate::jwe::alg::rsaes::RsaesJweAlgorithm::RsaOaep256;

pub use crate::jwe::enc::aes_cbc_hmac::AesCbcHmacJweEncryption::A128CbcHS256;
pub use crate::jwe::enc::aes_cbc_hmac::AesCbcHmacJweEncryption::A192CbcHS384;
pub use crate::jwe::enc::aes_cbc_hmac::AesCbcHmacJweEncryption::A256CbcHS512;

pub use crate::jwe::enc::aes_gcm::AesGcmJweEncryption::A128Gcm;
pub use crate::jwe::enc::aes_gcm::AesGcmJweEncryption::A192Gcm;
pub use crate::jwe::enc::aes_gcm::AesGcmJweEncryption::A256Gcm;

pub use crate::jwe::zip::deflate::DeflateJweCompression::Def;

static DEFAULT_CONTEXT: Lazy<JweContext> = Lazy::new(|| JweContext::new());

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct JweContext {
    acceptable_criticals: BTreeSet<String>,
    compressions: BTreeMap<String, Box<dyn JweCompression>>,
    content_encryptions: BTreeMap<String, Box<dyn JweContentEncryption>>,
}

impl JweContext {
    pub fn new() -> Self {
        Self {
            acceptable_criticals: BTreeSet::new(),
            compressions: {
                let compressions: Vec<Box<dyn JweCompression>> = vec![Box::new(Def)];

                let mut map = BTreeMap::new();
                for compression in compressions {
                    map.insert(compression.name().to_string(), compression);
                }
                map
            },
            content_encryptions: {
                let content_encryptions: Vec<Box<dyn JweContentEncryption>> = vec![
                    Box::new(A128CbcHS256),
                    Box::new(A192CbcHS384),
                    Box::new(A256CbcHS512),
                    Box::new(A128Gcm),
                    Box::new(A192Gcm),
                    Box::new(A256Gcm),
                ];

                let mut map = BTreeMap::new();
                for content_encryption in content_encryptions {
                    map.insert(content_encryption.name().to_string(), content_encryption);
                }
                map
            },
        }
    }

    /// Test a critical header claim name is acceptable.
    ///
    /// # Arguments
    ///
    /// * `name` - a critical header claim name
    pub fn is_acceptable_critical(&self, name: &str) -> bool {
        self.acceptable_criticals.contains(name)
    }

    /// Add a acceptable critical header claim name
    ///
    /// # Arguments
    ///
    /// * `name` - a acceptable critical header claim name
    pub fn add_acceptable_critical(&mut self, name: &str) {
        self.acceptable_criticals.insert(name.to_string());
    }

    /// Remove a acceptable critical header claim name
    ///
    /// # Arguments
    ///
    /// * `name` - a acceptable critical header claim name
    pub fn remove_acceptable_critical(&mut self, name: &str) {
        self.acceptable_criticals.remove(name);
    }

    /// Get a compression algorithm for zip header claim value.
    ///
    /// # Arguments
    ///
    /// * `name` - a zip header claim name
    pub fn get_compression(&self, name: &str) -> Option<&dyn JweCompression> {
        match self.compressions.get(name) {
            Some(val) => Some(val.as_ref()),
            None => None,
        }
    }

    /// Add a compression algorithm for zip header claim name.
    ///
    /// # Arguments
    ///
    /// * `compression` - a compression algorithm
    pub fn add_compression(&mut self, compression: Box<dyn JweCompression>) {
        self.compressions
            .insert(compression.name().to_string(), compression);
    }

    /// Remove a compression algorithm for zip header claim name.
    ///
    /// # Arguments
    ///
    /// * `name` - a zip header claim name
    pub fn remove_compression(&mut self, name: &str) {
        self.compressions.remove(name);
    }

    /// Get a content encryption algorithm for enc header claim value.
    ///
    /// # Arguments
    ///
    /// * `name` - a content encryption header claim name
    pub fn get_content_encryption(&self, name: &str) -> Option<&dyn JweContentEncryption> {
        match self.content_encryptions.get(name) {
            Some(val) => Some(val.as_ref()),
            None => None,
        }
    }

    /// Add a content encryption algorithm for enc header claim name.
    ///
    /// # Arguments
    ///
    /// * `content_encryption` - a content encryption algorithm
    pub fn add_content_encryption(&mut self, content_encryption: Box<dyn JweContentEncryption>) {
        self.content_encryptions
            .insert(content_encryption.name().to_string(), content_encryption);
    }

    /// Remove a content encryption algorithm for enc header claim name.
    ///
    /// # Arguments
    ///
    /// * `name` - a enc header claim name
    pub fn remove_content_encryption(&mut self, name: &str) {
        self.content_encryptions.remove(name);
    }

    /// Return a representation of the data that is formatted by compact serialization.
    ///
    /// # Arguments
    ///
    /// * `payload` - The payload data.
    /// * `header` - The JWS heaser claims.
    /// * `encrypter` - The JWS encrypter.
    pub fn serialize_compact(
        &self,
        payload: &[u8],
        header: &JweHeader,
        encrypter: &dyn JweEncrypter,
    ) -> Result<String, JoseError> {
        self.serialize_compact_with_selector(payload, header, |_header| Some(encrypter))
    }

    /// Return a representation of the data that is formatted by compact serialization.
    ///
    /// # Arguments
    ///
    /// * `payload` - The payload data.
    /// * `header` - The JWS heaser claims.
    /// * `selector` - a function for selecting the signing algorithm.
    pub fn serialize_compact_with_selector<'a, F>(
        &self,
        payload: &[u8],
        header: &JweHeader,
        selector: F,
    ) -> Result<String, JoseError>
    where
        F: Fn(&JweHeader) -> Option<&'a dyn JweEncrypter>,
    {
        (|| -> anyhow::Result<String> {
            let encrypter = match selector(header) {
                Some(val) => val,
                None => bail!("A encrypter is not found."),
            };

            let cencryption = match header.content_encryption() {
                Some(enc) => match self.get_content_encryption(enc) {
                    Some(val) => val,
                    None => bail!("A content encryption is not registered: {}", enc),
                },
                None => bail!("A enc header claim is required."),
            };

            let compression = match header.compression() {
                Some(zip) => match self.get_compression(zip) {
                    Some(val) => Some(val),
                    None => bail!("A compression algorithm is not registered: {}", zip),
                },
                None => None,
            };

            let mut header = header.clone();
            let (key, encrypted_key) = encrypter.encrypt(cencryption.key_len(), &mut header)?;
            if let None = header.claim("kid") {
                if let Some(key_id) = encrypter.key_id() {
                    header.set_key_id(key_id);
                }
            }
            let header_bytes = serde_json::to_vec(header.claims_set())?;

            let compressed;
            let content = if let Some(compression) = compression {
                compressed = compression.compress(payload)?;
                &compressed
            } else {
                payload
            };
            
            let mut iv = vec![0; cencryption.iv_len()];
            rand::rand_bytes(&mut iv)?;

            let (ciphertext, tag) = cencryption.encrypt(&key, &iv, content, &header_bytes)?;

            let mut capacity = 4;
            capacity += util::ceiling(header_bytes.len() * 4, 3);
            if let Some(val) = &encrypted_key {
                capacity += util::ceiling(val.len() * 4, 3);
            }
            capacity += util::ceiling(iv.len() * 4, 3);
            capacity += util::ceiling(ciphertext.len() * 4, 3);
            capacity += util::ceiling(tag.len() * 4, 3);

            let mut message = String::with_capacity(capacity);
            base64::encode_config_buf(header_bytes, base64::URL_SAFE_NO_PAD, &mut message);
            message.push_str(".");
            if let Some(val) = encrypted_key {
                base64::encode_config_buf(val, base64::URL_SAFE_NO_PAD, &mut message);
            }
            message.push_str(".");
            base64::encode_config_buf(iv, base64::URL_SAFE_NO_PAD, &mut message);
            message.push_str(".");
            base64::encode_config_buf(ciphertext, base64::URL_SAFE_NO_PAD, &mut message);
            message.push_str(".");
            base64::encode_config_buf(tag, base64::URL_SAFE_NO_PAD, &mut message);

            Ok(message)
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJweFormat(err),
        })
    }

    /// Return a representation of the data that is formatted by flattened json serialization.
    ///
    /// # Arguments
    ///
    /// * `protected` - The JWE protected header claims.
    /// * `unprotected` - The JWE unprotected header claims.
    /// * `header` - The JWE unprotected header claims per recipient.
    /// * `payload` - The payload data.
    /// * `encrypter` - The JWS encrypter.
    pub fn serialize_flattened_json(
        &self,
        payload: &[u8],
        protected: Option<&JweHeader>,
        unprotected: Option<&JweHeader>,
        header: Option<&JweHeader>,
        encrypter: &dyn JweEncrypter,
    ) -> Result<String, JoseError> {
        self.serialize_flattened_json_with_selector(payload, protected, unprotected, header, |_header| {
            Some(encrypter)
        })
    }

    /// Return a representation of the data that is formatted by flatted json serialization.
    ///
    /// # Arguments
    ///
    /// * `payload` - The payload data.
    /// * `protected` - The JWS protected header claims.
    /// * `unprotected` - The JWE unprotected header claims.
    /// * `header` - The JWE unprotected header claims per recipient.
    /// * `selector` - a function for selecting the encrypting algorithm.
    pub fn serialize_flattened_json_with_selector<'a, F>(
        &self,
        payload: &[u8],
        protected: Option<&JweHeader>,
        unprotected: Option<&JweHeader>,
        header: Option<&JweHeader>,
        selector: F,
    ) -> Result<String, JoseError>
    where
        F: Fn(&JweHeader) -> Option<&'a dyn JweEncrypter>,
    {
        (|| -> anyhow::Result<String> {
            let mut protected = match protected {
                Some(val) => val.clone(),
                None => JweHeader::new(),
            };

            let mut merged_map = protected.claims_set().clone();

            if let Some(val) = unprotected {
                for (key, value) in val.claims_set() {
                    if merged_map.contains_key(key) {
                        bail!("Duplicate key exists: {}", key);
                    }
                    merged_map.insert(key.clone(), value.clone());
                }
            }

            if let Some(val) = header {
                for  (key, value) in val.claims_set() {
                    if merged_map.contains_key(key) {
                        bail!("Duplicate key exists: {}", key);
                    }
                    merged_map.insert(key.clone(), value.clone());
                }
            }

            let merged = JweHeader::from_map(merged_map)?;
            let encrypter = match selector(&merged) {
                Some(val) => val,
                None => bail!("A encrypter is not found."),
            };

            let cencryption = match merged.content_encryption() {
                Some(enc) => match self.get_content_encryption(enc) {
                    Some(val) => val,
                    None => bail!("A content encryption is not registered: {}", enc),
                },
                None => bail!("A enc header claim is required."),
            };

            let compression = match merged.compression() {
                Some(zip) => match self.get_compression(zip) {
                    Some(val) => Some(val),
                    None => bail!("A compression algorithm is not registered: {}", zip),
                },
                None => None,
            };

            let compressed;
            let content = if let Some(compression) = compression {
                compressed = compression.compress(payload)?;
                &compressed
            } else {
                payload
            };
            
            let (key, encrypted_key) = encrypter.encrypt(cencryption.key_len(), &mut protected)?;
            if let None = merged.claim("kid") {
                if let Some(key_id) = encrypter.key_id() {
                    protected.set_key_id(key_id);
                }
            }
            let protected = serde_json::to_vec(protected.claims_set())?;

            let mut iv = vec![0; cencryption.iv_len()];
            rand::rand_bytes(&mut iv)?;

            let (ciphertext, tag) = cencryption.encrypt(&key, &iv, content, &protected)?;

            let mut json = String::new();
            json.push_str("{\"protected\":\"");
            base64::encode_config_buf(&protected, base64::URL_SAFE_NO_PAD, &mut json);
            json.push_str("\"");

            if let Some(val) = unprotected {
                let unprotected = serde_json::to_string(val.claims_set())?;
                json.push_str(",\"unprotected\":");
                json.push_str(&unprotected);
            }

            if let Some(val) = header {
                let header = serde_json::to_string(val.claims_set())?;
                json.push_str(",\"header\":");
                json.push_str(&header);
            }

            json.push_str(",\"encrypted_key\":\"");
            if let Some(val) = encrypted_key {
                base64::encode_config_buf(&val, base64::URL_SAFE_NO_PAD, &mut json);
            }
            json.push_str("\"");

            json.push_str(",\"aad\":\""); //TODO
            base64::encode_config_buf(&protected, base64::URL_SAFE_NO_PAD, &mut json);
            json.push_str("\"");

            json.push_str(",\"iv\":\"");
            base64::encode_config_buf(&iv, base64::URL_SAFE_NO_PAD, &mut json);
            json.push_str("\"");

            json.push_str(",\"ciphertext\":\"");
            base64::encode_config_buf(&ciphertext, base64::URL_SAFE_NO_PAD, &mut json);
            json.push_str("\"");

            json.push_str(",\"tag\":\"");
            base64::encode_config_buf(&tag, base64::URL_SAFE_NO_PAD, &mut json);
            json.push_str("\"}");

            Ok(json)
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJweFormat(err),
        })
    }

    /// Deserialize the input that is formatted by compact serialization.
    ///
    /// # Arguments
    ///
    /// * `input` - The input data.
    /// * `decrypter` - The JWS decrypter.
    pub fn deserialize_compact(
        &self,
        input: &str,
        decrypter: &dyn JweDecrypter,
    ) -> Result<(Vec<u8>, JweHeader), JoseError> {
        self.deserialize_compact_with_selector(input, |_header| Ok(Some(decrypter)))
    }

    /// Deserialize the input that is formatted by compact serialization.
    ///
    /// # Arguments
    ///
    /// * `input` - The input data.
    /// * `selector` - a function for selecting the decrypting algorithm.
    pub fn deserialize_compact_with_selector<'a, F>(
        &self,
        input: &str,
        selector: F,
    ) -> Result<(Vec<u8>, JweHeader), JoseError>
    where
        F: Fn(&JweHeader) -> Result<Option<&'a dyn JweDecrypter>, JoseError>,
    {
        (|| -> anyhow::Result<(Vec<u8>, JweHeader)> {
            let indexies: Vec<usize> = input
                .char_indices()
                .filter(|(_, c)| c == &'.')
                .map(|(i, _)| i)
                .collect();
            if indexies.len() != 4 {
                bail!(
                    "The compact serialization form of JWE must be five parts separated by colon."
                );
            }

            let header_b64 = &input[0..indexies[0]];
            let encrypted_key_b64 = &input[(indexies[0] + 1)..(indexies[1])];
            let iv_b64 = &input[(indexies[1] + 1)..(indexies[2])];
            let ciphertext_b64 = &input[(indexies[2] + 1)..(indexies[3])];
            let tag_b64 = &input[(indexies[3] + 1)..];

            let header_bytes = base64::decode_config(header_b64, base64::URL_SAFE_NO_PAD)?;
            let header: Map<String, Value> = serde_json::from_slice(&header_bytes)?;
            let header = JweHeader::from_map(header)?;

            let decrypter = match selector(&header)? {
                Some(val) => val,
                None => bail!("A decrypter is not found."),
            };

            let cencryption = match header.content_encryption() {
                Some(enc) => match self.get_content_encryption(enc) {
                    Some(val) => val,
                    None => bail!("A content encryption is not registered: {}", enc),
                },
                None => bail!("A enc header claim is required."),
            };

            let compression = match header.compression() {
                Some(zip) => match self.get_compression(zip) {
                    Some(val) => Some(val),
                    None => bail!("A compression algorithm is not registered: {}", zip),
                },
                None => None,
            };

            match header.algorithm() {
                Some(val) => {
                    let expected_alg = decrypter.algorithm().name();
                    if val != expected_alg {
                        bail!("The JWE alg header claim is not {}: {}", expected_alg, val);
                    }
                }
                None => bail!("The JWE alg header claim is required."),
            }

            match decrypter.key_id() {
                Some(expected) => match header.key_id() {
                    Some(actual) if expected == actual => {}
                    Some(actual) => bail!("The JWE kid header claim is mismatched: {}", actual),
                    None => bail!("The JWE kid header claim is required."),
                },
                None => {}
            }

            let encrypted_key = base64::decode_config(encrypted_key_b64, base64::URL_SAFE_NO_PAD)?;
            let key = decrypter.decrypt(&header, &encrypted_key)?;

            let iv = base64::decode_config(iv_b64, base64::URL_SAFE_NO_PAD)?;
            let ciphertext = base64::decode_config(ciphertext_b64, base64::URL_SAFE_NO_PAD)?;
            let tag = base64::decode_config(tag_b64, base64::URL_SAFE_NO_PAD)?;

            let content = cencryption.decrypt(&key, &iv, &ciphertext, &header_bytes, &tag)?;
            let content = match compression {
                Some(val) => val.decompress(&content)?,
                None => content,
            };

            Ok((content, header))
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJweFormat(err),
        })
    }

    /// Deserialize the input that is formatted by flattened json serialization.
    ///
    /// # Arguments
    ///
    /// * `input` - The input data.
    /// * `header` - The decoded JWS header claims.
    /// * `decrypter` - The JWE decrypter.
    pub fn deserialize_json<'a>(
        &self,
        input: &str,
        decrypter: &'a dyn JweDecrypter,
    ) -> Result<(Vec<u8>, JweHeader), JoseError> {
        self.deserialize_json_with_selector(input, |header| {
            match header.algorithm() {
                Some(val) => {
                    let expected_alg = decrypter.algorithm().name();
                    if val != expected_alg {
                        return Ok(None);
                    }
                }
                _ => return Ok(None),
            }

            match decrypter.key_id() {
                Some(expected) => match header.key_id() {
                    Some(actual) if expected == actual => {}
                    _ => return Ok(None),
                },
                None => {}
            }

            Ok(Some(decrypter))
        })
    }

    /// Deserialize the input that is formatted by flattened json serialization.
    ///
    /// # Arguments
    ///
    /// * `input` - The input data.
    /// * `selector` - a function for selecting the decrypting algorithm.
    pub fn deserialize_json_with_selector<'a, F>(
        &self,
        input: &str,
        selector: F,
    ) -> Result<(Vec<u8>, JweHeader), JoseError>
    where
        F: Fn(&JweHeader) -> Result<Option<&'a dyn JweDecrypter>, JoseError>,
    {
        (|| -> anyhow::Result<(Vec<u8>, JweHeader)> {
            unimplemented!("JWE is not supported yet.");
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJweFormat(err),
        })
    }
}

/// Return a representation of the data that is formatted by compact serialization.
///
/// # Arguments
///
/// * `payload` - The payload data.
/// * `header` - The JWS heaser claims.
/// * `encrypter` - The JWS encrypter.
pub fn serialize_compact(
    payload: &[u8],
    header: &JweHeader,
    encrypter: &dyn JweEncrypter,
) -> Result<String, JoseError> {
    DEFAULT_CONTEXT.serialize_compact(payload, header, encrypter)
}

/// Return a representation of the data that is formatted by compact serialization.
///
/// # Arguments
///
/// * `payload` - The payload data.
/// * `header` - The JWS heaser claims.
/// * `selector` - a function for selecting the signing algorithm.
pub fn serialize_compact_with_selector<'a, F>(
    payload: &[u8],
    header: &JweHeader,
    selector: F,
) -> Result<String, JoseError>
where
    F: Fn(&JweHeader) -> Option<&'a dyn JweEncrypter>,
{
    DEFAULT_CONTEXT.serialize_compact_with_selector(payload, header, selector)
}

/// Return a representation of the data that is formatted by flattened json serialization.
///
/// # Arguments
///
/// * `protected` - The JWE protected header claims.
/// * `header` - The JWE unprotected header claims.
/// * `payload` - The payload data.
/// * `encrypter` - The JWS encrypter.
pub fn serialize_flattened_json(
    payload: &[u8],
    protected: Option<&JweHeader>,
    unprotected: Option<&JweHeader>,
    header: Option<&JweHeader>,
    encrypter: &dyn JweEncrypter,
) -> Result<String, JoseError> {
    DEFAULT_CONTEXT.serialize_flattened_json(payload, protected, unprotected, header, encrypter)
}

/// Return a representation of the data that is formatted by flatted json serialization.
///
/// # Arguments
///
/// * `payload` - The payload data.
/// * `protected` - The JWS protected header claims.
/// * `header` - The JWS unprotected header claims.
/// * `selector` - a function for selecting the encrypting algorithm.
pub fn serialize_flattened_json_with_selector<'a, F>(
    payload: &[u8],
    protected: Option<&JweHeader>,
    unprotected: Option<&JweHeader>,
    header: Option<&JweHeader>,
    selector: F,
) -> Result<String, JoseError>
where
    F: Fn(&JweHeader) -> Option<&'a dyn JweEncrypter>,
{
    DEFAULT_CONTEXT.serialize_flattened_json_with_selector(payload, protected, unprotected, header, selector)
}

/// Deserialize the input that is formatted by compact serialization.
///
/// # Arguments
///
/// * `input` - The input data.
/// * `decrypter` - The JWS decrypter.
pub fn deserialize_compact(
    input: &str,
    decrypter: &dyn JweDecrypter,
) -> Result<(Vec<u8>, JweHeader), JoseError> {
    DEFAULT_CONTEXT.deserialize_compact(input, decrypter)
}

/// Deserialize the input that is formatted by compact serialization.
///
/// # Arguments
///
/// * `input` - The input data.
/// * `selector` - a function for selecting the decrypting algorithm.
pub fn deserialize_compact_with_selector<'a, F>(
    input: &str,
    selector: F,
) -> Result<(Vec<u8>, JweHeader), JoseError>
where
    F: Fn(&JweHeader) -> Result<Option<&'a dyn JweDecrypter>, JoseError>,
{
    DEFAULT_CONTEXT.deserialize_compact_with_selector(input, selector)
}

/// Deserialize the input that is formatted by flattened json serialization.
///
/// # Arguments
///
/// * `input` - The input data.
/// * `header` - The decoded JWS header claims.
/// * `decrypter` - The JWE decrypter.
pub fn deserialize_json<'a>(
    input: &str,
    decrypter: &'a dyn JweDecrypter,
) -> Result<(Vec<u8>, JweHeader), JoseError> {
    DEFAULT_CONTEXT.deserialize_json(input, decrypter)
}

/// Deserialize the input that is formatted by flattened json serialization.
///
/// # Arguments
///
/// * `input` - The input data.
/// * `selector` - a function for selecting the decrypting algorithm.
pub fn deserialize_json_with_selector<'a, F>(
    input: &str,
    selector: F,
) -> Result<(Vec<u8>, JweHeader), JoseError>
where
    F: Fn(&JweHeader) -> Result<Option<&'a dyn JweDecrypter>, JoseError>,
{
    DEFAULT_CONTEXT.deserialize_json_with_selector(input, selector)
}

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct JweHeader {
    claims: Map<String, Value>,
    sources: HashMap<String, SourceValue>,
}

impl JweHeader {
    /// Return a new JweHeader instance.
    pub fn new() -> Self {
        Self {
            claims: Map::new(),
            sources: HashMap::new(),
        }
    }

    /// Set a value for algorithm header claim (alg).
    ///
    /// # Arguments
    ///
    /// * `value` - a algorithm
    pub fn set_algorithm(&mut self, value: impl Into<String>) {
        let value: String = value.into();
        self.claims.insert("alg".to_string(), Value::String(value));
    }

    /// Set a value for content encryption header claim (enc).
    ///
    /// # Arguments
    ///
    /// * `value` - a content encryption
    pub fn set_content_encryption(&mut self, value: impl Into<String>) {
        let value: String = value.into();
        self.claims.insert("enc".to_string(), Value::String(value));
    }

    /// Return the value for content encryption header claim (enc).
    pub fn content_encryption(&self) -> Option<&str> {
        match self.claims.get("enc") {
            Some(Value::String(val)) => Some(val),
            None => None,
            _ => unreachable!(),
        }
    }

    /// Set a value for compression header claim (zip).
    ///
    /// # Arguments
    ///
    /// * `value` - a encryption
    pub fn set_compression(&mut self, value: impl Into<String>) {
        let value: String = value.into();
        self.claims.insert("zip".to_string(), Value::String(value));
    }

    /// Return the value for compression header claim (zip).
    pub fn compression(&self) -> Option<&str> {
        match self.claims.get("zip") {
            Some(Value::String(val)) => Some(val),
            None => None,
            _ => unreachable!(),
        }
    }

    /// Set a value for JWK set URL header claim (jku).
    ///
    /// # Arguments
    ///
    /// * `value` - a JWK set URL
    pub fn set_jwk_set_url(&mut self, value: impl Into<String>) {
        let value: String = value.into();
        self.claims.insert("jku".to_string(), Value::String(value));
    }

    /// Return the value for JWK set URL header claim (jku).
    pub fn jwk_set_url(&self) -> Option<&str> {
        match self.claims.get("jku") {
            Some(Value::String(val)) => Some(val),
            None => None,
            _ => unreachable!(),
        }
    }

    /// Set a value for JWK header claim (jwk).
    ///
    /// # Arguments
    ///
    /// * `value` - a JWK
    pub fn set_jwk(&mut self, value: Jwk) {
        let key = "jwk".to_string();
        self.claims
            .insert(key.clone(), Value::Object(value.as_ref().clone()));
        self.sources.insert(key, SourceValue::Jwk(value));
    }

    /// Return the value for JWK header claim (jwk).
    pub fn jwk(&self) -> Option<&Jwk> {
        match self.sources.get("jwk") {
            Some(SourceValue::Jwk(val)) => Some(val),
            None => None,
            _ => unreachable!(),
        }
    }

    /// Set a value for X.509 URL header claim (x5u).
    ///
    /// # Arguments
    ///
    /// * `value` - a X.509 URL
    pub fn set_x509_url(&mut self, value: impl Into<String>) {
        let value: String = value.into();
        self.claims.insert("x5u".to_string(), Value::String(value));
    }

    /// Return a value for a X.509 URL header claim (x5u).
    pub fn x509_url(&self) -> Option<&str> {
        match self.claims.get("x5u") {
            Some(Value::String(val)) => Some(val),
            None => None,
            _ => unreachable!(),
        }
    }

    /// Set values for X.509 certificate chain header claim (x5c).
    ///
    /// # Arguments
    ///
    /// * `values` - X.509 certificate chain
    pub fn set_x509_certificate_chain(&mut self, values: Vec<Vec<u8>>) {
        let key = "x5c".to_string();
        let mut vec = Vec::with_capacity(values.len());
        for val in &values {
            vec.push(Value::String(base64::encode_config(
                &val,
                base64::URL_SAFE_NO_PAD,
            )));
        }
        self.claims.insert(key.clone(), Value::Array(vec));
        self.sources.insert(key, SourceValue::BytesArray(values));
    }

    /// Return values for a X.509 certificate chain header claim (x5c).
    pub fn x509_certificate_chain(&self) -> Option<&Vec<Vec<u8>>> {
        match self.sources.get("x5c") {
            Some(SourceValue::BytesArray(val)) => Some(val),
            None => None,
            _ => unreachable!(),
        }
    }

    /// Set a value for X.509 certificate SHA-1 thumbprint header claim (x5t).
    ///
    /// # Arguments
    ///
    /// * `value` - A X.509 certificate SHA-1 thumbprint
    pub fn set_x509_certificate_sha1_thumbprint(&mut self, value: Vec<u8>) {
        let key = "x5t".to_string();
        let val = base64::encode_config(&value, base64::URL_SAFE_NO_PAD);
        self.claims.insert(key.clone(), Value::String(val));
        self.sources.insert(key, SourceValue::Bytes(value));
    }

    /// Return the value for X.509 certificate SHA-1 thumbprint header claim (x5t).
    pub fn x509_certificate_sha1_thumbprint(&self) -> Option<&Vec<u8>> {
        match self.sources.get("x5t") {
            Some(SourceValue::Bytes(val)) => Some(val),
            None => None,
            _ => unreachable!(),
        }
    }

    /// Set a value for a x509 certificate SHA-256 thumbprint header claim (x5t#S256).
    ///
    /// # Arguments
    ///
    /// * `value` - A x509 certificate SHA-256 thumbprint
    pub fn set_x509_certificate_sha256_thumbprint(&mut self, value: Vec<u8>) {
        let key = "x5t#S256".to_string();
        let val = base64::encode_config(&value, base64::URL_SAFE_NO_PAD);

        self.claims.insert(key.clone(), Value::String(val));
        self.sources.insert(key, SourceValue::Bytes(value));
    }

    /// Return the value for X.509 certificate SHA-256 thumbprint header claim (x5t#S256).
    pub fn x509_certificate_sha256_thumbprint(&self) -> Option<&Vec<u8>> {
        match self.sources.get("x5t#S256") {
            Some(SourceValue::Bytes(val)) => Some(val),
            None => None,
            _ => unreachable!(),
        }
    }

    /// Set a value for key ID header claim (kid).
    ///
    /// # Arguments
    ///
    /// * `value` - a key ID
    pub fn set_key_id(&mut self, value: impl Into<String>) {
        let value: String = value.into();
        self.claims.insert("kid".to_string(), Value::String(value));
    }

    /// Return the value for key ID header claim (kid).
    pub fn key_id(&self) -> Option<&str> {
        match self.claims.get("kid") {
            Some(Value::String(val)) => Some(val),
            _ => None,
        }
    }

    /// Set a value for token type header claim (typ).
    ///
    /// # Arguments
    ///
    /// * `value` - a token type (e.g. "JWT")
    pub fn set_token_type(&mut self, value: impl Into<String>) {
        let value: String = value.into();
        self.claims.insert("typ".to_string(), Value::String(value));
    }

    /// Return the value for token type header claim (typ).
    pub fn token_type(&self) -> Option<&str> {
        match self.claims.get("typ") {
            Some(Value::String(val)) => Some(val),
            _ => None,
        }
    }

    /// Set a value for content type header claim (cty).
    ///
    /// # Arguments
    ///
    /// * `value` - a content type (e.g. "JWT")
    pub fn set_content_type(&mut self, value: impl Into<String>) {
        let value: String = value.into();
        self.claims.insert("cty".to_string(), Value::String(value));
    }

    /// Return the value for content type header claim (cty).
    pub fn content_type(&self) -> Option<&str> {
        match self.claims.get("cty") {
            Some(Value::String(val)) => Some(val),
            _ => None,
        }
    }

    /// Set values for critical header claim (crit).
    ///
    /// # Arguments
    ///
    /// * `values` - critical claim names
    pub fn set_critical(&mut self, values: Vec<impl Into<String>>) {
        let key = "crit".to_string();
        let mut vec1 = Vec::with_capacity(values.len());
        let mut vec2 = Vec::with_capacity(values.len());
        for val in values {
            let val: String = val.into();
            vec1.push(Value::String(val.clone()));
            vec2.push(val);
        }
        self.claims.insert(key.clone(), Value::Array(vec1));
        self.sources.insert(key, SourceValue::StringArray(vec2));
    }

    /// Return values for critical header claim (crit).
    pub fn critical(&self) -> Option<&Vec<String>> {
        match self.sources.get("crit") {
            Some(SourceValue::StringArray(val)) => Some(val),
            None => None,
            _ => unreachable!(),
        }
    }
}

impl JoseHeader for JweHeader {
    fn from_map(claims: Map<String, Value>) -> Result<Self, JoseError> {
        Ok(Self {
            claims,
            sources: HashMap::new(),
        })
    }

    fn claims_set(&self) -> &Map<String, Value> {
        &self.claims
    }

    fn set_claim(&mut self, key: &str, value: Option<Value>) -> Result<(), JoseError> {
        (|| -> anyhow::Result<()> {
            match key {
                "alg" | "enc" | "zip" | "jku" | "x5u" | "kid" | "typ" | "cty" => match &value {
                    Some(Value::String(_)) => {
                        self.claims.insert(key.to_string(), value.unwrap());
                    }
                    None => {
                        self.claims.remove(key);
                    }
                    _ => bail!("The JWE {} header claim must be string.", key),
                },
                "jwk" => match &value {
                    Some(Value::Object(vals)) => {
                        let key = key.to_string();
                        let val = Jwk::from_map(vals.clone())?;
                        self.claims.insert(key.clone(), value.unwrap());
                        self.sources.insert(key, SourceValue::Jwk(val));
                    }
                    None => {
                        self.claims.remove(key);
                        self.sources.remove(key);
                    }
                    _ => bail!("The JWE {} header claim must be a string.", key),
                },
                "x5t" => match &value {
                    Some(Value::String(val)) => {
                        let key = key.to_string();
                        let val = base64::decode_config(val, base64::URL_SAFE_NO_PAD)?;
                        self.claims.insert(key.clone(), value.unwrap());
                        self.sources.insert(key, SourceValue::Bytes(val));
                    }
                    None => {
                        self.claims.remove(key);
                        self.sources.remove(key);
                    }
                    _ => bail!("The JWE {} header claim must be a string.", key),
                },
                "x5t#S256" => match &value {
                    Some(Value::String(val)) => {
                        let key = key.to_string();
                        let val = base64::decode_config(val, base64::URL_SAFE_NO_PAD)?;
                        self.claims.insert(key.to_string(), value.unwrap());
                        self.sources.insert(key, SourceValue::Bytes(val));
                    }
                    None => {
                        self.claims.remove(key);
                        self.sources.remove(key);
                    }
                    _ => bail!("The JWE {} header claim must be a string.", key),
                },
                "x5c" => match &value {
                    Some(Value::Array(vals)) => {
                        let key = key.to_string();
                        let mut vec = Vec::with_capacity(vals.len());
                        for val in vals {
                            match val {
                                Value::String(val) => {
                                    let decoded =
                                        base64::decode_config(val, base64::URL_SAFE_NO_PAD)?;
                                    vec.push(decoded);
                                }
                                _ => bail!(
                                    "An element of the JWE {} header claim must be a string.",
                                    key
                                ),
                            }
                        }
                        self.claims.insert(key.clone(), value.unwrap());
                        self.sources.insert(key, SourceValue::BytesArray(vec));
                    }
                    None => {
                        self.claims.remove(key);
                        self.sources.remove(key);
                    }
                    _ => bail!("The JWE {} header claim must be a array.", key),
                },
                "crit" => match &value {
                    Some(Value::Array(vals)) => {
                        let key = key.to_string();
                        let mut vec = Vec::with_capacity(vals.len());
                        for val in vals {
                            match val {
                                Value::String(val) => vec.push(val.to_string()),
                                _ => bail!(
                                    "An element of the JWE {} header claim must be a string.",
                                    key
                                ),
                            }
                        }
                        self.claims.insert(key.to_string(), value.unwrap());
                        self.sources.insert(key, SourceValue::StringArray(vec));
                    }
                    None => {
                        self.claims.remove(key);
                        self.sources.remove(key);
                    }
                    _ => bail!("The JWE {} header claim must be a array.", key),
                },
                _ => match &value {
                    Some(_) => {
                        self.claims.insert(key.to_string(), value.unwrap());
                    }
                    None => {
                        self.claims.remove(key);
                    }
                },
            }

            Ok(())
        })()
        .map_err(|err| JoseError::InvalidJweFormat(err))
    }
}

impl AsRef<Map<String, Value>> for JweHeader {
    fn as_ref(&self) -> &Map<String, Value> {
        &self.claims
    }  
}

impl Into<Map<String, Value>> for JweHeader {
    fn into(self) -> Map<String, Value> {
        self.claims
    }
}

impl Display for JweHeader {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        let val = serde_json::to_string(self.claims_set()).map_err(|_e| std::fmt::Error {})?;
        fmt.write_str(&val)
    }
}

pub trait JweAlgorithm: Debug + Send + Sync {
    /// Return the "alg" (algorithm) header parameter value of JWE.
    fn name(&self) -> &str;

    fn box_clone(&self) -> Box<dyn JweAlgorithm>;
}

impl PartialEq for Box<dyn JweAlgorithm> {
    fn eq(&self, other: &Self) -> bool {
        self == other
    }
}

impl Eq for Box<dyn JweAlgorithm> {}

impl Clone for Box<dyn JweAlgorithm> {
    fn clone(&self) -> Self {
        self.box_clone()
    }
}

pub trait JweEncrypter: Debug + Send + Sync {
    /// Return the source algorithm instance.
    fn algorithm(&self) -> &dyn JweAlgorithm;

    /// Return the source key ID.
    /// The default value is a value of kid parameter in JWK.
    fn key_id(&self) -> Option<&str>;

    /// Set a compared value for a kid header claim (kid).
    ///
    /// # Arguments
    ///
    /// * `key_id` - a key ID
    fn set_key_id(&mut self, key_id: &str);

    /// Remove a compared value for a kid header claim (kid).
    fn remove_key_id(&mut self);

    /// Return a content encryption key and encypted data.
    /// # Arguments
    ///
    /// * `key_len` - the length of content encryption key
    /// * `header` - the header
    fn encrypt(&self, key_len: usize, header: &mut JweHeader) -> Result<(Cow<[u8]>, Option<Vec<u8>>), JoseError>;

    fn box_clone(&self) -> Box<dyn JweEncrypter>;
}

impl Clone for Box<dyn JweEncrypter> {
    fn clone(&self) -> Self {
        self.box_clone()
    }
}

pub trait JweDecrypter: Debug + Send + Sync {
    /// Return the source algorithm instance.
    fn algorithm(&self) -> &dyn JweAlgorithm;

    /// Return the source key ID.
    /// The default value is a value of kid parameter in JWK.
    fn key_id(&self) -> Option<&str>;

    /// Set a compared value for a kid header claim (kid).
    ///
    /// # Arguments
    ///
    /// * `key_id` - A key ID
    fn set_key_id(&mut self, key_id: &str);

    /// Remove a compared value for a kid header claim (kid).
    fn remove_key_id(&mut self);

    /// Return a decrypted key.
    ///
    /// # Arguments
    ///
    /// * `header` - The header
    /// * `encrypted_key` - The encrypted key.
    fn decrypt(&self, header: &JweHeader, encrypted_key: &[u8]) -> Result<Cow<[u8]>, JoseError>;
    
    fn box_clone(&self) -> Box<dyn JweDecrypter>;
}

impl Clone for Box<dyn JweDecrypter> {
    fn clone(&self) -> Self {
        self.box_clone()
    }
}

pub trait JweContentEncryption: Debug + Send + Sync {
    /// Return the "enc" (encryption) header parameter value of JWE.
    fn name(&self) -> &str;

    fn key_len(&self) -> usize;

    fn iv_len(&self) -> usize;

    fn encrypt(&self, key: &[u8], iv: &[u8], message: &[u8], aad: &[u8]) -> Result<(Vec<u8>, Vec<u8>), JoseError>;

    fn decrypt(&self, key: &[u8], iv: &[u8], encrypted_message: &[u8], aad: &[u8], tag: &[u8]) -> Result<Vec<u8>, JoseError>;

    fn box_clone(&self) -> Box<dyn JweContentEncryption>;
}

impl PartialEq for Box<dyn JweContentEncryption> {
    fn eq(&self, other: &Self) -> bool {
        self == other
    }
}

impl Eq for Box<dyn JweContentEncryption> {}

impl Clone for Box<dyn JweContentEncryption> {
    fn clone(&self) -> Self {
        self.box_clone()
    }
}

pub trait JweCompression: Debug + Send + Sync {
    /// Return the "zip" (compression algorithm) header parameter value of JWE.
    fn name(&self) -> &str;

    fn compress(&self, message: &[u8]) -> Result<Vec<u8>, io::Error>;

    fn decompress(&self, message: &[u8]) -> Result<Vec<u8>, io::Error>;

    fn box_clone(&self) -> Box<dyn JweCompression>;
}

impl PartialEq for Box<dyn JweCompression> {
    fn eq(&self, other: &Self) -> bool {
        self == other
    }
}

impl Eq for Box<dyn JweCompression> {}

impl Clone for Box<dyn JweCompression> {
    fn clone(&self) -> Self {
        self.box_clone()
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use serde_json::Value;

    use crate::jose::JoseHeader;
    use crate::jwe::{self, Dir, JweHeader, JweAlgorithm};

    #[test]
    fn test_jwe_compact_serialization() -> Result<()> {
        for enc in &["A128CBC-HS256", "A256GCM"] {
            println!("enc: {}", enc);

            let mut src_header = JweHeader::new();
            src_header.set_content_encryption(*enc);
            src_header.set_token_type("JWT");
            let src_payload = b"test payload!";
    
            let alg = Dir;
            let key = match *enc {
                "A128CBC-HS256" => b"01234567890123456789012345678901".as_ref(),
                "A128GCM" => b"0123456789012345".as_ref(),
                "A192GCM" => b"012345678901234567890123".as_ref(),
                "A256GCM" => b"01234567890123456789012345678901".as_ref(),
                _ => unreachable!(),
            };
            let encrypter = alg.encrypter_from_slice(key)?;
    
            let jwe = jwe::serialize_compact(src_payload, &src_header, &encrypter)?;

            println!("JWE: {}", jwe);
    
            let decrypter = alg.decrypter_from_slice(key)?;
            let (dst_payload, dst_header) = jwe::deserialize_compact(&jwe, &decrypter)?;
    
            src_header.set_claim("alg", Some(Value::String(alg.name().to_string())))?;
            assert_eq!(src_header, dst_header);
            assert_eq!(src_payload.to_vec(), dst_payload);
        }

        Ok(())
    }
}
