use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use hyper::{Body, Method, Request, Response};
use openssl::{pkey::{PKey, Private}};
use p256::ecdsa::SigningKey;
use serde::Serialize;
use serde_json::json;

use crate::{
    directory::Directory,
    error::LetsEncryptError,
    jwt::{generate_es256_key, get_jwt, JWSProtected, ESJWK, JwkThumbprint},
    nonce::get_nonce,
    request::get_client, challenge::Http01Challenge, cert::{create_rsa_key, create_csr, Certificate}, order::{new_order, get_order}, response_debug_string,
};

#[derive(Debug)]
pub struct Account {
    pub es_key: Vec<u8>,
    pub location: String,
    pub directory: Directory,

    pub private_key: PKey<Private>
}

#[async_trait]
pub trait ServesChallenge {
    async fn prepare_challenge(self: &Arc<Self>, challenge: Http01Challenge);
}

async fn send_request<T: Serialize> (
    method: Method,
    directory: &Directory,
    url: &str,
    es_key: &SigningKey,
    kid: Option<String>,
    payload: T
) -> Result<Response<Body>, LetsEncryptError> {
    let client = get_client();
    let jwk: Option<ESJWK> = match kid {
        None => Some(es_key.clone().into()),
        Some(_) => None
    };

    let protected = JWSProtected {
        alg: "ES256".into(),
        kid,
        jwk,
        nonce: get_nonce(&directory).await?,
        url: url.into(),
    };

    let jws = serde_json::to_value(&get_jwt(&es_key, &protected, &payload)?)?;

    let req = Request::builder()
        .method(method)
        .uri(url)
        .header("Content-Type", "application/jose+json")
        .body(Body::from(jws.to_string()))
        .unwrap();

    Ok(client.request(req).await?)
}

impl Account {

    /// ## Request Structure
    /// ```text
    ///    POST /acme/new-account HTTP/1.1
    ///    Host: example.com
    ///    Content-Type: application/jose+json

    ///    {
    ///      "protected": base64url({
    ///        "alg": "ES256",
    ///        "jwk": {...},
    ///        "nonce": "6S8IqOGY7eL2lsGoTZYifg",
    ///        "url": "https://example.com/acme/new-account"
    ///      }),
    ///      "payload": base64url({
    ///        "termsOfServiceAgreed": true,
    ///        "contact": [
    ///          "mailto:cert-admin@example.org",
    ///          "mailto:admin@example.org"
    ///        ]
    ///      }),
    ///      "signature": "RZPOnYoPs1PhjszF...-nh6X1qtOFPB519I"
    ///    }
    /// ```
    ///
    /// ## Response Structure
    /// ```text
    ///    HTTP/1.1 201 Created
    ///    Content-Type: application/json
    ///    Replay-Nonce: D8s4D2mLs8Vn-goWuPQeKA
    ///    Location: https://example.com/acme/acct/evOfKhNU60wg
    ///    Link: <https://example.com/acme/some-directory>;rel="index"
    ///
    ///    {
    ///      "status": "valid",
    ///
    ///      "contact": [
    ///        "mailto:cert-admin@example.com",
    ///        "mailto:admin@example.com"
    ///      ],
    ///
    ///      "orders": "https://example.com/acme/acct/evOfKhNU60wg/orders"
    ///    }
    /// ```

    pub async fn new_account(
        directory: Directory,
        email: &str,
    ) -> Result<Self, LetsEncryptError> {
        let es_key = generate_es256_key();
        let private_key = create_rsa_key(2048);

        Self::get_account(directory, email, es_key, private_key).await
    }

    pub async fn account_from(
        directory: Directory,
        email: &str,
        es_key: &Vec<u8>,
        private_key: &Vec<u8>
    ) -> Result<Self, LetsEncryptError> {
        let private_key = PKey::private_key_from_pem(&private_key)
            .map_err(|_| LetsEncryptError::PrivateKeyError)?;

        let es_key = SigningKey::from_bytes(&es_key)
            .map_err(|_| LetsEncryptError::SigningKeyError)?;

        Self::get_account(directory, email, es_key, private_key).await
    }

