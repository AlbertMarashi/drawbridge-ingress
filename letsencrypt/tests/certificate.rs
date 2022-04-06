use std::sync::Arc;

use async_trait::async_trait;
use letsencrypt::{error::LetsEncryptError, directory::Directory, account::ServesChallenge, challenge::Http01Challenge};
use utils::test_server::with_directory_server;
mod utils;

#[tokio::test]
async fn can_generate_certificate() -> Result<(), LetsEncryptError> {
    let (_handle, url) = with_directory_server();
    let directory = Directory::from_url(&url).await?;
    let account = directory.new_account("test@example.com").await?;

    struct Handler;

    #[async_trait]
    impl ServesChallenge for Handler {
        async fn prepare_challenge(self: &Arc<Self>, challenge: Http01Challenge) {
            dbg!("{:?}", challenge);
        }
    }
    let handler = Arc::new(Handler);
    let _cert = account.generate_certificate(&[
        "www.example.com".to_string(),
        "www.example.org".to_string()
    ], handler).await?;

    Ok(())
}

#[tokio::test]
async fn can_generate_csr() -> Result<(), LetsEncryptError> {
    let (_handle, url) = with_directory_server();
    let directory = Directory::from_url(&url).await?;
    let account = directory.new_account("test@example.com").await?;

    account.generate_csr(&[
        "www.example.com".to_string(),
        "www.example.org".to_string()
    ])?;

    Ok(())
}