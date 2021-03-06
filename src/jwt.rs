use std::collections::HashMap;
use std::convert::Into;
use std::fmt::Display;
use std::time::{Duration, SystemTime};

use anyhow::bail;
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use serde_json::{Map, Number, Value};

use crate::jose::{JoseError, JoseHeader};
use crate::jwe::{JweContext, JweDecrypter, JweEncrypter, JweHeader};
use crate::jwk::{Jwk, JwkSet};
use crate::jws::{JwsContext, JwsHeader, JwsSigner, JwsVerifier};
use crate::util::{self, SourceValue};

static DEFAULT_CONTEXT: Lazy<JwtContext> = Lazy::new(|| JwtContext::new());

#[derive(Debug, Eq, PartialEq, Clone)]
pub struct JwtContext {
    jws_context: JwsContext,
    jwe_context: JweContext,
}

impl JwtContext {
    pub fn new() -> Self {
        Self {
            jws_context: JwsContext::new(),
            jwe_context: JweContext::new(),
        }
    }

    /// Test a critical header claim name is acceptable.
    ///
    /// # Arguments
    ///
    /// * `name` - a critical header claim name
    pub fn is_acceptable_critical(&self, name: &str) -> bool {
        self.jws_context.is_acceptable_critical(name)
    }

    /// Add a acceptable critical header claim name
    ///
    /// # Arguments
    ///
    /// * `name` - a acceptable critical header claim name
    pub fn add_acceptable_critical(&mut self, name: &str) {
        self.jws_context.add_acceptable_critical(name);
        self.jwe_context.add_acceptable_critical(name);
    }

    /// Remove a acceptable critical header claim name
    ///
    /// # Arguments
    ///
    /// * `name` - a acceptable critical header claim name
    pub fn remove_acceptable_critical(&mut self, name: &str) {
        self.jws_context.remove_acceptable_critical(name);
        self.jwe_context.remove_acceptable_critical(name);
    }

    /// Return the string repsentation of the JWT with a "none" algorithm.
    ///
    /// # Arguments
    ///
    /// * `payload` - The payload data.
    /// * `header` - The JWT heaser claims.
    pub fn encode_unsecured(
        &self,
        payload: &JwtPayload,
        header: &JwsHeader,
    ) -> Result<String, JoseError> {
        (|| -> anyhow::Result<String> {
            let mut header = header.claims_set().clone();
            header.insert("alg".to_string(), Value::String("none".to_string()));
            let header_bytes = serde_json::to_vec(&header)?;

            let payload_bytes = serde_json::to_vec(payload.claims_set())?;

            let mut capacity = 2;
            capacity += util::ceiling(header_bytes.len() * 4, 3);
            capacity += util::ceiling(payload_bytes.len() * 4, 3);

            let mut message = String::with_capacity(capacity);
            base64::encode_config_buf(header_bytes, base64::URL_SAFE_NO_PAD, &mut message);
            message.push_str(".");
            base64::encode_config_buf(payload_bytes, base64::URL_SAFE_NO_PAD, &mut message);
            message.push_str(".");

            Ok(message)
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJwtFormat(err),
        })
    }

    /// Return the string repsentation of the JWT with the siginig algorithm.
    ///
    /// # Arguments
    ///
    /// * `payload` - The payload data.
    /// * `header` - The JWS heaser claims.
    /// * `signer` - a signer object.
    pub fn encode_with_signer(
        &self,
        payload: &JwtPayload,
        header: &JwsHeader,
        signer: &dyn JwsSigner,
    ) -> Result<String, JoseError> {
        (|| -> anyhow::Result<String> {
            if let Some(vals) = header.critical() {
                if vals.iter().any(|val| val == "b64") {
                    bail!("JWT is not support b64 header claim.");
                }
            }

            let payload_bytes = serde_json::to_vec(payload.claims_set()).unwrap();
            let jwt = self
                .jws_context
                .serialize_compact(&payload_bytes, header, signer)?;
            Ok(jwt)
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJwtFormat(err),
        })
    }

    /// Return the string repsentation of the JWT with the encrypting algorithm.
    ///
    /// # Arguments
    ///
    /// * `payload` - The payload data.
    /// * `header` - The JWE heaser claims.
    /// * `encrypter` - a encrypter object.
    pub fn encode_with_encrypter(
        &self,
        payload: &JwtPayload,
        header: &JweHeader,
        encrypter: &dyn JweEncrypter,
    ) -> Result<String, JoseError> {
        let payload_bytes = serde_json::to_vec(payload.claims_set()).unwrap();
        let jwt = self
            .jwe_context
            .serialize_compact(&payload_bytes, header, encrypter)?;
        Ok(jwt)
    }

    /// Return the JWT object decoded with the "none" algorithm.
    ///
    /// # Arguments
    ///
    /// * `input` - a JWT string representation.
    pub fn decode_unsecured(&self, input: &str) -> Result<(JwtPayload, JwsHeader), JoseError> {
        (|| -> anyhow::Result<(JwtPayload, JwsHeader)> {
            let parts: Vec<&str> = input.split('.').collect();
            if parts.len() != 3 {
                bail!("The unsecured JWT must be three parts separated by colon.");
            }
            if parts[2].len() != 0 {
                bail!("The unsecured JWT must not have a signature part.");
            }

            let header = base64::decode_config(parts[0], base64::URL_SAFE_NO_PAD)?;
            let header: Map<String, Value> = serde_json::from_slice(&header)?;

            match header.get("alg") {
                Some(Value::String(val)) if val == "none" => {}
                Some(Value::String(val)) => bail!("The JWT alg header claim is not none: {}", val),
                Some(_) => bail!("The JWT alg header claim must be a string."),
                None => bail!("The JWT alg header claim is missing."),
            }

            match header.get("kid") {
                None => {}
                Some(_) => bail!("A JWT of none alg cannot have kid header claim."),
            }

            let header = JwsHeader::from_map(header)?;

            let payload = base64::decode_config(parts[1], base64::URL_SAFE_NO_PAD)?;
            let payload: Map<String, Value> = serde_json::from_slice(&payload)?;
            let payload = JwtPayload::from_map(payload)?;

            Ok((payload, header))
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJwtFormat(err),
        })
    }

