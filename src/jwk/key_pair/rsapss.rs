use std::ops::Deref;

use anyhow::bail;
use openssl::pkey::{PKey, Private};
use openssl::rsa::Rsa;
use serde_json::Value;

use crate::der::oid::{OID_MGF1, OID_RSASSA_PSS, OID_SHA256, OID_SHA384, OID_SHA512};
use crate::der::{DerBuilder, DerClass, DerReader, DerType};
use crate::jose::JoseError;
use crate::jwk::{Jwk, KeyPair};
use crate::util::{self, HashAlgorithm};

#[derive(Debug, Clone)]
pub struct RsaPssKeyPair {
    private_key: PKey<Private>,
    key_len: u32,
    hash: HashAlgorithm,
    mgf1_hash: HashAlgorithm,
    salt_len: u8,
    alg: Option<String>,
}

impl RsaPssKeyPair {
    pub fn key_len(&self) -> u32 {
        self.key_len
    }

    pub fn set_algorithm(&mut self, value: Option<&str>) {
        self.alg = value.map(|val| val.to_string());
    }

    pub(crate) fn into_private_key(self) -> PKey<Private> {
        self.private_key
    }

    /// Generate RSA key pair.
    ///
    /// # Arguments
    /// * `bits` - RSA key length
    pub fn generate(
        bits: u32,
        hash: HashAlgorithm,
        mgf1_hash: HashAlgorithm,
        salt_len: u8,
    ) -> Result<RsaPssKeyPair, JoseError> {
        (|| -> anyhow::Result<RsaPssKeyPair> {
            let rsa = Rsa::generate(bits)?;
            let key_len = rsa.size();
            let private_key = PKey::from_rsa(rsa)?;

            Ok(RsaPssKeyPair {
                private_key,
                key_len,
                hash,
                mgf1_hash,
                salt_len,
                alg: None,
            })
        })()
        .map_err(|err| JoseError::InvalidKeyFormat(err))
    }

    /// Create a RSA-PSS key pair from a private key that is a DER encoded PKCS#8 PrivateKeyInfo or PKCS#1 RSAPrivateKey.
    ///
    /// # Arguments
    /// * `input` - A private key that is a DER encoded PKCS#8 PrivateKeyInfo or PKCS#1 RSAPrivateKey.
    /// * `hash` A hash algorithm for signing
    /// * `mgf1_hash` A hash algorithm for MGF1
    /// * `salt_len` A salt length
    pub fn from_der(
        input: impl AsRef<[u8]>,
        hash: Option<HashAlgorithm>,
        mgf1_hash: Option<HashAlgorithm>,
        salt_len: Option<u8>,
    ) -> Result<Self, JoseError> {
        (|| -> anyhow::Result<Self> {
            let pkcs8_der_vec;
            let (pkcs8_der, hash, mgf1_hash, salt_len) =
                match Self::detect_pkcs8(input.as_ref(), false) {
                    Some((hash2, mgf1_hash2, salt_len2)) => {
                        let hash = match hash {
                            Some(val) if val == hash2 => hash2,
                            Some(_) => bail!("The hash algorithm is mismatched: {}", hash2),
                            None => hash2,
                        };

                        let mgf1_hash = match mgf1_hash {
                            Some(val) if val == mgf1_hash2 => mgf1_hash2,
                            Some(_) => {
                                bail!("The MGF1 hash algorithm is mismatched: {}", mgf1_hash2)
                            }
                            None => hash2,
                        };

                        let salt_len = match salt_len {
                            Some(val) if val == salt_len2 => salt_len2,
                            Some(_) => bail!("The salt length is mismatched: {}", salt_len2),
                            None => salt_len2,
                        };

                        (input.as_ref(), hash, mgf1_hash, salt_len)
                    }
                    None => {
                        let hash = match hash {
                            Some(val) => val,
                            None => bail!("The hash algorithm is required."),
                        };

                        let mgf1_hash = match mgf1_hash {
                            Some(val) => val,
                            None => bail!("The MGF1 hash algorithm is required."),
                        };

                        let salt_len = match salt_len {
                            Some(val) => val,
                            None => bail!("The salt length is required."),
                        };

                        pkcs8_der_vec =
                            Self::to_pkcs8(input.as_ref(), false, hash, mgf1_hash, salt_len);
                        (pkcs8_der_vec.as_slice(), hash, mgf1_hash, salt_len)
                    }
                };

            let private_key = PKey::private_key_from_der(pkcs8_der)?;
            let rsa = private_key.rsa()?;
            let key_len = rsa.size();

            Ok(RsaPssKeyPair {
                private_key,
                key_len,
                hash,
                mgf1_hash,
                salt_len,
                alg: None,
            })
        })()
        .map_err(|err| JoseError::InvalidKeyFormat(err))
    }

