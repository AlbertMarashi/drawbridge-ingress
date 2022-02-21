use hyper::{Request, Body};

use crate::{request::get_client, directory::Directory, error::LetsEncryptError};


pub type Nonce = String;

pub async fn get_nonce(directory: &Directory) -> Result<Nonce, LetsEncryptError> {
    let client = get_client();
    let url = &directory.new_nonce;
    let req = Request::head(url).body(Body::empty()).unwrap();
    let resp = client.request(req).await?;
    let nonce = resp.headers().get("replay-nonce").ok_or(LetsEncryptError::NoNonce)?;
    Ok(nonce.to_str().map_err(|_| LetsEncryptError::NoNonce)?.to_string())
}