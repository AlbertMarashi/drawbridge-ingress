use letsencrypt::{directory::{Directory, STAGING}, error::LetsEncryptError};



#[tokio::test]
async fn can_get_directory() -> Result<(), LetsEncryptError>{
    let _directory = Directory::from_url(STAGING).await?;

    Ok(())
}

