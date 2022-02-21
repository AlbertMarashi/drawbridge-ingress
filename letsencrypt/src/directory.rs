use hyper::{Request, Body, body::to_bytes};
use crate::{error::LetsEncryptError, request::get_client, account::Account};


pub const PRODUCTON: &str = "https://acme-v02.api.letsencrypt.org/directory";
pub const STAGING: &str = "https://acme-staging-v02.api.letsencrypt.org/directory";

#[derive(Serialize, Deserialize, Debug)]
pub struct Directory {
    #[serde(rename = "newAccount")]
    pub new_account: String,
    #[serde(rename = "newNonce")]
    pub new_nonce: String,
    #[serde(rename = "newOrder")]
    pub new_order: String,
}

impl Directory {
    /// Use hyper to make a request to the directory to get the URLs for resources
    pub async fn from_url(url: &str) -> Result<Self, LetsEncryptError> {
        let client = get_client();

        let request = Request::get(url)
            .body(Body::empty())
            .unwrap();

        let response = client.request(request).await?;

        let body = to_bytes(response.into_body())
            .await
            .map_err(|e| LetsEncryptError::HyperError(e))?
            .to_vec();

        let dir: Directory = serde_json::from_slice(&body)
            .map_err(|e| LetsEncryptError::SerdeJSONError(e))?;

        Ok(dir)
    }

    pub async fn new_account(self, email: &str) -> Result<Account, LetsEncryptError> {
        Account::new_account(self, email).await
    }
}

