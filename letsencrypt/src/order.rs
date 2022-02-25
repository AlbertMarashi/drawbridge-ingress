use std::future::Future;

use hyper::{Method};
use serde_json::json;

use crate::{
    error::LetsEncryptError,
    account::Account, challenge::Http01Challenge,
};


#[derive(Deserialize, Debug)]
pub struct Identifier {
    /// The domain name
    pub value: String,
}

pub async fn new_order<F, Fut>(
    account: &Account,
    domains: &[String],
    handle_challenge: F
) -> Result<(String, String), LetsEncryptError>
where F: Fn(Http01Challenge) -> Fut,
    Fut: Future<Output = ()>
{
    let domain_identifiers = domains
        .iter()
        .map(|domain| json!({ "type": "dns", "value": domain }))
        .collect::<Vec<_>>();

    let response = account.send_request(Method::POST, &account.directory.new_order, json!({
        "identifiers": domain_identifiers,
    })).await?;

    if response.status() != 201 {
        return Err(LetsEncryptError::CouldNotCreateOrder);
    }

    let order_url = response
        .headers()
        .get("Location")
        .ok_or_else(|| LetsEncryptError::MissingAccountLocationHeader)?
        .to_str()
        .unwrap()
        .to_string();

    let body = hyper::body::to_bytes(response.into_body())
        .await
        .map_err(|e| LetsEncryptError::HyperError(e))?
        .to_vec();

    #[derive(Deserialize)]
    struct OrderResponse {
        status: String,
        authorizations: Vec<String>,
        finalize: String,
    }

    let response: OrderResponse = serde_json::from_slice(&body).map_err(|e| LetsEncryptError::SerdeJSONError(e))?;

    if response.status != "pending" {
        return Err(LetsEncryptError::CouldNotCreateOrder);
    }

    let mut challenges = Vec::new();
    for authorization in &response.authorizations {
        challenges.push(Http01Challenge::new_http_01_challenge(account, &authorization).await?);
    }

    for challenge in challenges {
        let mut i = 1;
        handle_challenge(challenge.clone()).await;

        loop {
            i += 1;
            let response = account.send_request(Method::POST, &challenge.finalise_url, json!({})).await?;
            let err_msg = format!("Could not validate challenge for domain: {} after {} attempts", &challenge.domain, i);
            let err = LetsEncryptError::CouldNotValidateChallenge(err_msg.clone());
            if response.status() != 200 {
                return Err(err);
            }

            let body = hyper::body::to_bytes(response.into_body())
                .await
                .map_err(|e| LetsEncryptError::HyperError(e))?
                .to_vec();

            #[derive(Deserialize)]
            struct ChallengeResponse {
                status: String,
            }

            let response: ChallengeResponse = serde_json::from_slice(&body)
                .map_err(|_| LetsEncryptError::CouldNotValidateChallenge(err_msg))?;

            if response.status == "pending" {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }

            if response.status == "valid" {
                break;
            }

            return Err(err);
        }
    }

    Ok((order_url, response.finalize))
}