    /// Return the JWT object decoded by the selected verifier.
    ///
    /// # Arguments
    ///
    /// * `verifier` - a verifier of the signing algorithm.
    /// * `input` - a JWT string representation.
    pub fn decode_with_verifier(
        &self,
        input: &str,
        verifier: &dyn JwsVerifier,
    ) -> Result<(JwtPayload, JwsHeader), JoseError> {
        self.decode_with_verifier_selector(input, |_header| Ok(Some(verifier)))
    }

    /// Return the JWT object decoded with a selected verifying algorithm.
    ///
    /// # Arguments
    ///
    /// * `input` - a JWT string representation.
    /// * `selector` - a function for selecting the verifying algorithm.
    pub fn decode_with_verifier_selector<'a, F>(
        &self,
        input: &str,
        selector: F,
    ) -> Result<(JwtPayload, JwsHeader), JoseError>
    where
        F: Fn(&JwsHeader) -> Result<Option<&'a dyn JwsVerifier>, JoseError>,
    {
        (|| -> anyhow::Result<(JwtPayload, JwsHeader)> {
            let (payload, header) =
                self.jws_context
                    .deserialize_compact_with_selector(input, |header| {
                        (|| -> anyhow::Result<Option<&'a dyn JwsVerifier>> {
                            let verifier = match selector(&header)? {
                                Some(val) => val,
                                None => return Ok(None),
                            };

                            if self.is_acceptable_critical("b64") {
                                bail!("JWT is not supported b64 header claim.");
                            }

                            Ok(Some(verifier))
                        })()
                        .map_err(|err| {
                            match err.downcast::<JoseError>() {
                                Ok(err) => err,
                                Err(err) => JoseError::InvalidJwtFormat(err),
                            }
                        })
                    })?;

            let payload: Map<String, Value> = serde_json::from_slice(&payload)?;
            let payload = JwtPayload::from_map(payload)?;

            Ok((payload, header))
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJwtFormat(err),
        })
    }

    /// Return the JWT object decoded by using a JWK set.
    ///
    /// # Arguments
    ///
    /// * `input` - a JWT string representation.
    /// * `jwk_set` - a JWK set.
    /// * `selector` - a function for selecting the verifying algorithm.
    pub fn decode_with_verifier_in_jwk_set<F>(
        &self,
        input: &str,
        jwk_set: &JwkSet,
        selector: F,
    ) -> Result<(JwtPayload, JwsHeader), JoseError>
    where
        F: Fn(&Jwk) -> Result<Option<&dyn JwsVerifier>, JoseError>,
    {
        self.decode_with_verifier_selector(input, |header| {
            let key_id = match header.key_id() {
                Some(val) => val,
                None => return Ok(None),
            };

            for jwk in jwk_set.get(key_id) {
                if let Some(val) = selector(jwk)? {
                    return Ok(Some(val));
                }
            }
            Ok(None)
        })
    }

    /// Return the JWT object decoded by the selected decrypter.
    ///
    /// # Arguments
    ///
    /// * `input` - a JWT string representation.
    /// * `decrypter` - a decrypter of the decrypting algorithm.
    pub fn decode_with_decrypter(
        &self,
        input: &str,
        decrypter: &dyn JweDecrypter,
    ) -> Result<(JwtPayload, JweHeader), JoseError> {
        self.decode_with_decrypter_selector(input, |_header| Ok(Some(decrypter)))
    }

    /// Return the JWT object decoded with a selected decrypting algorithm.
    ///
    /// # Arguments
    ///
    /// * `input` - a JWT string representation.
    /// * `decrypter_selector` - a function for selecting the decrypting algorithm.
    pub fn decode_with_decrypter_selector<'a, F>(
        &self,
        input: &str,
        selector: F,
    ) -> Result<(JwtPayload, JweHeader), JoseError>
    where
        F: Fn(&JweHeader) -> Result<Option<&'a dyn JweDecrypter>, JoseError>,
    {
        (|| -> anyhow::Result<(JwtPayload, JweHeader)> {
            let (payload, header) =
                self.jwe_context
                    .deserialize_compact_with_selector(input, |header| {
                        let decrypter = match selector(&header)? {
                            Some(val) => val,
                            None => return Ok(None),
                        };

                        Ok(Some(decrypter))
                    })?;

            let payload: Map<String, Value> = serde_json::from_slice(&payload)?;
            let payload = JwtPayload::from_map(payload)?;

            Ok((payload, header))
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJwtFormat(err),
        })
    }

    /// Return the JWT object decoded by using a JWK set.
    ///
    /// # Arguments
    ///
    /// * `input` - a JWT string representation.
    /// * `jwk_set` - a JWK set.
    /// * `selector` - a function for selecting the decrypting algorithm.
    pub fn decode_with_decrypter_in_jwk_set<F>(
        &self,
        input: &str,
        jwk_set: &JwkSet,
        selector: F,
    ) -> Result<(JwtPayload, JweHeader), JoseError>
    where
        F: Fn(&Jwk) -> Result<Option<&dyn JweDecrypter>, JoseError>,
    {
        self.decode_with_decrypter_selector(input, |header| {
            let key_id = match header.key_id() {
                Some(val) => val,
                None => return Ok(None),
            };

            for jwk in jwk_set.get(key_id) {
                if let Some(val) = selector(jwk)? {
                    return Ok(Some(val));
                }
            }
            Ok(None)
        })
    }
}

/// Return the string repsentation of the JWT with a "none" algorithm.
///
/// # Arguments
///
/// * `payload` - The payload data.
/// * `header` - The JWT heaser claims.
pub fn encode_unsecured(payload: &JwtPayload, header: &JwsHeader) -> Result<String, JoseError> {
    DEFAULT_CONTEXT.encode_unsecured(payload, header)
}

/// Return the string repsentation of the JWT with the siginig algorithm.
///
/// # Arguments
///
/// * `payload` - The payload data.
/// * `header` - The JWS heaser claims.
/// * `signer` - a signer object.
pub fn encode_with_signer(
    payload: &JwtPayload,
    header: &JwsHeader,
    signer: &dyn JwsSigner,
) -> Result<String, JoseError> {
    DEFAULT_CONTEXT.encode_with_signer(payload, header, signer)
}

