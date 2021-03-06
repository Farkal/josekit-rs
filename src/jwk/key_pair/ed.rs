use std::fmt::Display;
use std::ops::Deref;

use anyhow::bail;
use once_cell::sync::Lazy;
use openssl::pkey::{PKey, Private};
use serde_json::Value;

use crate::der::oid::ObjectIdentifier;
use crate::der::{DerBuilder, DerReader, DerType};
use crate::jose::JoseError;
use crate::jwk::{Jwk, KeyPair};
use crate::util;

static OID_ED25519: Lazy<ObjectIdentifier> =
    Lazy::new(|| ObjectIdentifier::from_slice(&[1, 3, 101, 112]));

static OID_ED448: Lazy<ObjectIdentifier> =
    Lazy::new(|| ObjectIdentifier::from_slice(&[1, 3, 101, 113]));

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub enum EdCurve {
    Ed25519,
    Ed448,
}

impl EdCurve {
    pub fn name(&self) -> &str {
        match self {
            Self::Ed25519 => "Ed25519",
            Self::Ed448 => "Ed448",
        }
    }

    pub fn oid(&self) -> &ObjectIdentifier {
        match self {
            Self::Ed25519 => &*OID_ED25519,
            Self::Ed448 => &*OID_ED448,
        }
    }
}

impl Display for EdCurve {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        fmt.write_str(self.name())
    }
}

#[derive(Debug, Clone)]
pub struct EdKeyPair {
    private_key: PKey<Private>,
    curve: EdCurve,
    alg: Option<String>,
}

impl EdKeyPair {
    pub fn set_algorithm(&mut self, value: Option<&str>) {
        self.alg = value.map(|val| val.to_string());
    }

    pub(crate) fn into_private_key(self) -> PKey<Private> {
        self.private_key
    }

    pub fn curve(&self) -> EdCurve {
        self.curve
    }

    /// Generate a Ed keypair
    ///
    /// # Arguments
    /// * `curve` - EdDSA curve algorithm
    pub fn generate(curve: EdCurve) -> Result<Self, JoseError> {
        (|| -> anyhow::Result<Self> {
            let private_key = match curve {
                EdCurve::Ed25519 => PKey::generate_ed25519()?,
                EdCurve::Ed448 => PKey::generate_ed448()?,
            };

            Ok(Self {
                curve,
                private_key,
                alg: None,
            })
        })()
        .map_err(|err| JoseError::InvalidKeyFormat(err))
    }

    /// Create a EdDSA key pair from a private key that is a DER encoded PKCS#8 PrivateKeyInfo.
    ///
    /// # Arguments
    /// * `input` - A private key that is a DER encoded PKCS#8 PrivateKeyInfo.
    /// * `curve` - EC curve
    pub fn from_der(input: impl AsRef<[u8]>, curve: Option<EdCurve>) -> Result<Self, JoseError> {
        (|| -> anyhow::Result<Self> {
            let (pkcs8_der, curve) = match Self::detect_pkcs8(input.as_ref(), false) {
                Some(val) => match curve {
                    Some(val2) if val2 == val => (input.as_ref(), val),
                    Some(val2) => bail!("The curve is mismatched: {}", val2),
                    None => (input.as_ref(), val),
                },
                None => bail!("The EdDSA private key must be wrapped by PKCS#8 format."),
            };

            let private_key = PKey::private_key_from_der(pkcs8_der)?;

            Ok(Self {
                private_key,
                curve,
                alg: None,
            })
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidKeyFormat(err),
        })
    }

    /// Create a EdDSA key pair from a private key of common or traditinal PEM format.
    ///
    /// Common PEM format is a DER and base64 encoded PKCS#8 PrivateKeyInfo
    /// that surrounded by "-----BEGIN/END PRIVATE KEY----".
    ///
    /// Traditional PEM format is a DER and base64 encoded PKCS#8 PrivateKeyInfo
    /// that surrounded by "-----BEGIN/END ED25519/ED448 PRIVATE KEY----".
    ///
    /// # Arguments
    /// * `input` - A private key of common or traditinal PEM format.
    /// * `curve` - EC curve
    pub fn from_pem(input: impl AsRef<[u8]>, curve: Option<EdCurve>) -> Result<Self, JoseError> {
        (|| -> anyhow::Result<Self> {
            let (alg, data) = util::parse_pem(input.as_ref())?;
            let (pkcs8_der, curve) = match alg.as_str() {
                "PRIVATE KEY" => match EdKeyPair::detect_pkcs8(&data, false) {
                    Some(val) => match curve {
                        Some(val2) if val2 == val => (data.as_slice(), val),
                        Some(val2) => bail!("The curve is mismatched: {}", val2),
                        None => (data.as_slice(), val),
                    },
                    None => bail!("The EdDSA private key must be wrapped by PKCS#8 format."),
                },
                "ED25519 PRIVATE KEY" => match EdKeyPair::detect_pkcs8(&data, false) {
                    Some(val) => {
                        if val == EdCurve::Ed25519 {
                            match curve {
                                Some(val2) if val2 == val => (data.as_slice(), val),
                                Some(val2) => bail!("The curve is mismatched: {}", val2),
                                None => (data.as_slice(), val),
                            }
                        } else {
                            bail!("The EdDSA curve is mismatched: {}", val.name());
                        }
                    }
                    None => bail!("The EdDSA private key must be wrapped by PKCS#8 format."),
                },
                "ED448 PRIVATE KEY" => match EdKeyPair::detect_pkcs8(&data, false) {
                    Some(val) => {
                        if val == EdCurve::Ed448 {
                            match curve {
                                Some(val2) if val2 == val => (data.as_slice(), val),
                                Some(val2) => bail!("The curve is mismatched: {}", val2),
                                None => (data.as_slice(), val),
                            }
                        } else {
                            bail!("The EdDSA curve is mismatched: {}", val.name());
                        }
                    }
                    None => bail!("The EdDSA private key must be wrapped by PKCS#8 format."),
                },
                alg => bail!("Inappropriate algorithm: {}", alg),
            };

            let private_key = PKey::private_key_from_der(pkcs8_der)?;

            Ok(Self {
                private_key,
                curve,
                alg: None,
            })
        })()
        .map_err(|err| JoseError::InvalidKeyFormat(err))
    }

