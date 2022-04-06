use crate::certificate_state::{CertificateState, CertKey, Host, CertData, cert_key_from};
use crate::error::{IngressLoadBalancerError, Code};
use crate::kube_config_tracker::RoutingTable;
use k8s_openapi::api::core::v1::Secret;
use kube::ResourceExt;
use kube::{api::PostParams, Api, Client};
use letsencrypt::account::{Account, ServesChallenge};
use letsencrypt::directory::{Directory, PRODUCTON, STAGING};
use serde_json::json;
use std::sync::Arc;
use std::{collections::HashMap};

pub const NAMESPACE: &str = "drawbridge-ingress";
const ENV: Environment = Environment::Production;
pub type SecretCerts = Vec<(Host, CertData)>;

#[allow(dead_code)]
enum Environment {
    Production,
    Staging,
}

impl Environment {
    fn to_url(&self) -> &str {
        match self {
            Environment::Production => PRODUCTON,
            Environment::Staging => STAGING,
        }
    }

    fn to_name(&self) -> &str {
        match self {
            Environment::Production => "production",
            Environment::Staging => "staging",
        }
    }
}

pub struct CertGenerator {
    pub account: Account,
    pub routing_table: Arc<RoutingTable>,
    pub kube_api: Client,
    pub state: Arc<CertificateState>,
}


impl CertGenerator {
    pub async fn create(rt: Arc<RoutingTable>, state: Arc<CertificateState>) -> Arc<Self> {
        let mut kube_api = Client::try_default()
            .await
            .expect("Expected a valid KUBECONFIG environment variable");
        let account = Self::get_account(&mut kube_api).await;
        let certs = Self::get_certs(&kube_api).await;
        *state.certs.write().await = certs;

        Arc::new(Self {
            state,
            account,
            routing_table: rt,
            kube_api,
        })
    }

    /// we want to check if there is an existing account in the kubernetes secrets
    /// if there is, we want to use that account, otherwise we want to create a new one
    /// and store it in the kubernetes secrets
    async fn get_account(kube_api: &Client) -> Account {
        let secrets: Api<Secret> = Api::namespaced(kube_api.clone(), NAMESPACE);

        let account_secret = secrets
            .get(&format!("letsencrypt-account-{}", ENV.to_name()))
            .await
            .ok();

        let directory = Directory::from_url(ENV.to_url())
            .await
            .expect("Could not get letsencrypt directory");

        match account_secret {
            Some(Secret {
                data: Some(data), ..
            }) => {
                let email = &data.get("email").unwrap().0;
                let private_key = &data.get("private_key").unwrap().0;
                let es_key = &data.get("es_key").unwrap().0;

                let account = Account::account_from(
                    directory,
                    &String::from_utf8_lossy(&email),
                    &es_key,
                    &private_key,
                )
                .await
                .unwrap();

                return account;
            }
            _ => {
                let account = directory
                    .new_account("albert@framework.tools")
                    .await
                    .unwrap();

                let private_key = account.private_key.private_key_to_pem_pkcs8().unwrap();

                let data: Secret = serde_json::from_value(json!({
                    "apiVersion": "v1",
                    "kind": "Secret",
                    "metadata": {
                        "name": format!("letsencrypt-account-{}", ENV.to_name()),
                        "namespace": NAMESPACE
                    },
                    "data": {
                        "private_key": base64::encode(&private_key),
                        "email": base64::encode("albert@framework.tools"),
                        "es_key": base64::encode(&account.es_key)
                    }
                }))
                .unwrap();

                secrets
                    .create(&PostParams::default(), &data)
                    .await
                    .expect("Failed to create secret");

                return account;
            }
        }
    }

    pub async fn get_certs(kube_api: &Client) -> HashMap<String, CertKey> {
        let mut map = HashMap::new();

        let secrets: Api<Secret> = Api::namespaced(kube_api.clone(), NAMESPACE);

        let certs = secrets
            .get(&format!("letsencrypt-certs-{}", ENV.to_name()))
            .await
            .ok();

        match certs {
            Some(Secret {
                data: Some(data), ..
            }) => {
                let certs = &data.get("certs").unwrap().0;

                let certs: SecretCerts = serde_json::from_slice(certs).unwrap();

                for (host, cert_data) in certs {
                    map.insert(host, cert_key_from(cert_data.certs, cert_data.private_key));
                }
            }
            _ => {}
        }

        map
    }