    pub async fn get_account(
        directory: Directory,
        email: &str,
        es_key: SigningKey,
        private_key: PKey<Private>
    ) -> Result<Self, LetsEncryptError> {
        let payload = json!({
            "termsOfServiceAgreed": true,
            "contact": [
                format!("mailto:{}", email),
            ]
        });


        let response = send_request(Method::POST, &directory, &directory.new_account, &es_key, None, payload)
            .await?;

        if !response.status().is_success() {
            return Err(LetsEncryptError::UnexpectedResponse(response_debug_string(response).await?));
        }

        let location = response
            .headers()
            .get("Location")
            .ok_or_else(|| LetsEncryptError::MissingAccountLocationHeader)?
            .to_str()
            .unwrap()
            .to_string();

        let body = hyper::body::to_bytes(response.into_body())
            .await
            .map_err(|e| LetsEncryptError::HyperError(e))?
            .to_vec();

        #[derive(Deserialize)]
        struct AccountResponse {
            status: String
        }

        let response: AccountResponse = serde_json::from_slice(&body).map_err(|e| LetsEncryptError::SerdeJSONError(e))?;

        if response.status != "valid" {
            return Err(LetsEncryptError::CouldNotCreateAccount);
        }

        Ok(Account {
            es_key: es_key.to_bytes().to_vec(),
            location,
            directory,
            private_key
        })
    }

    pub async fn send_request<T: Serialize> (&self, method: Method, url: &str, payload: T) -> Result<Response<Body>, LetsEncryptError> {
        send_request(
            method,
            &self.directory,
            url,
            &self.get_signing_key()?,
            Some(self.location.clone()),
            payload)
        .await
    }

    pub fn get_signing_key(&self) -> Result<SigningKey, LetsEncryptError> {
        SigningKey::from_bytes(&self.es_key).map_err(|_| LetsEncryptError::SigningKeyError)
    }

    pub async fn thumbprint(&self, token: &str) -> Result<String, LetsEncryptError> {
        let jwk: ESJWK = self.get_signing_key()?.into();
        let jwk_thumb: JwkThumbprint = jwk.into();
        jwk_thumb.to_key_authorizaiton(token)
    }

    pub fn generate_csr(&self, domains: &[String]) -> Result<String, LetsEncryptError> {
        let csr = create_csr(&self.private_key, domains)?
            .to_der()
            .map_err(|e| LetsEncryptError::CSRError(format!("{:#?}", e)))?;
        // to der or pem??
        Ok(base64_url::encode(&csr))

    }

    pub async fn generate_certificate<S: ServesChallenge>(&self, domains: &[String], challenge_handler: Arc<S>) -> Result<Certificate, LetsEncryptError> {
        let payload = json!({
            "csr": self.generate_csr(domains)?
        });

        let (order_url, finalize_url) = new_order(&self, domains, challenge_handler).await?;

        let response = self.send_request(Method::POST, &finalize_url, payload).await?;

        if !response.status().is_success() {
            eprintln!("FinalizeURL: {}", response_debug_string(response).await?);
            return Err(LetsEncryptError::CouldNotFinaliseOrder);
        }

        let i = 1;
        let cert_url = loop {
            if i > 6 {
                return Err(LetsEncryptError::CouldNotFinaliseOrder);
            }

            let order = get_order(self, &order_url).await?;
            dbg!(&order);

            if order.status == "valid" {
                break order.certificate.ok_or(LetsEncryptError::CouldNotFinaliseOrder)?;
            }

            tokio::time::sleep(Duration::from_secs(1)).await;
        };

        let cert_data = self.send_request(Method::POST, &cert_url, json!("")).await?;

        if !cert_data.status().is_success() {
            eprintln!("GettingCertFailed: {}", response_debug_string(cert_data).await?);
            return Err(LetsEncryptError::CouldNotFinaliseOrder);
        }

        let cert = hyper::body::to_bytes(cert_data.into_body())
            .await
            .map_err(|e| LetsEncryptError::HyperError(e))?
            .to_vec();

        let priv_key_pem = self.private_key.private_key_to_pem_pkcs8()
            .map_err(|_| LetsEncryptError::PrivateKeyError)?;

        let cert = Certificate::new(priv_key_pem, cert);

        Ok(cert)
    }
}