    /// Create a EdDSA key pair from a private key that is formatted by a JWK of OKP type.
    ///
    /// # Arguments
    /// * `jwk` - A private key that is formatted by a JWK of OKP type.
    /// * `curve` - EdDSA curve
    pub fn from_jwk(jwk: &Jwk, curve: Option<EdCurve>) -> Result<Self, JoseError> {
        (|| -> anyhow::Result<Self> {
            match jwk.key_type() {
                val if val == "OKP" => {}
                val => bail!("A parameter kty must be OKP: {}", val),
            }
            let curve = match jwk.parameter("crv") {
                Some(Value::String(val)) => match curve {
                    Some(val2) if val2.name() == val => val2,
                    Some(val2) => bail!("The curve is mismatched: {}", val2),
                    None => match val.as_str() {
                        "Ed25519" => EdCurve::Ed25519,
                        "Ed448" => EdCurve::Ed448,
                        _ => bail!("A parameter crv is unrecognized: {}", val),
                    },
                },
                Some(_) => bail!("A parameter crv must be a string."),
                None => bail!("A parameter crv is required."),
            };
            let d = match jwk.parameter("d") {
                Some(Value::String(val)) => base64::decode_config(val, base64::URL_SAFE_NO_PAD)?,
                Some(_) => bail!("A parameter d must be a string."),
                None => bail!("A parameter d is required."),
            };

            let mut builder = DerBuilder::new();
            builder.append_octed_string_from_slice(&d);

            let pkcs8 = Self::to_pkcs8(&builder.build(), false, curve);
            let private_key = PKey::private_key_from_der(&pkcs8)?;

            Ok(Self {
                private_key,
                curve,
                alg: None,
            })
        })()
        .map_err(|err| JoseError::InvalidKeyFormat(err))
    }

    pub fn to_traditional_pem_private_key(&self) -> Vec<u8> {
        let der = self.private_key.private_key_to_der().unwrap();
        let der = base64::encode_config(&der, base64::STANDARD);
        let alg = match self.curve {
            EdCurve::Ed25519 => "ED25519 PRIVATE KEY",
            EdCurve::Ed448 => "ED448 PRIVATE KEY",
        };

        let mut result = String::new();
        result.push_str("-----BEGIN ");
        result.push_str(alg);
        result.push_str("-----\r\n");
        for i in 0..((der.len() + 64 - 1) / 64) {
            result.push_str(&der[(i * 64)..std::cmp::min((i + 1) * 64, der.len())]);
            result.push_str("\r\n");
        }
        result.push_str("-----END ");
        result.push_str(alg);
        result.push_str("-----\r\n");

        result.into_bytes()
    }

