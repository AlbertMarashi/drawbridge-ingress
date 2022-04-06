use std::sync::Arc;

use hyper::{Method};
use serde_json::json;

use crate::{
    error::LetsEncryptError,
    account::{Account, ServesChallenge}, challenge::{Http01Challenge, get_authorisation}, response_debug_string,
};


#[derive(Deserialize, Debug)]
pub struct Identifier {
    /// The domain name
    pub value: String,
}

#[derive(Deserialize, Debug)]
pub struct OrderResponse {
    pub status: String,
    pub authorizations: Vec<String>,
    pub finalize: String,
    pub certificate: Option<String>
}

pub async fn get_order(account: &Account, order_url: &str) -> Result<OrderResponse, LetsEncryptError> {
    let response = account.send_request(Method::POST, order_url, json!("")).await?;

    let body = hyper::body::to_bytes(response.into_body())
        .await
        .map_err(|e| LetsEncryptError::HyperError(e))?
        .to_vec();

    let response: OrderResponse = serde_json::from_slice(&body)
        .map_err(|_| LetsEncryptError::CouldNotGetOrder)?;

    Ok(response)
}

pub async fn new_order<S: ServesChallenge>(
    account: &Account,
    domains: &[String],
    challenge_handler: Arc<S>,
) -> Result<(String, String), LetsEncryptError> {
    let domain_identifiers = domains
        .iter()
        .map(|domain| json!({ "type": "dns", "value": domain }))
        .collect::<Vec<_>>();

    let response = account.send_request(Method::POST, &account.directory.new_order, json!({
        "identifiers": domain_identifiers,
    })).await?;

    if !response.status().is_success() {
        eprintln!("{}", response_debug_string(response).await?);
        return Err(LetsEncryptError::CouldNotCreateOrder);
    }

    let order_url = response
        .headers()
        .get("Location")
        .ok_or_else(|| LetsEncryptError::MissingAccountLocationHeader)?
        .to_str()
        .unwrap()
        .to_string();

    let order = get_order(&account, &order_url).await?;

    // if order.status != "pending" && order.status != "ready" {
    //     println!("{:?}", order);
    //     return Err(LetsEncryptError::CouldNotCreateOrder);
    // }

    let mut auths = Vec::new();
    for authorization in &order.authorizations {
        auths.push((authorization.clone(), Http01Challenge::new_http_01_challenge(account, &authorization).await?));
    }

    for (authorization, challenge) in auths {
        challenge_handler.prepare_challenge(challenge.clone()).await;
        let response = account.send_request(Method::POST, &challenge.challenge_url, json!({})).await?;

        if !response.status().is_success() {
            let err_msg = format!("Could not validate challenge for domain: {}", &challenge.domain);
            let err = LetsEncryptError::CouldNotValidateChallenge(err_msg.clone());

            eprintln!("{}", response_debug_string(response).await?);
            return Err(err);
        }

        for i in 1..6 {
            let authorisation_response = get_authorisation(account, &authorization).await?;

            if authorisation_response.status == "pending" {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }

            if authorisation_response.status == "valid" || authorisation_response.status == "ready" {
                break;
            }

            dbg!(authorisation_response);

            let err_msg = format!("Could not validate challenge for domain: {} after {} attempts", &challenge.domain, i);
            let err = LetsEncryptError::CouldNotValidateChallenge(err_msg.clone());

            return Err(err);
        }
    }

    let order = get_order(&account, &order_url).await?;
    dbg!(&order);

    return Ok((order_url, order.finalize.clone()));
    // 5 attempts to check if order is valid
    // for _ in 0..5 {
    //     let order = get_order(&account, &order_url).await?;

    //     if order.status == "valid" || order.status == "ready" {

    //     }

    //     tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    // }

    // dbg!("Failed to create order");
    // Err(LetsEncryptError::CouldNotCreateOrder)
}