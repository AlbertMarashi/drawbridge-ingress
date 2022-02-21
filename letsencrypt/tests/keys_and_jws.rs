use letsencrypt::{key::generate_key_pair};
use rsa::{PublicKey, PaddingScheme};


#[tokio::test]
async fn can_generate_key_pair() {
    let (private_key, public_key) = generate_key_pair(2048).unwrap();

    let data = b"hello world";
    let padding = PaddingScheme::new_pkcs1v15_encrypt();

    let mut rng = rand_core::OsRng;
    let enc_data = public_key.encrypt(&mut rng, padding, data).expect("failed to encrypt");

    let padding = PaddingScheme::new_pkcs1v15_encrypt();
    let dec_data = private_key.decrypt(padding, &enc_data).expect("failed to decrypt");

    assert_eq!(dec_data, data);
}