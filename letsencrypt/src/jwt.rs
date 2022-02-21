// use serde::{Deserialize, Serialize};

// use crate::acc::AcmeKey;
// use crate::cert::EC_GROUP_P256;
// use crate::util::base64url;

use p256::{ecdsa::{SigningKey, signature::{Signer}}};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::LetsEncryptError;


#[derive(Serialize, Deserialize)]
pub struct JWS {
    pub protected: String,
    pub payload: String,
    pub signature: String,
}

#[derive(Serialize, Deserialize)]
pub struct JWSProtected {
    pub alg: String, // we should always use ES256
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jwk: Option<ESJWK>,
    pub nonce: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kid: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct ESJWK {
    // we don't want serde to try to deserialize the key
    // #[serde(skip)]
    // pub alg: String,
    pub crv: String,
    pub kty: String,
    pub x: String,
    pub y: String,
    // #[serde(rename = "use")]
    // pub _use: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
// LEXICAL ORDER OF FIELDS MATTER!
pub struct JwkThumbprint {
    pub crv: String,
    pub kty: String,
    pub x: String,
    pub y: String,
}

impl JwkThumbprint {
    pub fn to_key_authorizaiton(&self, token: &str) -> Result<String, LetsEncryptError> {
        let jwk_json = serde_json::to_string(self)?;
        let digest = Sha256::digest(jwk_json.as_bytes());
        let digest_base64 = base64_url::encode(&digest);
        Ok(format!("{}.{}", token, digest_base64))
    }
}

impl From<SigningKey> for ESJWK {
    fn from(key: SigningKey) -> Self {
        let point = key.verifying_key().to_encoded_point(false);

        ESJWK {
            // alg: "ES256".into(),
            kty: "EC".into(),
            crv: "P-256".into(),
            // _use: "sig".into(),
            x: base64_url::encode(&point.x().unwrap().to_vec()),
            y: base64_url::encode(&point.y().unwrap().to_vec()),
        }
    }
}

impl From<ESJWK> for JwkThumbprint {
    fn from(jwk: ESJWK) -> Self {
        Self {
            crv: jwk.crv.clone(),
            kty: jwk.kty.clone(),
            x: jwk.x.clone(),
            y: jwk.y.clone(),
        }
    }
}

pub fn get_jwt<T: Serialize>(key: &SigningKey, protected: &JWSProtected, payload: &T) -> Result<JWS, LetsEncryptError> {
    let protected = base64_url::encode(&serde_json::to_string(protected)?);
    let payload = {
        let payload = &serde_json::to_string(payload)?;
        if payload == "\"\"" {
            "".to_string()
        } else {
            base64_url::encode(payload)
        }
    };

    let to_sign = format!("{}.{}", protected, payload);
    let to_sign = to_sign.as_bytes();

    let signed = key.sign(&to_sign);

    let r = signed.r().to_bytes();
    let s = signed.s().to_bytes();

    let mut v = Vec::with_capacity(r.len() + s.len());
    v.extend_from_slice(&r);
    v.extend_from_slice(&s);

    let signature = base64_url::encode(&v);

    Ok(JWS {
        protected,
        payload,
        signature,
    })
}

pub fn generate_es256_key() -> SigningKey {
    SigningKey::random(&mut rand_core::OsRng)
}