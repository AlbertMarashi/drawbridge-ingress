use letsencrypt::{error::LetsEncryptError, nonce::get_nonce, directory::{Directory, STAGING}};

pub mod utils;

#[tokio::test]
async fn can_get_nonce() -> Result<(), LetsEncryptError> {
    let directory = Directory::from_url(STAGING).await?;

    let nonce = get_nonce(&directory).await?;

    assert!(nonce.len() > 0);

    Ok(())
}