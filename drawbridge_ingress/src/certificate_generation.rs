use crate::kube_config_tracker::RoutingTable;
use hyper::{Body, Response};
use k8s_openapi::api::core::v1::Secret;
use kube::ResourceExt;
use kube::{api::PostParams, Api, Client};
use letsencrypt::account::Account;
use letsencrypt::challenge::Http01Challenge;
use letsencrypt::directory::{Directory, PRODUCTON, STAGING};
use rustls::ServerConfig;
use rustls::sign::CertifiedKey;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tls_acceptor::tls_acceptor::ResolvesServerConf;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

const NAMESPACE: &str = "drawbridge-ingress";
const ENV: Environment = Environment::Staging;

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

pub type Host = String;
pub type Path = String;
pub struct LetsEncrypt {
    pub account: Account,
    pub state: Arc<RwLock<CertificateState>>,
    pub routing_table: Arc<RoutingTable>,
    pub kube_api: Client,
}

pub struct CertificateState {
    pub certs: RwLock<HashMap<String, CertKey>>,
    pub challenges: RwLock<HashMap<(Host, Path), String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CertificateStateMinimal {
    pub certs: HashMap<String, CertKey>,
    pub challenges: HashMap<(Host, Path), String>,
}

impl CertificateState {
    pub async fn to_minimal(&self) -> CertificateStateMinimal {
        CertificateStateMinimal {
            certs: self.certs.read().await.clone(),
            challenges: self.challenges.read().await.clone(),
        }
    }
}

impl From<CertificateStateMinimal> for CertificateState {
    fn from(state: CertificateStateMinimal) -> Self {
        CertificateState {
            certs: RwLock::new(state.certs),
            challenges: RwLock::new(state.challenges),
        }
    }
}

impl Serialize for CertKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize)]
        struct CertKeyMinimal {
            pub certs: Vec<Vec<u8>>,
            pub private_key: Vec<u8>,
        }

        let cert_key_minimal = CertKeyMinimal {
            certs: self.certs.clone(),
            private_key: self.private_key.clone(),
        };

        cert_key_minimal.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CertKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct CertKeyMinimal {
            pub certs: Vec<Vec<u8>>,
            pub private_key: Vec<u8>,
        }

        let cert_key_minimal = CertKeyMinimal::deserialize(deserializer)?;

        Ok(cert_key_from(
            cert_key_minimal.certs,
            cert_key_minimal.private_key,
        ))
    }
}

#[derive(Clone)]
pub struct CertKey {
    // DER encoded
    pub certs: Vec<Vec<u8>>,
    pub private_key: Vec<u8>,
    pub certified_key: Arc<CertifiedKey>,
}

impl std::fmt::Debug for CertKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CertKey {{ certs: {:?}, private_key: {:?} }}",
            self.certs, self.private_key
        )
    }
}

pub type SecretCerts = Vec<(Host, CertData)>;

#[derive(Debug, Deserialize, Serialize)]
pub struct CertData {
    pub private_key: Vec<u8>,
    pub certs: Vec<Vec<u8>>,
}

fn cert_key_from(certs: Vec<Vec<u8>>, private_key: Vec<u8>) -> CertKey {
    let key = rustls::sign::RsaSigningKey::new(&rustls::PrivateKey(private_key.clone())).unwrap();

    CertKey {
        certified_key: Arc::new(CertifiedKey::new(
            certs
                .iter()
                .map(|cert| rustls::Certificate(cert.clone()))
                .collect(),
            Arc::new(key),
        )),
        certs,
        private_key,
    }
}

#[async_trait::async_trait]
impl ResolvesServerConf for LetsEncrypt {
    async fn resolve_server_config(&self, hello: &rustls::server::ClientHello) -> Option<Arc<ServerConfig>> {
        let name = hello.server_name();

        match name {
            Some(name) => {
                let state = self.state.clone();
                let state = state.read().await;
                let certs = state.certs.read().await;
                let cert = certs.get(name);

                match cert {
                    Some(cert) => {
                        let mut cfg = rustls::ServerConfig::builder()
                            .with_safe_default_cipher_suites()
                            .with_safe_default_kx_groups()
                            .with_safe_default_protocol_versions()
                            .unwrap()
                            .with_no_client_auth()
                            .with_single_cert(cert.certs.iter()
                            .map(|cert| rustls::Certificate(cert.clone()))
                            .collect(), rustls::PrivateKey(cert.private_key.clone()))
                            .unwrap();

                        cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

                        Some(Arc::new(cfg))
                    }
                    None => None,
                }
            }
            None => None,
        }
    }
}

impl LetsEncrypt {
    pub async fn setup(rt: Arc<RoutingTable>) -> Self {
        let mut kube_api = Client::try_default()
            .await
            .expect("Expected a valid KUBECONFIG environment variable");
        let account = Self::get_account(&mut kube_api).await;
        let certs = Self::get_certs(&mut kube_api).await;

        Self {
            account,
            state: Arc::new(RwLock::new(CertificateState {
                certs: RwLock::new(certs),
                challenges: RwLock::new(HashMap::new()),
            })),
            routing_table: rt,
            kube_api,
        }
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

    #[inline]
    pub async fn handle_if_challenge(&self, host: &str, path: &str) -> Option<Response<Body>> {
        let state = self.state.read().await;
        if let Some(challenge) = state
            .challenges
            .read()
            .await
            .get(&(host.to_string(), path.to_string()))
        {
            return Some(Response::new(Body::from(challenge.clone())));
        }
        None
    }

    pub async fn check_for_new_certificates(&self) {
        let backends = self.routing_table.backends_by_host.read().await;
        let state = self.state.read().await;

        let mut certs = state.certs.write().await;

        for (host, _backend) in backends.iter() {
            let cert = certs.get(host);
            if cert.is_none() {
                let cert = self
                    .account
                    .generate_certificate(
                    &[host.to_string()],
                    |Http01Challenge {
                            contents,
                            domain,
                            path,
                            ..
                        }| {
                            let state = self.state.clone();
                            async move {
                                let state = state.write().await;

                                let mut challenges = state.challenges.write().await;
                                challenges.insert((domain, path), contents);
                            }
                        },
                    )
                    .await
                    .expect("Expected a valid certificate");

                let certs_vec =
                    rustls_pemfile::certs(&mut Box::new(&cert.certificate_to_pem()[..])).unwrap();
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

        drop(certs);

        let mut data: Secret = serde_json::from_value(json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": {
                "name": format!("letsencrypt-certs-{}", ENV.to_name()),
                "namespace": NAMESPACE,
            },
            "data": {
                "certs": entries
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

    let _account = LetsEncrypt::get_account(&kube_api).await;
}
