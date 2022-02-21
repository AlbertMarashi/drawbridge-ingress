use rsa::{RsaPrivateKey, RsaPublicKey};

use crate::error::LetsEncryptError;

pub fn generate_key_pair(bits: usize) -> Result<(RsaPrivateKey, RsaPublicKey), LetsEncryptError>{
    let mut rng = rand_core::OsRng;

    let private_key = RsaPrivateKey::new(&mut rng, bits).map_err(|_| LetsEncryptError::CouldNotGeneratePrivateKey)?;
    let public_key = RsaPublicKey::from(&private_key);

    Ok((private_key, public_key))
}