/// Return the string repsentation of the JWT with the encrypting algorithm.
///
/// # Arguments
///
/// * `payload` - The payload data.
/// * `header` - The JWE heaser claims.
/// * `encrypter` - a encrypter object.
pub fn encode_with_encrypter(
    payload: &JwtPayload,
    header: &JweHeader,
    encrypter: &dyn JweEncrypter,
) -> Result<String, JoseError> {
    DEFAULT_CONTEXT.encode_with_encrypter(payload, header, encrypter)
}

/// Return the JWT object decoded with the "none" algorithm.
///
/// # Arguments
///
/// * `input` - a JWT string representation.
pub fn decode_unsecured(input: &str) -> Result<(JwtPayload, JwsHeader), JoseError> {
    DEFAULT_CONTEXT.decode_unsecured(input)
}

/// Return the JWT object decoded by the selected verifier.
///
/// # Arguments
///
/// * `verifier` - a verifier of the signing algorithm.
/// * `input` - a JWT string representation.
pub fn decode_with_verifier(
    input: &str,
    verifier: &dyn JwsVerifier,
) -> Result<(JwtPayload, JwsHeader), JoseError> {
    DEFAULT_CONTEXT.decode_with_verifier(input, verifier)
}

/// Return the JWT object decoded with a selected verifying algorithm.
///
/// # Arguments
///
/// * `input` - a JWT string representation.
/// * `selector` - a function for selecting the verifying algorithm.
pub fn decode_with_verifier_selector<'a, F>(
    input: &str,
    selector: F,
) -> Result<(JwtPayload, JwsHeader), JoseError>
where
    F: Fn(&JwsHeader) -> Result<Option<&'a dyn JwsVerifier>, JoseError>,
{
    DEFAULT_CONTEXT.decode_with_verifier_selector(input, selector)
}

/// Return the JWT object decoded by using a JWK set.
///
/// # Arguments
///
/// * `input` - a JWT string representation.
/// * `jwk_set` - a JWK set.
/// * `selector` - a function for selecting the verifying algorithm.
pub fn decode_with_verifier_in_jwk_set<F>(
    input: &str,
    jwk_set: &JwkSet,
    selector: F,
) -> Result<(JwtPayload, JwsHeader), JoseError>
where
    F: Fn(&Jwk) -> Result<Option<&dyn JwsVerifier>, JoseError>,
{
    DEFAULT_CONTEXT.decode_with_verifier_in_jwk_set(input, jwk_set, selector)
}

/// Return the JWT object decoded by the selected decrypter.
///
/// # Arguments
///
/// * `input` - a JWT string representation.
/// * `decrypter` - a decrypter of the decrypting algorithm.
pub fn decode_with_decrypter(
    input: &str,
    decrypter: &dyn JweDecrypter,
) -> Result<(JwtPayload, JweHeader), JoseError> {
    DEFAULT_CONTEXT.decode_with_decrypter(input, decrypter)
}

/// Return the JWT object decoded with a selected decrypting algorithm.
///
/// # Arguments
///
/// * `input` - a JWT string representation.
/// * `decrypter_selector` - a function for selecting the decrypting algorithm.
pub fn decode_with_decrypter_selector<'a, F>(
    input: &str,
    selector: F,
) -> Result<(JwtPayload, JweHeader), JoseError>
where
    F: Fn(&JweHeader) -> Result<Option<&'a dyn JweDecrypter>, JoseError>,
{
    DEFAULT_CONTEXT.decode_with_decrypter_selector(input, selector)
}

/// Return the JWT object decoded by using a JWK set.
///
/// # Arguments
///
/// * `input` - a JWT string representation.
/// * `jwk_set` - a JWK set.
/// * `selector` - a function for selecting the decrypting algorithm.
pub fn decode_with_decrypter_in_jwk_set<F>(
    input: &str,
    jwk_set: &JwkSet,
    selector: F,
) -> Result<(JwtPayload, JweHeader), JoseError>
where
    F: Fn(&Jwk) -> Result<Option<&dyn JweDecrypter>, JoseError>,
{
    DEFAULT_CONTEXT.decode_with_decrypter_in_jwk_set(input, jwk_set, selector)
}

#[derive(Debug, Eq, PartialEq, Clone, Default)]
pub struct JwtPayload {
    claims: Map<String, Value>,
    sources: HashMap<String, SourceValue>,
}

impl JwtPayload {
    /// Return a new JWT payload
    pub fn new() -> Self {
        Self {
            claims: Map::new(),
            sources: HashMap::new(),
        }
    }

    /// Return the JWT payload from map.
    ///
    /// # Arguments
    ///
    /// * `claims` - JWT payload claims.
    pub fn from_map(claims: Map<String, Value>) -> Result<Self, JoseError> {
        (|| -> anyhow::Result<Self> {
            let mut sources = HashMap::new();
            for (key, value) in &claims {
                match key.as_ref() {
                    "iss" | "sub" | "jti" => match value {
                        Value::String(_) => {}
                        _ => bail!("The JWT {} payload claim must be a string.", key),
                    },
                    "aud" => match value {
                        Value::String(_) => {}
                        Value::Array(vals) => {
                            let mut vec = Vec::with_capacity(vals.len());
                            for val in vals {
                                match val {
                                    Value::String(val) => vec.push(val.to_string()),
                                    _ => bail!(
                                        "An element of JWT {} payload claim must be a string.",
                                        key
                                    ),
                                }
                            }
                            sources.insert(key.clone(), SourceValue::StringArray(vec));
                        }
                        _ => bail!("The JWT {} payload claim must be a string or array.", key),
                    },
                    "exp" | "nbf" | "iat" => match value {
                        Value::Number(val) => match val.as_u64() {
                            Some(val) => {
                                let val = SystemTime::UNIX_EPOCH + Duration::from_secs(val);
                                sources.insert(key.clone(), SourceValue::SystemTime(val));
                            }
                            None => bail!(
                                "The JWT {} payload claim must be a positive integer within 64bit.",
                                key
                            ),
                        },
                        _ => bail!("The JWT {} payload claim must be a string type.", key),
                    },
                    _ => {}
                }
            }

            Ok(Self { claims, sources })
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidJwtFormat(err),
        })
    }