    /// Create a RSA-PSS key pair from a private key of common or traditinal PEM format.
    ///
    /// Common PEM format is a DER and base64 encoded PKCS#8 PrivateKeyInfo
    /// that surrounded by "-----BEGIN/END PRIVATE KEY----".
    ///
    /// Traditional PEM format is a DER and base64 encoded PKCS#8 PrivateKeyInfo or PKCS#1 RSAPrivateKey
    /// that surrounded by "-----BEGIN/END RSA-PSS/RSA PRIVATE KEY----".
    ///
    /// # Arguments
    /// * `input` - A private key of common or traditinal PEM format.
    /// * `hash` A hash algorithm for signing
    /// * `mgf1_hash` A hash algorithm for MGF1
    /// * `salt_len` A salt length
    pub fn from_pem(
        input: impl AsRef<[u8]>,
        hash: HashAlgorithm,
        mgf1_hash: HashAlgorithm,
        salt_len: u8,
    ) -> Result<Self, JoseError> {
        (|| -> anyhow::Result<Self> {
            let (alg, data) = util::parse_pem(input.as_ref())?;

            let private_key = match alg.as_str() {
                "PRIVATE KEY" | "RSA-PSS PRIVATE KEY" => match Self::detect_pkcs8(&data, false) {
                    Some((hash2, mgf1_hash2, salt_len2)) => {
                        if hash2 != hash {
                            bail!("The message digest parameter is mismatched: {}", hash2);
                        }

                        if mgf1_hash2 != mgf1_hash {
                            bail!(
                                "The mgf1 message digest parameter is mismatched: {}",
                                mgf1_hash2
                            );
                        }

                        if salt_len2 != salt_len {
                            bail!("The salt size is mismatched: {}", salt_len2);
                        }

                        PKey::private_key_from_der(&data)?
                    }
                    None => bail!("Invalid PEM contents."),
                },
                "RSA PRIVATE KEY" => {
                    let pkcs8 = Self::to_pkcs8(&data, false, hash, mgf1_hash, salt_len);
                    PKey::private_key_from_der(&pkcs8)?
                }
                alg => bail!("Inappropriate algorithm: {}", alg),
            };

            let rsa = private_key.rsa()?;
            let key_len = rsa.size();

            Ok(RsaPssKeyPair {
                private_key,
                key_len,
                hash,
                mgf1_hash,
                salt_len,
                alg: None,
            })
        })()
        .map_err(|err| JoseError::InvalidKeyFormat(err))
    }