    fn to_jwk(&self, private: bool, public: bool) -> Jwk {
        let mut jwk = Jwk::new("OKP");
        jwk.set_key_use("sig");
        jwk.set_key_operations({
            let mut key_ops = Vec::new();
            if private {
                key_ops.push("sign");
            }
            if public {
                key_ops.push("verify");
            }
            key_ops
        });
        if let Some(val) = &self.alg {
            jwk.set_algorithm(val);
        }
        jwk.set_parameter("crv", Some(Value::String(self.curve.name().to_string())))
            .unwrap();

        if private {
            let private_der = self.private_key.private_key_to_der().unwrap();

            let mut reader = DerReader::from_bytes(&private_der);

            match reader.next() {
                Ok(Some(DerType::Sequence)) => {}
                _ => unreachable!("Invalid private key."),
            }

            match reader.next() {
                Ok(Some(DerType::Integer)) => {
                    if reader.to_u8().unwrap() != 0 {
                        unreachable!("Invalid private key.");
                    }
                }
                _ => unreachable!("Invalid private key."),
            }

            match reader.next() {
                Ok(Some(DerType::Sequence)) => {}
                _ => unreachable!("Invalid private key."),
            }

            match reader.next() {
                Ok(Some(DerType::ObjectIdentifier)) => {
                    if &reader.to_object_identifier().unwrap() != self.curve.oid() {
                        unreachable!("Invalid private key.");
                    }
                }
                _ => unreachable!("Invalid private key."),
            }

            match reader.next() {
                Ok(Some(DerType::EndOfContents)) => {}
                _ => unreachable!("Invalid private key."),
            }

            let d = match reader.next() {
                Ok(Some(DerType::OctetString)) => {
                    let private_key = reader.contents().unwrap();
                    let mut reader = DerReader::from_bytes(&private_key);
                    match reader.next() {
                        Ok(Some(DerType::OctetString)) => {
                            let d = reader.contents().unwrap();
                            base64::encode_config(d, base64::URL_SAFE_NO_PAD)
                        }
                        _ => unreachable!("Invalid private key."),
                    }
                }
                _ => unreachable!("Invalid private key."),
            };

            jwk.set_parameter("d", Some(Value::String(d))).unwrap();
        }
        if public {
            let public_der = self.private_key.public_key_to_der().unwrap();
            let mut reader = DerReader::from_bytes(&public_der);

            match reader.next() {
                Ok(Some(DerType::Sequence)) => {}
                _ => unreachable!("Invalid private key."),
            }

            match reader.next() {
                Ok(Some(DerType::Sequence)) => {}
                _ => unreachable!("Invalid private key."),
            }

            match reader.next() {
                Ok(Some(DerType::ObjectIdentifier)) => {
                    if &reader.to_object_identifier().unwrap() != self.curve.oid() {
                        unreachable!("Invalid private key.");
                    }
                }
                _ => unreachable!("Invalid private key."),
            }

            match reader.next() {
                Ok(Some(DerType::EndOfContents)) => {}
                _ => unreachable!("Invalid private key."),
            }

            let x = match reader.next() {
                Ok(Some(DerType::BitString)) => {
                    if let (x, 0) = reader.to_bit_vec().unwrap() {
                        base64::encode_config(x, base64::URL_SAFE_NO_PAD)
                    } else {
                        unreachable!("Invalid private key.")
                    }
                }
                _ => unreachable!("Invalid private key."),
            };

            jwk.set_parameter("x", Some(Value::String(x))).unwrap();
        }

        jwk
    }

    pub(crate) fn detect_pkcs8(input: &[u8], is_public: bool) -> Option<EdCurve> {
        let curve;
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
                curve = match reader.next() {
                    Ok(Some(DerType::ObjectIdentifier)) => match reader.to_object_identifier() {
                        Ok(val) if val == *OID_ED25519 => EdCurve::Ed25519,
                        Ok(val) if val == *OID_ED448 => EdCurve::Ed448,
                        _ => return None,
                    },
                    _ => return None,
                }
            }
        }

        Some(curve)
    }

    pub(crate) fn to_pkcs8(input: &[u8], is_public: bool, curve: EdCurve) -> Vec<u8> {
        let mut builder = DerBuilder::new();
        builder.begin(DerType::Sequence);
        {
            if !is_public {
                builder.append_integer_from_u8(0);
            }

            builder.begin(DerType::Sequence);
            {
                builder.append_object_identifier(curve.oid());
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

impl KeyPair for EdKeyPair {
    fn algorithm(&self) -> Option<&str> {
        match &self.alg {
            Some(val) => Some(val.as_str()),
            None => None,
        }
    }

    fn to_der_private_key(&self) -> Vec<u8> {
        self.private_key.private_key_to_der().unwrap()
    }

    fn to_der_public_key(&self) -> Vec<u8> {
        self.private_key.public_key_to_der().unwrap()
    }

    fn to_pem_private_key(&self) -> Vec<u8> {
        self.private_key.private_key_to_pem_pkcs8().unwrap()
    }

    fn to_pem_public_key(&self) -> Vec<u8> {
        self.private_key.public_key_to_pem().unwrap()
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

impl Deref for EdKeyPair {
    type Target = dyn KeyPair;

    fn deref(&self) -> &Self::Target {
        self
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use crate::jwk::{EdCurve, EdKeyPair};

    #[test]
    fn test_ed_jwt() -> Result<()> {
        for curve in vec![EdCurve::Ed25519, EdCurve::Ed448] {
            let keypair1 = EdKeyPair::generate(curve)?;
            let der_private1 = keypair1.to_der_private_key();
            let der_public1 = keypair1.to_der_public_key();

            let jwk_keypair1 = keypair1.to_jwk_keypair();

            let keypair2 = EdKeyPair::from_jwk(&jwk_keypair1, Some(curve))?;
            let der_private2 = keypair2.to_der_private_key();
            let der_public2 = keypair2.to_der_public_key();

            assert_eq!(der_private1, der_private2);
            assert_eq!(der_public1, der_public2);
        }

        Ok(())
    }
}
