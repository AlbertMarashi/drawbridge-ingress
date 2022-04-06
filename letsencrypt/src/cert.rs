use std::time::{Duration, SystemTime};

use openssl::asn1::Asn1Time;
use openssl::hash::MessageDigest;
use openssl::pkey::{PKey, Private};
use openssl::rsa::Rsa;
use openssl::stack::Stack;
use openssl::x509::extension::SubjectAlternativeName;
use openssl::x509::{X509Req, X509ReqBuilder, X509};

use crate::error::LetsEncryptError;

/// Make an RSA private key (from which we can derive a public key).
///
/// This library does not check the number of bits used to create the key pair.
/// For Let's Encrypt, the bits must be between 2048 and 4096.
pub fn create_rsa_key(bits: u32) -> PKey<Private> {
    let pri_key_rsa = Rsa::generate(bits).expect("Rsa::generate");
    PKey::from_rsa(pri_key_rsa).expect("PKey::from_rsa")
}

pub(crate) fn create_csr(pkey: &PKey<Private>, domains: &[String]) -> Result<X509Req, LetsEncryptError> {
    //
    // the csr builder
    let mut req_bld = X509ReqBuilder::new()
        .expect("X509ReqBuilder::new");

    // set private/public key in builder
    req_bld.set_pubkey(pkey)
        .expect("set_pubkey");

    // set all domains as alt names
    let mut stack = Stack::new().expect("Stack::new");
    let ctx = req_bld.x509v3_context(None);
    let as_lst = domains
        .iter()
        .map(|e| format!("DNS:{}", e))
        .collect::<Vec<_>>()
        .join(", ");
    let as_lst = as_lst[4..].to_string();
    let mut an = SubjectAlternativeName::new();
    an.dns(&as_lst);
    let ext = an.build(&ctx).expect("SubjectAlternativeName::build");
    stack.push(ext).expect("Stack::push");
    req_bld.add_extensions(&stack).expect("add_extensions");

    // sign it
    req_bld
        .sign(pkey, MessageDigest::sha256())
        .expect("csr_sign");
    let csr = req_bld.build();

    // the csr
    Ok(csr)
}

/// Encapsulated certificate and private key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Certificate {
    pub private_key: Vec<u8>,
    pub certificate: Vec<u8>,
}

impl Certificate {
    pub(crate) fn new(private_key: Vec<u8>, certificate: Vec<u8>) -> Self {
        Certificate {
            private_key,
            certificate,
        }
    }

    /// The PEM encoded private key.
    pub fn private_key_to_pem(&self) -> &[u8] {
        &self.private_key
    }

    /// The private key as DER.
    pub fn private_key_to_der(&self) -> Vec<u8> {
        let pkey = PKey::private_key_from_pem(&self.private_key).expect("from_pem");
        pkey.private_key_to_der().expect("private_key_to_der")
    }

    /// The PEM encoded issued certificate.
    pub fn certificate_to_pem(&self) -> &[u8] {
        &self.certificate
    }

    /// The issued certificate as DER.
    pub fn certificate_to_der(&self) -> Vec<u8> {
        let x509 = X509::from_pem(&self.certificate).expect("from_pem");
        x509.to_der().expect("to_der")
    }

    pub fn expiry(&self) -> Result<std::time::SystemTime, LetsEncryptError> {
        let x509 = X509::from_pem(&self.certificate)
            .map_err(|_| LetsEncryptError::InvalidCertificate)?;

        let not_after = x509.not_after();
        let unix_time = Asn1Time::from_unix(0)
            .map_err(|_| LetsEncryptError::InvalidCertificate)?
            .diff(&not_after)
            .map_err(|_| LetsEncryptError::InvalidCertificate)?;

        let time_diff = unix_time.days as u64 * 86400 + unix_time.secs as u64;
        let time_elapsed = Duration::from_secs(time_diff);
        Ok(SystemTime::UNIX_EPOCH + time_elapsed)
    }

    /// Inspect the certificate to count the number of (whole) valid days left.
    ///
    /// It's up to the ACME API provider to decide how long an issued certificate is valid.
    /// Let's Encrypt sets the validity to 90 days. This function reports 89 days for newly
    /// issued cert, since it counts _whole_ days.
    ///
    /// It is possible to get negative days for an expired certificate.
    pub fn valid_days_left(&self) -> Result<i64, LetsEncryptError> {
        let expiry = self.expiry()?;
        let now = SystemTime::now();
        let duration = expiry.duration_since(now)
            .map_err(|_| LetsEncryptError::InvalidCertificate)?;
        let days = duration.as_secs() / 86400;

        Ok(days as i64)
    }
}