    /// Set a value for issuer payload claim (iss).
    ///
    /// # Arguments
    ///
    /// * `value` - a issuer
    pub fn set_issuer(&mut self, value: impl Into<String>) {
        let value: String = value.into();
        self.claims.insert("iss".to_string(), Value::String(value));
    }

    /// Return the value for issuer payload claim (iss).
    pub fn issuer(&self) -> Option<&str> {
        match self.claims.get("iss") {
            Some(Value::String(val)) => Some(val),
            _ => None,
        }
    }

    /// Set a value for subject payload claim (sub).
    ///
    /// # Arguments
    ///
    /// * `value` - a subject
    pub fn set_subject(&mut self, value: impl Into<String>) {
        let value: String = value.into();
        self.claims.insert("sub".to_string(), Value::String(value));
    }

    /// Return the value for subject payload claim (sub).
    pub fn subject(&self) -> Option<&str> {
        match self.claims.get("sub") {
            Some(Value::String(val)) => Some(val),
            _ => None,
        }
    }

    /// Set values for audience payload claim (aud).
    ///
    /// # Arguments
    ///
    /// * `values` - a list of audiences
    pub fn set_audience(&mut self, values: Vec<impl Into<String>>) {
        let key = "aud".to_string();
        if values.len() == 1 {
            for val in values {
                let val: String = val.into();
                self.sources.remove(&key);
                self.claims.insert(key, Value::String(val));
                break;
            }
        } else if values.len() > 1 {
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
    }

    /// Return values for audience payload claim (aud).
    pub fn audience(&self) -> Option<&Vec<String>> {
        match self.sources.get("aud") {
            Some(SourceValue::StringArray(val)) => Some(val),
            None => None,
            _ => unreachable!(),
        }
    }

    /// Set a system time for expires at payload claim (exp).
    ///
    /// # Arguments
    ///
    /// * `value` - A expiration time on or after which the JWT must not be accepted for processing.
    pub fn set_expires_at(&mut self, value: SystemTime) {
        let key = "exp".to_string();
        let val = Number::from(
            value
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        );
        self.claims.insert(key.clone(), Value::Number(val));
        self.sources.insert(key, SourceValue::SystemTime(value));
    }

    /// Return the system time for expires at payload claim (exp).
    pub fn expires_at(&self) -> Option<&SystemTime> {
        match self.sources.get("exp") {
            Some(SourceValue::SystemTime(val)) => Some(val),
            None => None,
            _ => unreachable!(),
        }
    }

    /// Set a system time for not before payload claim (nbf).
    ///
    /// # Arguments
    ///
    /// * `value` - A time before which the JWT must not be accepted for processing.
    pub fn set_not_before(&mut self, value: SystemTime) {
        let key = "nbf".to_string();
        let val = Number::from(
            value
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        );
        self.claims.insert(key.clone(), Value::Number(val));
        self.sources.insert(key, SourceValue::SystemTime(value));
    }

    /// Return the system time for not before payload claim (nbf).
    pub fn not_before(&self) -> Option<&SystemTime> {
        match self.sources.get("nbf") {
            Some(SourceValue::SystemTime(val)) => Some(val),
            None => None,
            _ => unreachable!(),
        }
    }

    /// Set a time for issued at payload claim (iat).
    ///
    /// # Arguments
    ///
    /// * `value` - a time at which the JWT was issued.
    pub fn set_issued_at(&mut self, value: SystemTime) {
        let key = "iat".to_string();
        let val = Number::from(
            value
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        );
        self.claims.insert(key.clone(), Value::Number(val));
        self.sources.insert(key, SourceValue::SystemTime(value));
    }

    /// Return the time for a issued at payload claim (iat).
    pub fn issued_at(&self) -> Option<&SystemTime> {
        match self.sources.get("iat") {
            Some(SourceValue::SystemTime(val)) => Some(val),
            None => None,
            _ => unreachable!(),
        }
    }

    /// Set a value for JWT ID payload claim (jti).
    ///
    /// # Arguments
    ///
    /// * `value` - a JWT ID
    pub fn set_jwt_id(&mut self, value: impl Into<String>) {
        let value: String = value.into();
        self.claims.insert("jti".to_string(), Value::String(value));
    }

    /// Return the value for JWT ID payload claim (jti).
    pub fn jwt_id(&self) -> Option<&str> {
        match self.claims.get("jti") {
            Some(Value::String(val)) => Some(val),
            _ => None,
        }
    }

    /// Set a value for payload claim of a specified key.
    ///
    /// # Arguments
    ///
    /// * `key` - a key name of payload claim
    /// * `value` - a typed value of payload claim
    pub fn set_claim(&mut self, key: &str, value: Option<Value>) -> Result<(), JoseError> {
        (|| -> anyhow::Result<()> {
            match key {
                "iss" | "sub" | "jti" => match &value {
                    Some(Value::String(_)) => {
                        self.claims.insert(key.to_string(), value.unwrap());
                    }
                    None => {
                        self.claims.remove(key);
                    }
                    _ => bail!("The JWT {} payload claim must be a string.", key),
                },
                "aud" => match &value {
                    Some(Value::String(_)) => {
                        let key = key.to_string();
                        self.sources.remove(&key);
                        self.claims.insert(key, value.unwrap());
                    }
                    Some(Value::Array(vals)) => {
                        let key = key.to_string();
                        let mut vec = Vec::with_capacity(vals.len());
                        for val in vals {
                            match val {
                                Value::String(val) => vec.push(val.to_string()),
                                _ => bail!(
                                    "An element of the JWT {} payload claim must be a string.",
                                    key
                                ),
                            }
                        }
                        self.sources
                            .insert(key.clone(), SourceValue::StringArray(vec));
                        self.claims.insert(key, value.unwrap());
                    }
                    None => {
                        self.sources.remove(key);
                        self.claims.remove(key);
                    }
                    _ => bail!("The JWT {} payload claim must be a string or array.", key),
                },
                "exp" | "nbf" | "iat" => match &value {
                    Some(Value::Number(val)) => match val.as_u64() {
                        Some(val) => {
                            let key = key.to_string();
                            let val = SystemTime::UNIX_EPOCH + Duration::from_secs(val);
                            self.sources
                                .insert(key.clone(), SourceValue::SystemTime(val));
                            self.claims.insert(key, value.unwrap());
                        }
                        None => bail!(
                            "The JWT {} payload claim must be a positive integer within 64bit.",
                            key
                        ),
                    },
                    None => {
                        self.sources.remove(key);
                        self.claims.remove(key);
                    }
                    _ => bail!("The JWT {} header claim must be a string.", key),
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
        .map_err(|err| JoseError::InvalidJwtFormat(err))
    }

    /// Return a value for payload claim of a specified key.
    ///
    /// # Arguments
    ///
    /// * `key` - a key name of payload claim
    pub fn claim(&self, key: &str) -> Option<&Value> {
        self.claims.get(key)
    }

    /// Return values for payload claims set
    pub fn claims_set(&self) -> &Map<String, Value> {
        &self.claims
    }
}

impl AsRef<Map<String, Value>> for JwtPayload {
    fn as_ref(&self) -> &Map<String, Value> {
        &self.claims
    }
}

impl Into<Map<String, Value>> for JwtPayload {
    fn into(self) -> Map<String, Value> {
        self.claims
    }
}

impl Display for JwtPayload {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        let val = serde_json::to_string(self.claims_set()).map_err(|_e| std::fmt::Error {})?;
        fmt.write_str(&val)
    }
}

/// Represents JWT payload validator.
#[derive(Debug, Eq, PartialEq)]
pub struct JwtPayloadValidator {
    base_time: Option<SystemTime>,
    min_issued_time: Option<SystemTime>,
    max_issued_time: Option<SystemTime>,
    audience: Option<String>,
    claims: Map<String, Value>,
}

impl JwtPayloadValidator {
    /// Return a new JwtPayloadValidator.
    pub fn new() -> Self {
        Self {
            base_time: None,
            min_issued_time: None,
            max_issued_time: None,
            audience: None,
            claims: Map::new(),
        }
    }

    /// Set a base time for time related claims (exp, nbf) validation.
    ///
    /// # Arguments
    ///
    /// * `base_time` - a min time
    pub fn set_base_time(&mut self, base_time: SystemTime) {
        self.base_time = Some(base_time);
    }

    /// Return the base time for time related claims (exp, nbf) validation.
    pub fn base_time(&self) -> Option<&SystemTime> {
        self.base_time.as_ref()
    }

    /// Set a minimum time for issued at payload claim (iat) validation.
    ///
    /// # Arguments
    ///
    /// * `min_issued_time` - a minimum time at which the JWT was issued.
    pub fn set_min_issued_time(&mut self, min_issued_time: SystemTime) {
        self.min_issued_time = Some(min_issued_time);
    }

    /// Return the minimum time for issued at payload claim (iat).
    pub fn min_issued_time(&self) -> Option<&SystemTime> {
        self.min_issued_time.as_ref()
    }

    /// Set a maximum time for issued at payload claim (iat) validation.
    ///
    /// # Arguments
    ///
    /// * `max_issued_time` - a maximum time at which the JWT was issued.
    pub fn set_max_issued_time(&mut self, max_issued_time: SystemTime) {
        self.max_issued_time = Some(max_issued_time);
    }

    /// Return the maximum time for issued at payload claim (iat).
    pub fn max_issued_time(&self) -> Option<&SystemTime> {
        self.max_issued_time.as_ref()
    }

    /// Set a value for issuer payload claim (iss) validation.
    ///
    /// # Arguments
    ///
    /// * `value` - a issuer
    pub fn set_issuer(&mut self, value: impl Into<String>) {
        let value: String = value.into();
        self.claims.insert("iss".to_string(), Value::String(value));
    }

    /// Return the value for issuer payload claim (iss) validation.
    pub fn issuer(&self) -> Option<&str> {
        match self.claims.get("iss") {
            Some(Value::String(val)) => Some(val),
            _ => None,
        }
    }

    /// Set a value for subject payload claim (sub) validation.
    ///
    /// # Arguments
    ///
    /// * `value` - a subject
    pub fn set_subject(&mut self, value: impl Into<String>) {
        let value: String = value.into();
        self.claims.insert("sub".to_string(), Value::String(value));
    }

    /// Return the value for subject payload claim (sub) validation.
    pub fn subject(&self) -> Option<&str> {
        match self.claims.get("sub") {
            Some(Value::String(val)) => Some(val),
            _ => None,
        }
    }

    /// Set a value for audience payload claim (aud) validation.
    ///
    /// # Arguments
    ///
    /// * `value` - a audience
    pub fn set_audience(&mut self, value: impl Into<String>) {
        let value: String = value.into();
        self.audience = Some(value);
    }

    /// Return the value for audience payload claim (aud) validation.
    pub fn audience(&self) -> Option<&str> {
        match self.audience {
            Some(ref val) => Some(val),
            _ => None,
        }
    }

    /// Set a value for JWT ID payload claim (jti) validation.
    ///
    /// # Arguments
    ///
    /// * `value` - A JWT ID
    pub fn set_jwt_id(&mut self, value: impl Into<String>) {
        let value: String = value.into();
        self.claims.insert("jti".to_string(), Value::String(value));
    }

    /// Return the value for JWT ID payload claim (jti) validation.
    pub fn jwt_id(&self) -> Option<&str> {
        match self.claims.get("jti") {
            Some(Value::String(val)) => Some(val),
            _ => None,
        }
    }

    /// Set a value for payload claim of a specified key.
    ///
    /// # Arguments
    ///
    /// * `key` - a key name of payload claim
    /// * `value` - a typed value of payload claim
    pub fn set_claim(&mut self, key: &str, value: Value) {
        self.claims.insert(key.to_string(), value);
    }

    /// Return the value for payload claim of a specified key.
    ///
    /// # Arguments
    ///
    /// * `key` - a key name of payload claim
    pub fn claim(&self, key: &str) -> Option<&Value> {
        self.claims.get(key)
    }

    /// Validate a decoded JWT payload.
    ///
    /// # Arguments
    ///
    /// * `payload` - a decoded JWT payload.
    pub fn validate(&self, payload: &JwtPayload) -> Result<(), JoseError> {
        (|| -> anyhow::Result<()> {
            let now = SystemTime::now();
            let current_time = self.base_time().unwrap_or(&now);
            let min_issued_time = self.min_issued_time().unwrap_or(&SystemTime::UNIX_EPOCH);
            let max_issued_time = self.max_issued_time().unwrap_or(&now);

            if let Some(not_before) = payload.not_before() {
                if not_before > &current_time {
                    bail!(
                        "The token is not yet valid: {}",
                        DateTime::<Utc>::from(*not_before)
                    );
                }
            }

            if let Some(expires_at) = payload.expires_at() {
                if expires_at <= &current_time {
                    bail!(
                        "The token has expired: {}",
                        DateTime::<Utc>::from(*expires_at)
                    );
                }
            }

            if let Some(issued_at) = payload.issued_at() {
                if issued_at < &min_issued_time {
                    bail!(
                        "The issued time is too old: {}",
                        DateTime::<Utc>::from(*issued_at)
                    );
                }

                if issued_at > &max_issued_time {
                    bail!(
                        "The issued time is too new: {}",
                        DateTime::<Utc>::from(*issued_at)
                    );
                }
            }

            if let Some(audience) = &self.audience {
                if let Some(audiences) = payload.audience() {
                    if !audiences.contains(&audience) {
                        bail!("Key aud is invalid: {}", audiences.join(", "));
                    }
                }
            }

            for (key, value1) in &self.claims {
                if let Some(value2) = payload.claim(key) {
                    if value1 != value2 {
                        bail!("Key {} is invalid: {}", key, value2);
                    }
                } else {
                    bail!("Key {} is missing.", key);
                }
            }

            Ok(())
        })()
        .map_err(|err| match err.downcast::<JoseError>() {
            Ok(err) => err,
            Err(err) => JoseError::InvalidClaim(err),
        })
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use serde_json::json;
    use std::fs::File;
    use std::io::Read;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    use crate::jwk::Jwk;
    use crate::jws::{
        EdDSA, JwsHeader, ES256, ES256K, ES384, ES512, HS256, HS384, HS512, PS256, PS384, PS512,
        RS256, RS384, RS512,
    };
    use crate::jwt::{self, JwtPayload, JwtPayloadValidator};
    use crate::util;

    #[test]
    fn test_new_header() -> Result<()> {
        let mut header = JwsHeader::new();
        let jwk = Jwk::new("oct");
        header.set_jwk_set_url("jku");
        header.set_jwk(jwk.clone());
        header.set_x509_url("x5u");
        header.set_x509_certificate_chain(vec![b"x5c0".to_vec(), b"x5c1".to_vec()]);
        header.set_x509_certificate_sha1_thumbprint(b"x5t".to_vec());
        header.set_x509_certificate_sha256_thumbprint(b"x5t#S256".to_vec());
        header.set_key_id("kid");
        header.set_token_type("typ");
        header.set_content_type("cty");
        header.set_critical(vec!["crit0", "crit1"]);
        header.set_url("url");
        header.set_nonce(b"nonce".to_vec());
        header.set_claim("header_claim", Some(json!("header_claim")))?;

        assert!(matches!(header.jwk_set_url(), Some("jku")));
        assert!(matches!(header.jwk(), Some(val) if val == &jwk));
        assert!(matches!(header.x509_url(), Some("x5u")));
        assert!(
            matches!(header.x509_certificate_chain(), Some(vals) if vals == &vec![
                b"x5c0".to_vec(),
                b"x5c1".to_vec(),
            ])
        );
        assert!(
            matches!(header.x509_certificate_sha1_thumbprint(), Some(val) if val == &b"x5t".to_vec())
        );
        assert!(
            matches!(header.x509_certificate_sha256_thumbprint(), Some(val) if val == &b"x5t#S256".to_vec())
        );
        assert!(matches!(header.key_id(), Some("kid")));
        assert!(matches!(header.token_type(), Some("typ")));
        assert!(matches!(header.content_type(), Some("cty")));
        assert!(matches!(header.url(), Some("url")));
        assert!(matches!(header.nonce(), Some(val) if val == &b"nonce".to_vec()));
        assert!(matches!(header.critical(), Some(vals) if vals == &vec!["crit0", "crit1"]));
        assert!(matches!(header.claim("header_claim"), Some(val) if val == &json!("header_claim")));

        Ok(())
    }

    #[test]
    fn test_new_payload() -> Result<()> {
        let mut payload = JwtPayload::new();
        payload.set_issuer("iss");
        payload.set_subject("sub");
        payload.set_audience(vec!["aud0", "aud1"]);
        payload.set_expires_at(SystemTime::UNIX_EPOCH);
        payload.set_not_before(SystemTime::UNIX_EPOCH);
        payload.set_issued_at(SystemTime::UNIX_EPOCH);
        payload.set_jwt_id("jti");
        payload.set_claim("payload_claim", Some(json!("payload_claim")))?;

        assert!(matches!(payload.issuer(), Some("iss")));
        assert!(matches!(payload.subject(), Some("sub")));
        assert!(matches!(payload.audience(), Some(vals) if vals == &vec!["aud0", "aud1"]));
        assert!(matches!(payload.expires_at(), Some(val) if val == &SystemTime::UNIX_EPOCH));
        assert!(matches!(payload.not_before(), Some(val) if val == &SystemTime::UNIX_EPOCH));
        assert!(matches!(payload.issued_at(), Some(val) if val == &SystemTime::UNIX_EPOCH));
        assert!(matches!(payload.jwt_id(), Some("jti")));
        assert!(
            matches!(payload.claim("payload_claim"), Some(val) if val == &json!("payload_claim"))
        );

        Ok(())
    }

    #[test]
    fn test_jwt_unsecured() -> Result<()> {
        let mut src_header = JwsHeader::new();
        src_header.set_token_type("JWT");
        let src_payload = JwtPayload::new();
        let jwt_string = jwt::encode_unsecured(&src_payload, &src_header)?;
        let (dst_payload, dst_header) = jwt::decode_unsecured(&jwt_string)?;

        src_header.set_claim("alg", Some(json!("none")))?;
        assert_eq!(src_header, dst_header);
        assert_eq!(src_payload, dst_payload);

        Ok(())
    }

    #[test]
    fn test_jwt_with_hmac() -> Result<()> {
        for alg in &[HS256, HS384, HS512] {
            let private_key = util::rand_bytes(64);

            let mut src_header = JwsHeader::new();
            src_header.set_token_type("JWT");
            let src_payload = JwtPayload::new();
            let signer = alg.signer_from_slice(&private_key)?;
            let jwt_string = jwt::encode_with_signer(&src_payload, &src_header, &signer)?;

            let verifier = alg.verifier_from_slice(&private_key)?;
            let (dst_payload, dst_header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            src_header.set_claim("alg", Some(json!(alg.name())))?;
            assert_eq!(src_header, dst_header);
            assert_eq!(src_payload, dst_payload);
        }

        Ok(())
    }

    #[test]
    fn test_jwt_with_rsa_pem() -> Result<()> {
        for alg in &[RS256, RS384, RS512] {
            let private_key = load_file("pem/RSA_2048bit_private.pem")?;
            let public_key = load_file("pem/RSA_2048bit_public.pem")?;

            let mut src_header = JwsHeader::new();
            src_header.set_token_type("JWT");
            let src_payload = JwtPayload::new();
            let signer = alg.signer_from_pem(&private_key)?;
            let jwt_string = jwt::encode_with_signer(&src_payload, &src_header, &signer)?;

            let verifier = alg.verifier_from_pem(&public_key)?;
            let (dst_payload, dst_header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            src_header.set_claim("alg", Some(json!(alg.name())))?;
            assert_eq!(src_header, dst_header);
            assert_eq!(src_payload, dst_payload);
        }

        Ok(())
    }

    #[test]
    fn test_jwt_with_rsapss_pem() -> Result<()> {
        for alg in &[PS256, PS384, PS512] {
            let private_key = load_file(match alg.name() {
                "PS256" => "pem/RSA-PSS_2048bit_SHA-256_private.pem",
                "PS384" => "pem/RSA-PSS_2048bit_SHA-384_private.pem",
                "PS512" => "pem/RSA-PSS_2048bit_SHA-512_private.pem",
                _ => unreachable!(),
            })?;
            let public_key = load_file(match alg.name() {
                "PS256" => "pem/RSA-PSS_2048bit_SHA-256_public.pem",
                "PS384" => "pem/RSA-PSS_2048bit_SHA-384_public.pem",
                "PS512" => "pem/RSA-PSS_2048bit_SHA-512_public.pem",
                _ => unreachable!(),
            })?;

            let mut src_header = JwsHeader::new();
            src_header.set_token_type("JWT");
            let src_payload = JwtPayload::new();
            let signer = alg.signer_from_pem(&private_key)?;
            let jwt_string = jwt::encode_with_signer(&src_payload, &src_header, &signer)?;

            let verifier = alg.verifier_from_pem(&public_key)?;
            let (dst_payload, dst_header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            src_header.set_claim("alg", Some(json!(alg.name())))?;
            assert_eq!(src_header, dst_header);
            assert_eq!(src_payload, dst_payload);
        }

        Ok(())
    }

    #[test]
    fn test_jwt_with_rsa_der() -> Result<()> {
        for alg in &[RS256, RS384, RS512] {
            let private_key = load_file("der/RSA_2048bit_pkcs8_private.der")?;
            let public_key = load_file("der/RSA_2048bit_spki_public.der")?;

            let mut src_header = JwsHeader::new();
            src_header.set_token_type("JWT");
            let src_payload = JwtPayload::new();
            let signer = alg.signer_from_der(&private_key)?;
            let jwt_string = jwt::encode_with_signer(&src_payload, &src_header, &signer)?;

            let verifier = alg.verifier_from_der(&public_key)?;
            let (dst_payload, dst_header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            src_header.set_claim("alg", Some(json!(alg.name())))?;
            assert_eq!(src_header, dst_header);
            assert_eq!(src_payload, dst_payload);
        }

        Ok(())
    }

    #[test]
    fn test_jwt_with_ecdsa_pem() -> Result<()> {
        for alg in &[ES256, ES384, ES512, ES256K] {
            let private_key = load_file(match alg {
                ES256 => "pem/EC_P-256_private.pem",
                ES384 => "pem/EC_P-384_private.pem",
                ES512 => "pem/EC_P-521_private.pem",
                ES256K => "pem/EC_secp256k1_private.pem",
            })?;
            let public_key = load_file(match alg {
                ES256 => "pem/EC_P-256_public.pem",
                ES384 => "pem/EC_P-384_public.pem",
                ES512 => "pem/EC_P-521_public.pem",
                ES256K => "pem/EC_secp256k1_public.pem",
            })?;

            let mut src_header = JwsHeader::new();
            src_header.set_token_type("JWT");
            let src_payload = JwtPayload::new();
            let signer = alg.signer_from_pem(&private_key)?;
            let jwt_string = jwt::encode_with_signer(&src_payload, &src_header, &signer)?;

            let verifier = alg.verifier_from_pem(&public_key)?;
            let (dst_payload, dst_header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            src_header.set_claim("alg", Some(json!(alg.name())))?;
            assert_eq!(src_header, dst_header);
            assert_eq!(src_payload, dst_payload);
        }

        Ok(())
    }

    #[test]
    fn test_jwt_with_ecdsa_der() -> Result<()> {
        for alg in &[ES256, ES384, ES512, ES256K] {
            let private_key = load_file(match alg {
                ES256 => "der/EC_P-256_pkcs8_private.der",
                ES384 => "der/EC_P-384_pkcs8_private.der",
                ES512 => "der/EC_P-521_pkcs8_private.der",
                ES256K => "der/EC_secp256k1_pkcs8_private.der",
            })?;
            let public_key = load_file(match alg {
                ES256 => "der/EC_P-256_spki_public.der",
                ES384 => "der/EC_P-384_spki_public.der",
                ES512 => "der/EC_P-521_spki_public.der",
                ES256K => "der/EC_secp256k1_spki_public.der",
            })?;

            let mut src_header = JwsHeader::new();
            src_header.set_token_type("JWT");
            let src_payload = JwtPayload::new();
            let signer = alg.signer_from_der(&private_key)?;
            let jwt_string = jwt::encode_with_signer(&src_payload, &src_header, &signer)?;

            let verifier = alg.verifier_from_der(&public_key)?;
            let (dst_payload, dst_header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            src_header.set_claim("alg", Some(json!(alg.name())))?;
            assert_eq!(src_header, dst_header);
            assert_eq!(src_payload, dst_payload);
        }

        Ok(())
    }

    #[test]
    fn test_jwt_payload_validate() -> Result<()> {
        let mut payload = JwtPayload::new();
        payload.set_issuer("iss");
        payload.set_subject("sub");
        payload.set_audience(vec!["aud0", "aud1"]);
        payload.set_expires_at(SystemTime::UNIX_EPOCH + Duration::from_secs(60));
        payload.set_not_before(SystemTime::UNIX_EPOCH + Duration::from_secs(10));
        payload.set_issued_at(SystemTime::UNIX_EPOCH);
        payload.set_jwt_id("jti");
        payload.set_claim("payload_claim", Some(json!("payload_claim")))?;

        let mut validator = JwtPayloadValidator::new();
        validator.set_base_time(SystemTime::UNIX_EPOCH + Duration::from_secs(30));
        validator.set_issuer("iss");
        validator.set_audience("aud1");
        validator.set_claim("payload_claim", json!("payload_claim"));
        validator.validate(&payload)?;

        Ok(())
    }

    #[test]
    fn test_external_jwt_verify_with_hmac() -> Result<()> {
        let jwk = Jwk::from_slice(&load_file("jwk/oct_private.jwk")?)?;

        for alg in &[HS256, HS384, HS512] {
            let verifier = alg.verifier_from_jwk(&jwk)?;
            let jwt_string = String::from_utf8(load_file(&format!("jwt/{}.jwt", alg.name()))?)?;
            let (payload, header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            assert_eq!(header.algorithm(), Some(verifier.algorithm().name()));
            assert_eq!(payload.issuer(), Some("joe"));
            assert_eq!(
                payload.expires_at(),
                Some(&(SystemTime::UNIX_EPOCH + Duration::from_secs(1300819380)))
            );
            assert_eq!(
                payload.claim("http://example.com/is_root"),
                Some(&json!(true))
            );
        }

        Ok(())
    }

    #[test]
    fn test_external_jwt_verify_with_rsa() -> Result<()> {
        let jwk = Jwk::from_slice(&load_file("jwk/RSA_public.jwk")?)?;

        for alg in &[RS256, RS384, RS512] {
            let verifier = alg.verifier_from_jwk(&jwk)?;
            let jwt_string = String::from_utf8(load_file(&format!("jwt/{}.jwt", alg.name()))?)?;
            let (payload, header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            assert_eq!(header.algorithm(), Some(verifier.algorithm().name()));
            assert_eq!(payload.issuer(), Some("joe"));
            assert_eq!(
                payload.expires_at(),
                Some(&(SystemTime::UNIX_EPOCH + Duration::from_secs(1300819380)))
            );
            assert_eq!(
                payload.claim("http://example.com/is_root"),
                Some(&json!(true))
            );
        }

        Ok(())
    }

    #[test]
    fn test_external_jwt_verify_with_rsapss() -> Result<()> {
        let jwk = Jwk::from_slice(&load_file("jwk/RSA_public.jwk")?)?;

        for alg in &[PS256, PS384, PS512] {
            let verifier = alg.verifier_from_jwk(&jwk)?;
            let jwt_string = String::from_utf8(load_file(&format!("jwt/{}.jwt", alg.name()))?)?;
            let (payload, header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            assert_eq!(header.algorithm(), Some(verifier.algorithm().name()));
            assert_eq!(payload.issuer(), Some("joe"));
            assert_eq!(
                payload.expires_at(),
                Some(&(SystemTime::UNIX_EPOCH + Duration::from_secs(1300819380)))
            );
            assert_eq!(
                payload.claim("http://example.com/is_root"),
                Some(&json!(true))
            );
        }

        Ok(())
    }

    #[test]
    fn test_external_jwt_verify_with_ecdsa() -> Result<()> {
        for alg in &[ES256, ES384, ES512, ES256K] {
            let jwk = Jwk::from_slice(&load_file(match alg {
                ES256 => "jwk/EC_P-256_public.jwk",
                ES384 => "jwk/EC_P-384_public.jwk",
                ES512 => "jwk/EC_P-521_public.jwk",
                ES256K => "jwk/EC_secp256k1_public.jwk",
            })?)?;
            let verifier = alg.verifier_from_jwk(&jwk)?;
            let jwt_string = String::from_utf8(load_file(&format!("jwt/{}.jwt", alg.name()))?)?;
            let (payload, header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            assert_eq!(header.algorithm(), Some(verifier.algorithm().name()));
            assert_eq!(payload.issuer(), Some("joe"));
            assert_eq!(
                payload.expires_at(),
                Some(&(SystemTime::UNIX_EPOCH + Duration::from_secs(1300819380)))
            );
            assert_eq!(
                payload.claim("http://example.com/is_root"),
                Some(&json!(true))
            );
        }

        Ok(())
    }

    #[test]
    fn test_external_jwt_verify_with_eddsa() -> Result<()> {
        for alg in &[EdDSA] {
            let jwk = Jwk::from_slice(&load_file(match alg {
                EdDSA => "jwk/OKP_Ed25519_public.jwk",
            })?)?;
            let verifier = alg.verifier_from_jwk(&jwk)?;
            let jwt_string = String::from_utf8(load_file(&format!("jwt/{}.jwt", alg.name()))?)?;
            let (payload, header) = jwt::decode_with_verifier(&jwt_string, &verifier)?;

            assert_eq!(header.algorithm(), Some(verifier.algorithm().name()));
            assert_eq!(payload.issuer(), Some("joe"));
            assert_eq!(
                payload.expires_at(),
                Some(&(SystemTime::UNIX_EPOCH + Duration::from_secs(1300819380)))
            );
            assert_eq!(
                payload.claim("http://example.com/is_root"),
                Some(&json!(true))
            );
        }

        Ok(())
    }

    fn load_file(path: &str) -> Result<Vec<u8>> {
        let mut pb = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        pb.push("data");
        pb.push(path);

        let mut file = File::open(&pb)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;
        Ok(data)
    }
}