    /// Create a RSA-PSS key pair from a private key that is formatted by a JWK of RSA type.
    ///
    /// # Arguments
    /// * `jwk` - A private key that is formatted by a JWK of RSA type.
    /// * `hash` A hash algorithm for signing
    /// * `mgf1_hash` A hash algorithm for MGF1
    /// * `salt_len` A salt length
    pub fn from_jwk(
        jwk: &Jwk,
        hash: HashAlgorithm,
        mgf1_hash: HashAlgorithm,
        salt_len: u8,
    ) -> Result<Self, JoseError> {
        (|| -> anyhow::Result<Self> {
            match jwk.key_type() {
                val if val == "RSA" => {}
                val => bail!("A parameter kty must be RSA: {}", val),
            }
            let n = match jwk.parameter("n") {
                Some(Value::String(val)) => base64::decode_config(val, base64::URL_SAFE_NO_PAD)?,
                Some(_) => bail!("A parameter n must be a string."),
                None => bail!("A parameter n is required."),
            };
            let e = match jwk.parameter("e") {
                Some(Value::String(val)) => base64::decode_config(val, base64::URL_SAFE_NO_PAD)?,
                Some(_) => bail!("A parameter e must be a string."),
                None => bail!("A parameter e is required."),
            };
            let d = match jwk.parameter("d") {
                Some(Value::String(val)) => base64::decode_config(val, base64::URL_SAFE_NO_PAD)?,
                Some(_) => bail!("A parameter d must be a string."),
                None => bail!("A parameter d is required."),
            };
            let p = match jwk.parameter("p") {
                Some(Value::String(val)) => base64::decode_config(val, base64::URL_SAFE_NO_PAD)?,
                Some(_) => bail!("A parameter p must be a string."),
                None => bail!("A parameter p is required."),
            };
            let q = match jwk.parameter("q") {
                Some(Value::String(val)) => base64::decode_config(val, base64::URL_SAFE_NO_PAD)?,
                Some(_) => bail!("A parameter q must be a string."),
                None => bail!("A parameter q is required."),
            };
            let dp = match jwk.parameter("dp") {
                Some(Value::String(val)) => base64::decode_config(val, base64::URL_SAFE_NO_PAD)?,
                Some(_) => bail!("A parameter dp must be a string."),
                None => bail!("A parameter dp is required."),
            };
            let dq = match jwk.parameter("dq") {
                Some(Value::String(val)) => base64::decode_config(val, base64::URL_SAFE_NO_PAD)?,
                Some(_) => bail!("A parameter dq must be a string."),
                None => bail!("A parameter dq is required."),
            };
            let qi = match jwk.parameter("qi") {
                Some(Value::String(val)) => base64::decode_config(val, base64::URL_SAFE_NO_PAD)?,
                Some(_) => bail!("A parameter qi must be a string."),
                None => bail!("A parameter qi is required."),
            };

            let mut builder = DerBuilder::new();
            builder.begin(DerType::Sequence);
            {
                builder.append_integer_from_u8(0); // version
                builder.append_integer_from_be_slice(&n, false); // n
                builder.append_integer_from_be_slice(&e, false); // e
                builder.append_integer_from_be_slice(&d, false); // d
                builder.append_integer_from_be_slice(&p, false); // p
                builder.append_integer_from_be_slice(&q, false); // q
                builder.append_integer_from_be_slice(&dp, false); // d mod (p-1)
                builder.append_integer_from_be_slice(&dq, false); // d mod (q-1)
                builder.append_integer_from_be_slice(&qi, false); // (inverse of q) mod p
            }
            builder.end();

            let pkcs8 = RsaPssKeyPair::to_pkcs8(&builder.build(), false, hash, mgf1_hash, salt_len);
            let private_key = PKey::private_key_from_der(&pkcs8)?;
            let rsa = private_key.rsa()?;
            let key_len = rsa.size();

            Ok(Self {
                private_key,
                key_len,
                hash,
                mgf1_hash,
                salt_len,
                alg: None,
            })
        })()
        .map_err(|err| JoseError::InvalidKeyFormat(err))
    }

    pub fn to_raw_private_key(&self) -> Vec<u8> {
        let rsa = self.private_key.rsa().unwrap();
        rsa.private_key_to_der().unwrap()
    }

    pub fn to_raw_public_key(&self) -> Vec<u8> {
        let rsa = self.private_key.rsa().unwrap();
        rsa.public_key_to_der_pkcs1().unwrap()
    }

