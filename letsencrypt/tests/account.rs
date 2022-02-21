use letsencrypt::{error::LetsEncryptError, account::Account, directory::{Directory, STAGING}};


#[tokio::test]
async fn can_create_account() -> Result<(), LetsEncryptError> {
    let directory = Directory::from_url(STAGING).await?;
    let _account = Account::new_account(directory, "albert@insanemarketing.com.au").await?;

    Ok(())
}