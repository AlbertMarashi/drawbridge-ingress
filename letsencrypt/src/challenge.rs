use hyper::{Method};
use serde_json::{json, Map, Value};

use crate::{account::Account, error::LetsEncryptError, order::Identifier};

#[derive(Deserialize, Debug, Clone)]
pub struct Http01Challenge {
    pub authorization_endpoint: String,
    pub path: String,
    pub contents: String,
    pub finalise_url: String,
    pub domain: String
}

impl Http01Challenge {
    pub async fn new_http_01_challenge(
        account: &Account,
        authorization: &str
    ) -> Result<Self, LetsEncryptError> {
        let response = account.send_request(Method::POST, &authorization, json!("")).await?;

        let body = hyper::body::to_bytes(response.into_body())
            .await
            .map_err(|e| LetsEncryptError::HyperError(e))?
            .to_vec();

        #[derive(Deserialize)]
        struct ChallengeResponse {
            challenges: Vec<Map<String, Value>>,
            identifier: Identifier,
        }

        let response: ChallengeResponse = serde_json::from_slice(&body)
            .map_err(|_| LetsEncryptError::CouldNotGetChallenge)?;

        let challenge = response.challenges.iter().find(|challenge| {
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
            authorization_endpoint: authorization.to_string(),
            path: format!("/.well-known/acme-challenge/{}", challenge.token),
            domain: response.identifier.value,
            contents: key_authorization,
            finalise_url: challenge.url
        })
    }
}