    pub fn to_traditional_pem_private_key(&self) -> Vec<u8> {
        let der = self.to_der_private_key();
        let der = base64::encode_config(&der, base64::STANDARD);

        let mut result = String::new();
        result.push_str("-----BEGIN RSA-PSS PRIVATE KEY-----\r\n");
        for i in 0..((der.len() + 64 - 1) / 64) {
            result.push_str(&der[(i * 64)..std::cmp::min((i + 1) * 64, der.len())]);
            result.push_str("\r\n");
        }
        result.push_str("-----END RSA-PSS PRIVATE KEY-----\r\n");
        result.into_bytes()
    }

    fn to_jwk(&self, private: bool, _public: bool) -> Jwk {
        let rsa = self.private_key.rsa().unwrap();

        let mut jwk = Jwk::new("RSA");
        if let Some(val) = &self.alg {
            jwk.set_algorithm(val);
        }
        let n = rsa.n().to_vec();
        let n = base64::encode_config(n, base64::URL_SAFE_NO_PAD);
        jwk.set_parameter("n", Some(Value::String(n))).unwrap();

        let e = rsa.e().to_vec();
        let e = base64::encode_config(e, base64::URL_SAFE_NO_PAD);
        jwk.set_parameter("e", Some(Value::String(e))).unwrap();

        if private {
            let d = rsa.d().to_vec();
            let d = base64::encode_config(d, base64::URL_SAFE_NO_PAD);
            jwk.set_parameter("d", Some(Value::String(d))).unwrap();

            let p = rsa.p().unwrap().to_vec();
            let p = base64::encode_config(p, base64::URL_SAFE_NO_PAD);
            jwk.set_parameter("p", Some(Value::String(p))).unwrap();

            let q = rsa.q().unwrap().to_vec();
            let q = base64::encode_config(q, base64::URL_SAFE_NO_PAD);
            jwk.set_parameter("q", Some(Value::String(q))).unwrap();

            let dp = rsa.dmp1().unwrap().to_vec();
            let dp = base64::encode_config(dp, base64::URL_SAFE_NO_PAD);
            jwk.set_parameter("dp", Some(Value::String(dp))).unwrap();

            let dq = rsa.dmq1().unwrap().to_vec();
            let dq = base64::encode_config(dq, base64::URL_SAFE_NO_PAD);
            jwk.set_parameter("dq", Some(Value::String(dq))).unwrap();

            let qi = rsa.iqmp().unwrap().to_vec();
            let qi = base64::encode_config(qi, base64::URL_SAFE_NO_PAD);
            jwk.set_parameter("qi", Some(Value::String(qi))).unwrap();
        }

        jwk
    }

