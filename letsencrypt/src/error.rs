

#[derive(Debug)]
pub enum LetsEncryptError {
    HyperError(hyper::Error),
    StdError(std::io::Error),
    Utf8Error(std::str::Utf8Error),
    SerdeJSONError(serde_json::Error),
    NoNonce,
    CouldNotGeneratePrivateKey,
    CouldNotCreateAccount,
    UnexpectedResponse(String),
    MissingAccountLocationHeader,
    SigningKeyError,
    CouldNotCreateOrder,
    CouldNotGetChallenge,
    CouldNotValidateChallenge(String),
    CouldNotFinaliseOrder,
    InvalidCertificate,
    CSRError(String),
    PrivateKeyError,
    CouldNotGetOrder
}

impl From<hyper::Error> for LetsEncryptError {
    fn from(e: hyper::Error) -> Self {
        LetsEncryptError::HyperError(e)
    }
}

impl From<std::io::Error> for LetsEncryptError {
    fn from(e: std::io::Error) -> Self {
        LetsEncryptError::StdError(e)
    }
}

impl From<serde_json::Error> for LetsEncryptError {
    fn from(e: serde_json::Error) -> Self {
        LetsEncryptError::SerdeJSONError(e)
    }
}