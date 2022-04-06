use hyper::{Response, Body};

use crate::error::LetsEncryptError;

pub mod error;
pub mod account;
pub mod directory;
pub mod order;
pub mod challenge;
pub mod request;
pub mod nonce;
pub mod jwt;
pub mod cert;
pub mod key;

#[macro_use]
extern crate serde;

pub async fn response_debug_string(response: Response<Body>) -> Result<String, LetsEncryptError> {
    let status = response.status();
    let headers = format!("{:?}", response.headers());
    Ok(format!("{status}\n{headers}\n {}", {
        String::from_utf8_lossy(&hyper::body::to_bytes(response.into_body())
            .await
            .map_err(|e| LetsEncryptError::HyperError(e))?
            .to_vec())
    }))
}