    pub(crate) fn detect_pkcs8(
        input: &[u8],
        is_public: bool,
    ) -> Option<(HashAlgorithm, HashAlgorithm, u8)> {
        let md;
        let mgf1_md;
        let salt_len;
        let mut reader = DerReader::from_reader(input);

        match reader.next() {
            Ok(Some(DerType::Sequence)) => {}
            _ => return None,
        }

        {
            if !is_public {
                // Version
                match reader.next() {
                    Ok(Some(DerType::Integer)) => match reader.to_u8() {
                        Ok(val) => {
                            if val != 0 {
                                return None;
                            }
                        }
                        _ => return None,
                    },
                    _ => return None,
                }
            }

            match reader.next() {
                Ok(Some(DerType::Sequence)) => {}
                _ => return None,
            }

            {
                match reader.next() {
                    Ok(Some(DerType::ObjectIdentifier)) => match reader.to_object_identifier() {
                        Ok(val) => {
                            if val != *OID_RSASSA_PSS {
                                return None;
                            }
                        }
                        _ => return None,
                    },
                    _ => return None,
                }

                match reader.next() {
                    Ok(Some(DerType::Sequence)) => {}
                    _ => return None,
                }

                {
                    match reader.next() {
                        Ok(Some(DerType::Other(DerClass::ContextSpecific, 0))) => {}
                        _ => return None,
                    }

                    match reader.next() {
                        Ok(Some(DerType::Sequence)) => {}
                        _ => return None,
                    }

                    {
                        md = match reader.next() {
                            Ok(Some(DerType::ObjectIdentifier)) => {
                                match reader.to_object_identifier() {
                                    Ok(val) if val == *OID_SHA256 => HashAlgorithm::Sha256,
                                    Ok(val) if val == *OID_SHA384 => HashAlgorithm::Sha384,
                                    Ok(val) if val == *OID_SHA512 => HashAlgorithm::Sha512,
                                    _ => return None,
                                }
                            }
                            _ => return None,
                        }
                    }

                    match reader.next() {
                        Ok(Some(DerType::EndOfContents)) => {}
                        _ => return None,
                    }

                    match reader.next() {
                        Ok(Some(DerType::Other(DerClass::ContextSpecific, 1))) => {}
                        _ => return None,
                    }

                    match reader.next() {
                        Ok(Some(DerType::Sequence)) => {}
                        _ => return None,
                    }

                    {
                        match reader.next() {
                            Ok(Some(DerType::ObjectIdentifier)) => {
                                match reader.to_object_identifier() {
                                    Ok(val) => {
                                        if val != *OID_MGF1 {
                                            return None;
                                        }
                                    }
                                    _ => return None,
                                }
                            }
                            _ => return None,
                        }

                        match reader.next() {
                            Ok(Some(DerType::Sequence)) => {}
                            _ => return None,
                        }

                        {
                            mgf1_md = match reader.next() {
                                Ok(Some(DerType::ObjectIdentifier)) => {
                                    match reader.to_object_identifier() {
                                        Ok(val) if val == *OID_SHA256 => HashAlgorithm::Sha256,
                                        Ok(val) if val == *OID_SHA384 => HashAlgorithm::Sha384,
                                        Ok(val) if val == *OID_SHA512 => HashAlgorithm::Sha512,
                                        _ => return None,
                                    }
                                }
                                _ => return None,
                            }
                        }
                    }

                    match reader.next() {
                        Ok(Some(DerType::EndOfContents)) => {}
                        _ => return None,
                    }

                    match reader.next() {
                        Ok(Some(DerType::Other(DerClass::ContextSpecific, 2))) => {}
                        _ => return None,
                    }

                    salt_len = match reader.next() {
                        Ok(Some(DerType::Integer)) => match reader.to_u8() {
                            Ok(val) => val,
                            _ => return None,
                        },
                        _ => return None,
                    }
                }
            }
        }

        Some((md, mgf1_md, salt_len))
    }

    pub(crate) fn to_pkcs8(
        input: &[u8],
        is_public: bool,
        hash: HashAlgorithm,
        mgf1_hash: HashAlgorithm,
        salt_len: u8,
    ) -> Vec<u8> {
        let mut builder = DerBuilder::new();
        builder.begin(DerType::Sequence);
        {
            if !is_public {
                builder.append_integer_from_u8(0);
            }

            builder.begin(DerType::Sequence);
            {
                builder.append_object_identifier(&OID_RSASSA_PSS);
                builder.begin(DerType::Sequence);
                {
                    builder.begin(DerType::Other(DerClass::ContextSpecific, 0));
                    {
                        builder.begin(DerType::Sequence);
                        {
                            builder.append_object_identifier(match hash {
                                HashAlgorithm::Sha256 => &OID_SHA256,
                                HashAlgorithm::Sha384 => &OID_SHA384,
                                HashAlgorithm::Sha512 => &OID_SHA512,
                            });
                        }
                        builder.end();
                    }
                    builder.end();

                    builder.begin(DerType::Other(DerClass::ContextSpecific, 1));
                    {
                        builder.begin(DerType::Sequence);
                        {
                            builder.append_object_identifier(&OID_MGF1);
                            builder.begin(DerType::Sequence);
                            {
                                builder.append_object_identifier(match mgf1_hash {
                                    HashAlgorithm::Sha256 => &OID_SHA256,
                                    HashAlgorithm::Sha384 => &OID_SHA384,
                                    HashAlgorithm::Sha512 => &OID_SHA512,
                                });
                            }
                            builder.end();
                        }
                        builder.end();
                    }
                    builder.end();

                    builder.begin(DerType::Other(DerClass::ContextSpecific, 2));
                    {
                        builder.append_integer_from_u8(salt_len);
                    }
                    builder.end();
                }
                builder.end();
            }
            builder.end();

            if is_public {
                builder.append_bit_string_from_slice(input, 0);
            } else {
                builder.append_octed_string_from_slice(input);
            }
        }
        builder.end();

        builder.build()
    }
}