    pub async fn check_for_new_certificates<S: ServesChallenge> (&self, server: Arc<S>) -> Result<(), IngressLoadBalancerError> {
        let backends = self.routing_table.backends_by_host.read().await;
        let mut certs = self.state.certs.write().await;

        for (host, _backend) in backends.iter() {
            let cert = certs.get(host);
            if cert.is_none() {
                let cert = self
                    .account
                    .generate_certificate( &[host.to_string()], server.clone())
                    .await
                    .map_err(|e| IngressLoadBalancerError::General(Code::CouldNotGenerateCertificate, format!("{:#?}", e).into()))?;

                let certs_vec =
                    rustls_pemfile::certs(&mut Box::new(&cert.certificate_to_pem()[..]))
                    .map_err(|e| IngressLoadBalancerError::General(Code::CouldNotGenerateCertificate, format!("{:#?}", e).into()))?;
                certs.insert(
                    host.to_string(),
                    cert_key_from(certs_vec, cert.private_key_to_der()),
                );
            }
        }

        let secrets: Api<Secret> = Api::namespaced(self.kube_api.clone(), NAMESPACE);

        let certs_secret = secrets
            .get(&format!("letsencrypt-certs-{}", ENV.to_name()))
            .await
            .ok();

        let entries = {
            let mut entries = Vec::new();
            for (host, host_cert_key) in certs.iter() {
                entries.push((
                    host.clone(),
                    CertData {
                        certs: host_cert_key.certs.clone(),
                        private_key: host_cert_key.private_key.clone(),
                    },
                ));
            }
            entries
        };


        let mut data: Secret = serde_json::from_value(json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": {
                "name": format!("letsencrypt-certs-{}", ENV.to_name()),
                "namespace": NAMESPACE,
            },
            "data": {
                "certs": base64::encode(json!(entries).to_string()),
            }
        }))
        .unwrap();

        match certs_secret {
            Some(secret) => {
                // first we need to get the data and get revision so we can replace
                data.metadata.resource_version = secret.resource_version();
                secrets
                    .replace(
                        &format!("letsencrypt-certs-{}", ENV.to_name()),
                        &PostParams::default(),
                        &data,
                    )
                    .await
                    .expect("Failed to replace secret");
            }
            None => {
                secrets
                    .create(&PostParams::default(), &data)
                    .await
                    .expect("Failed to create secret");
            }
        }

        Ok(())
    }
}

#[ignore]
#[tokio::test]
async fn can_convert_account_to_secret() {
    let kube_api = Client::try_default()
        .await
        .expect("Expected a valid KUBECONFIG environment variable");

    let secrets: Api<Secret> = Api::namespaced(kube_api.clone(), NAMESPACE);

    let directory = Directory::from_url(ENV.to_url())
        .await
        .expect("Could not get letsencrypt directory");

    let account = directory
        .new_account("albert@framework.tools")
        .await
        .unwrap();

    let private_key = account.private_key.private_key_to_pem_pkcs8().unwrap();

    let data: Secret = serde_json::from_value(json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": {
            "name": format!("letsencrypt-account-{}", ENV.to_name()),
            "namespace": NAMESPACE
        },
        "data": {
            "private_key": base64::encode(&private_key),
            "email": base64::encode("albert@framework.tools"),
            "es_key": base64::encode(&account.es_key),
        }
    }))
    .expect("Err creating secret");

    secrets
        .create(&PostParams::default(), &data)
        .await
        .expect("Failed to create secret");
}

#[ignore]
#[tokio::test]
async fn can_convert_account_to_secret_2() {
    let kube_api = Client::try_default()
        .await
        .expect("Expected a valid KUBECONFIG environment variable");

    let _account = CertGenerator::get_account(&kube_api).await;
}
