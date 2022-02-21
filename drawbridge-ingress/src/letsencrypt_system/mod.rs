use std::{collections::HashMap, sync::Arc};
use std::sync::RwLock as SyncRwLock;

use hyper::{Response, Body};
use k8s_openapi::api::core::v1::Secret;
use kube::ResourceExt;
use kube::{Client, Api, api::PostParams};
use letsencrypt::challenge::Http01Challenge;
use letsencrypt::directory::{Directory, STAGING};
use letsencrypt::{account::Account};
use rustls::sign::CertifiedKey;
use serde_json::json;
use tokio::sync::RwLock;

use crate::kube_config_tracker::RoutingTable;

const NAMESPACE: &str = "drawbridge-ingress";

type Host = String;
type Path = String;
pub struct LetsEncrypt {
    pub account: Account,
    pub certs: SyncRwLock<HashMap<String, CertKey>>,
    pub challenges: RwLock<HashMap<(Host, Path), String>>,
    pub routing_table: Arc<RoutingTable>,
    pub kube_api: Client,
}

pub struct CertKey { // DER encoded
    pub certs: Vec<Vec<u8>>,
    pub private_key: Vec<u8>,
    pub certified_key: Arc<CertifiedKey>,
}

pub type SecretCerts = Vec<(Host, CertData)>;

#[derive(Debug, Deserialize, Serialize)]
pub struct CertData {
    pub private_key: Vec<u8>,
    pub certs: Vec<Vec<u8>>,
}

impl rustls::server::ResolvesServerCert for LetsEncrypt {
    fn resolve(&self, client_hello: rustls::server::ClientHello) -> Option<Arc<CertifiedKey>> {
        let name = client_hello.server_name();

        match name {
            Some(name) => {
                let certs = self.certs.read().expect("certs lock poisoned");

                let cert = certs
                    .get(name);

                match cert {
                    Some(cert) => Some(cert.certified_key.clone()),
                    None => None,
                }
            }
            None => None,
        }
    }
}

fn cert_key_from(certs: Vec<Vec<u8>>, private_key: Vec<u8>) -> CertKey {
    let key = rustls::sign::RsaSigningKey::new(&rustls::PrivateKey(private_key.clone())).unwrap();

    CertKey {
        certified_key: Arc::new(CertifiedKey::new(
            certs.iter().map(|cert| rustls::Certificate(cert.clone())).collect(),
            Arc::new(key)
        )),
        certs,
        private_key,
    }
}

impl LetsEncrypt {
    pub async fn setup(rt: Arc<RoutingTable>) -> Self {
        let mut kube_api = Client::try_default().await.expect("Expected a valid KUBECONFIG environment variable");
        let account = Self::get_account(&mut kube_api).await;
        let certs = Self::get_certs(&mut kube_api).await;

        Self {
            account,
            certs: SyncRwLock::new(certs),
            challenges: RwLock::new(HashMap::new()),
            routing_table: rt,
            kube_api,
        }
    }

    /// we want to check if there is an existing account in the kubernetes secrets
    /// if there is, we want to use that account, otherwise we want to create a new one
    /// and store it in the kubernetes secrets
    async fn get_account(kube_api: &Client) -> Account {
        let secrets: Api<Secret> = Api::namespaced(kube_api.clone(), NAMESPACE);

        let account_secret = secrets.get("letsencrypt-account").await.ok();

        let directory = Directory::from_url(STAGING).await.expect("Could not get letsencrypt directory");

        match account_secret {
            Some(Secret {
                data: Some(data),
                ..
            }) => {
                let email = String::from_utf8_lossy(&data.get("email").unwrap().0).to_string();
                let private_key = &data.get("private_key").unwrap().0;
                let es_key = &data.get("es_key").unwrap().0;

                let account = Account::account_from(directory, &email, private_key, es_key).await.unwrap();

                return account;
            },
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
                        "name": "letsencrypt-account",
                        "namespace": NAMESPACE
                    },
                    "data": {
                        "private_key": private_key,
                        "email": "albert@framework.tools",
                        "es_key": account.es_key
                    }
                })).unwrap();

                secrets.create(&PostParams::default(), &data).await.expect("Failed to create secret");

                return account;
            },
        }
    }

    pub async fn get_certs(kube_api: &Client) -> HashMap<String, CertKey> {
        let mut map = HashMap::new();

        let secrets: Api<Secret> = Api::namespaced(kube_api.clone(), NAMESPACE);

        let certs = secrets.get("letsencrypt-certs").await.ok();

        match certs {
            Some(Secret {
                data: Some(data),
                ..
            }) => {
                let certs = &data.get("certs").unwrap().0;

                let certs: SecretCerts = serde_json::from_slice(certs).unwrap();

                for (host, cert_data) in certs {
                    map.insert(host, cert_key_from(cert_data.certs, cert_data.private_key));
                }
            },
            _ => {}
        }

        map
    }

    #[inline]
    pub async fn handle_if_challenge(&self, host: &str, path: &str) -> Option<Response<Body>> {
        if let Some(challenge) = self.challenges.read().await.get(&(host.to_string(), path.to_string())) {
            return Some(Response::new(Body::from(challenge.clone())))
        }
        None
    }

    pub async fn check_for_new_certificates(self: Arc<Self>) {
        let backends = &self.routing_table.backends_by_host
            .read()
            .await;

        let mut certs = self.certs.write().unwrap();

        for (host, _backend) in backends.iter() {
            let cert = certs.get(host);
            if cert.is_none() {
                let cert = self.account.generate_certificate(&[host.to_string()], |Http01Challenge {
                    contents,
                    domain,
                    path,
                    ..
                }| {
                    let clone = self.clone();
                    async move {
                        let mut challenges = clone.challenges.write().await;
                        challenges.insert((domain, path), contents);
                    }
                }).await.expect("Expected a valid certificate");

                let certs_vec = rustls_pemfile::certs(&mut Box::new(&cert.certificate_to_der()[..])).unwrap();
                certs.insert(host.to_string(), cert_key_from(certs_vec, cert.private_key_to_der()));
            }
        }

        let secrets: Api<Secret> = Api::namespaced(self.kube_api.clone(), NAMESPACE);

        let certs_secret = secrets.get("letsencrypt-certs").await.ok();

        let entries = {
            let mut entries = Vec::new();
            for (host, host_cert_key) in certs.iter() {
                entries.push((host.clone(), CertData {
                    certs: host_cert_key.certs.clone(),
                    private_key: host_cert_key.private_key.clone(),
                }));
            }
            entries
        };

        drop(certs);

        let mut data: Secret = serde_json::from_value(json!({
            "apiVersion": "v1",
            "kind": "Secret",
            "metadata": {
                "name": "letsencrypt-account",
                "namespace": NAMESPACE,
            },
            "data": {
                "certs": entries
            }
        })).unwrap();

        match certs_secret {
            Some(secret) => {
                // first we need to get the data and get revision so we can replace
                data.metadata.resource_version = secret.resource_version();
                secrets.replace("letsencrypt-account", &PostParams::default(), &data)
                    .await
                    .expect("Failed to replace secret");
            },
            None => {
                secrets.create(&PostParams::default(), &data)
                    .await
                    .expect("Failed to create secret");
            }
        }
    }
}