impl KeyPair for RsaPssKeyPair {
    fn algorithm(&self) -> Option<&str> {
        match &self.alg {
            Some(val) => Some(val.as_str()),
            None => None,
        }
    }

    fn to_der_private_key(&self) -> Vec<u8> {
        Self::to_pkcs8(
            &self.to_raw_private_key(),
            false,
            self.hash,
            self.mgf1_hash,
            self.salt_len,
        )
    }

    fn to_der_public_key(&self) -> Vec<u8> {
        Self::to_pkcs8(
            &self.to_raw_public_key(),
            true,
            self.hash,
            self.mgf1_hash,
            self.salt_len,
        )
    }

    fn to_pem_private_key(&self) -> Vec<u8> {
        let der = self.to_der_private_key();
        let der = base64::encode_config(&der, base64::STANDARD);

        let mut result = String::new();
        result.push_str("-----BEGIN PRIVATE KEY-----\r\n");
        for i in 0..((der.len() + 64 - 1) / 64) {
            result.push_str(&der[(i * 64)..std::cmp::min((i + 1) * 64, der.len())]);
            result.push_str("\r\n");
        }
        result.push_str("-----END PRIVATE KEY-----\r\n");
        result.into_bytes()
    }

    fn to_pem_public_key(&self) -> Vec<u8> {
        let der = self.to_der_public_key();
        let der = base64::encode_config(&der, base64::STANDARD);

        let mut result = String::new();
        result.push_str("-----BEGIN PUBLIC KEY-----\r\n");
        for i in 0..((der.len() + 64 - 1) / 64) {
            result.push_str(&der[(i * 64)..std::cmp::min((i + 1) * 64, der.len())]);
            result.push_str("\r\n");
        }
        result.push_str("-----END PUBLIC KEY-----\r\n");
        result.into_bytes()
    }

    fn to_jwk_private_key(&self) -> Jwk {
        self.to_jwk(true, false)
    }

    fn to_jwk_public_key(&self) -> Jwk {
        self.to_jwk(false, true)
    }

    fn to_jwk_keypair(&self) -> Jwk {
        self.to_jwk(true, true)
    }

    fn box_clone(&self) -> Box<dyn KeyPair> {
        Box::new(self.clone())
    }
}

impl Deref for RsaPssKeyPair {
    type Target = dyn KeyPair;

    fn deref(&self) -> &Self::Target {
        self
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use crate::jwk::RsaPssKeyPair;
    use crate::util::HashAlgorithm;

    #[test]
    fn test_rsa_jwt() -> Result<()> {
        for bits in vec![1024, 2048, 4096] {
            for hash in vec![
                HashAlgorithm::Sha256,
                HashAlgorithm::Sha384,
                HashAlgorithm::Sha512,
            ] {
                let keypair1 = RsaPssKeyPair::generate(bits, hash, hash, 20)?;
                let der_private1 = keypair1.to_der_private_key();
                let der_public1 = keypair1.to_der_public_key();

                let jwk_keypair1 = keypair1.to_jwk_keypair();

                let keypair2 = RsaPssKeyPair::from_jwk(&jwk_keypair1, hash, hash, 20)?;
                let der_private2 = keypair2.to_der_private_key();
                let der_public2 = keypair2.to_der_public_key();

                assert_eq!(der_private1, der_private2);
                assert_eq!(der_public1, der_public2);
            }
        }

        Ok(())
    }
}
