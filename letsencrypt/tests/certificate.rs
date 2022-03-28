// use letsencrypt::{error::LetsEncryptError, directory::Directory};
// use utils::test_server::with_directory_server;
// mod utils;

// #[tokio::test]
// async fn can_generate_certificate() -> Result<(), LetsEncryptError> {
//     let (_handle, url) = with_directory_server();
//     let directory = Directory::from_url(&url).await?;
//     let account = directory.new_account("test@example.com").await?;

//     let _cert = account.generate_certificate(&[
//         "www.example.com".to_string(),
//         "www.example.org".to_string()
//     ], |challenge| {

//         async move {
//             drop(challenge);
//         }
//     }).await?;

//     Ok(())
// }