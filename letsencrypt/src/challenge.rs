use hyper::{Method};
use serde_json::{json, Map, Value};

use crate::{account::Account, error::LetsEncryptError, order::Identifier};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Http01Challenge {
    pub path: String,
    pub contents: String,
    pub challenge_url: String,
    pub domain: String
}
#[derive(Deserialize, Debug)]
pub struct AuthorizationResponse {
    pub status: String,
    pub challenges: Vec<Map<String, Value>>,
    pub identifier: Identifier,
}

pub async fn get_authorisation(account: &Account, authorization_url: &str) -> Result<AuthorizationResponse, LetsEncryptError> {
    let response = account.send_request(Method::POST, authorization_url, json!("")).await?;

    let body = hyper::body::to_bytes(response.into_body())
        .await
        .map_err(|e| LetsEncryptError::HyperError(e))?
        .to_vec();

    let response: AuthorizationResponse = serde_json::from_slice(&body)
        .map_err(|_| LetsEncryptError::CouldNotGetChallenge)?;

    Ok(response)
}

impl Http01Challenge {
    pub async fn new_http_01_challenge(
        account: &Account,
        authorization: &str
    ) -> Result<Self, LetsEncryptError> {
        let authorization_response = get_authorisation(account, authorization).await?;

        let challenge = authorization_response.challenges.iter().find(|challenge| {
            challenge["type"].as_str() == Some("http-01")
        });

        if let None = challenge {
            return Err(LetsEncryptError::CouldNotGetChallenge);
        }

        #[derive(Deserialize)]
        struct Http01ChallengeResponse {
            token: String,
            url: String,
        }

        let challenge: Http01ChallengeResponse = serde_json::from_value(serde_json::Value::Object(challenge.unwrap().clone()))
            .map_err(|_| LetsEncryptError::CouldNotGetChallenge)?;

        let key_authorization = account.thumbprint(&challenge.token).await?;

        Ok(Http01Challenge {
            path: format!("/.well-known/acme-challenge/{}", challenge.token),
            domain: authorization_response.identifier.value,
            contents: key_authorization,
            challenge_url: challenge.url
        })
    }
}
