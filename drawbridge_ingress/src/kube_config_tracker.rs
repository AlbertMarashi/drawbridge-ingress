//! # Kubernetes Config Tracker
//!
//! This module tracks the configuration of services, ingresses required for routing traffic to kubernetes services.
//!
//! It sets up a watcher which listens for changes to the kubernetes api and updates the routing table accordingly.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use k8s_openapi::api::core::v1::{Pod};
use k8s_openapi::api::networking::v1::Ingress;
use kube::core::WatchEvent;
use kube::{Api, Client, api::ListParams, runtime};
use futures::{StreamExt, TryStreamExt};
use regex::Regex;
use tokio::sync::RwLock;

use crate::{IngressLoadBalancerError, Code};

#[derive(Debug, Clone)]
pub enum ChangeType {
    BackendChanged,
    PeerAdded(String),
    PeerRemoved(String),
}
pub struct RoutingTable {
    subscribers: RwLock<Vec<Box<dyn Fn(ChangeType) + Sync + Send>>>,
    pub backends_by_host: RwLock<HashMap<String, HashSet<Backend>>>, // there may be multiple backends for a host, so we need to store them in a map later
}

impl RoutingTable {
    pub fn new() -> Self {
        Self {
            subscribers: RwLock::new(Vec::new()),
            backends_by_host: RwLock::new(HashMap::new())
        }
    }

    pub async fn start_watching(self: Arc<Self>) {
        let client = Client::try_default().await.expect("Expected a valid KUBECONFIG environment variable");
        let pod_api: Api<Pod> = Api::all(client.clone());
        let ingress_api: Api<Ingress> = Api::all(client.clone());
        let mut handles = Vec::new();

        let rt = self.clone();

        handles.push(tokio::spawn(async move {
            let watcher = runtime::watcher(ingress_api, ListParams::default());
            runtime::utils::try_flatten_applied(watcher)
                .for_each_concurrent(None, |e| {
                    let rt = rt.clone();

                    async move {
                        let resource = e.expect("Expected a valid resource");

                        // Ingress name
                        let name = resource.metadata.name.clone();
                        println!("Ingress Name: {:?} changed", name);

                        if let None = resource.spec {
                            return;
                        }

                        if let None = resource.spec.as_ref().unwrap().rules {
                            return;
                        }

                        let rules = &resource.spec.unwrap().rules.unwrap();

                        for rule in rules {
                            if let None = rule.host {
                                continue; // we don't support rules without a host
                            }

                            if let None = rule.http.as_ref() {
                                continue; // we don't support rules without an http spec
                            }

                            let paths = &rule.http.as_ref().unwrap().paths;

                            for path in paths {
                                if let None = path.path {
                                    continue; // we don't support rules without a path
                                }

                                let backend = &path.backend;

                                if let None = backend.service {
                                    continue; // we don't support rules without a service
                                }

                                if let None = backend.service.as_ref().unwrap().port {
                                    continue; // we don't support rules without a service port
                                }

                                if let None = backend.service.as_ref().unwrap().port.as_ref().unwrap().number {
                                    continue; // we don't support rules without a service port
                                }

                                if let None = path.path_type {
                                    continue; // we don't support rules without a path type
                                }

                                let path_type = path.path_type.as_ref().unwrap();

                                if path_type != "Prefix" {
                                    continue; // we only support prefix paths
                                }

                                let host = rule.host.as_ref().unwrap();
                                let service_name = &backend.service.as_ref().unwrap().name;
                                let service_port = backend.service.as_ref().unwrap().port.as_ref().unwrap();
                                let path_prefix = path.path.as_ref().unwrap();

                                let backend = Backend::with_prefix(
                                    host.to_string(),
                                    path_prefix.to_string(),
                                    service_name.to_string(),
                                    service_port.number.unwrap() as u16);

                                let mut backends = rt.backends_by_host.write().await;

                                // check if the host already has backends
                                let backends_for_host = if let Some(backends_for_host) = backends.get_mut(host) {
                                    backends_for_host
                                } else {
                                    backends.insert(host.to_string(), HashSet::new());
                                    backends.get_mut(host).unwrap()
                                };

                                // add the backend to the host
                                backends_for_host.insert(backend);

                                rt.notify_subscribers(ChangeType::BackendChanged).await;
                            }
                        }
                    }
                }).await;
        }));

        let rt = self.clone();

        // watching for changes to peer ingress pods
        handles.push(tokio::spawn(async move {
            let mut stream = pod_api.watch(&ListParams::default().labels("app=drawbridge-ingress-service"), "0").await.unwrap().boxed();

            while let Some(status) = stream.try_next().await.unwrap() {
                match status {
                    WatchEvent::Added(pod) => {
                        let name = pod.metadata.name.clone().unwrap();
                        println!("Peer: {:?} added", name);

                        rt.notify_subscribers(ChangeType::PeerAdded(name)).await;
                    },
                    WatchEvent::Deleted(pod) => {
                        let name = pod.metadata.name.clone().unwrap();
                        println!("Peer: {:?} deleted", name);

                        rt.notify_subscribers(ChangeType::PeerRemoved(name)).await;
                    },
                    _ => {}
                }
            }
        }));


        // join all the futures
        futures::future::join_all(handles).await;
    }

    pub async fn subscribe(&self, subscriber: Box<dyn Fn(ChangeType) + Sync + Send>) {
        self.subscribers.write().await.push(subscriber);
    }

    pub async fn notify_subscribers(&self, change_type: ChangeType) {
        let mut subscribers = self.subscribers.write().await;

        for subscriber in subscribers.iter_mut() {
            subscriber(change_type.clone());
        }
    }

    pub async fn get_backend(&self, host: &str, path: &str) -> Result<String, IngressLoadBalancerError> {
        let backends_for_host = self.backends_by_host.read().await;

        if let Some(backends_for_host) = backends_for_host.get(host) {
            for backend in backends_for_host {
                if backend.matches(path) {
                    return Ok(backend.service_name.clone());
                }
            }
        }

        Err(IngressLoadBalancerError::general(Code::NonExistentHost, format!("No backend found for host: {}", host)))
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct Backend {
    host: String,
    path_regex: RegexWrapper,
    service_name: String,
    port: u16
}

#[derive(Debug)]
struct RegexWrapper(Regex);

impl Eq for RegexWrapper {}

impl PartialEq for RegexWrapper {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_str() == other.0.as_str()
    }
}

impl core::hash::Hash for RegexWrapper {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.as_str().hash(state);
    }
}

impl Backend {
    fn with_prefix(host: String, path_prefix: String, service_name: String, port: u16) -> Backend {
        let path_regex = Regex::new(&format!("^{}", regex::escape(&path_prefix))).expect("Expected a valid regex");

        Backend {
            host,
            path_regex: RegexWrapper(path_regex),
            service_name,
            port
        }
    }

    fn matches(&self, path: &str) -> bool {
        self.path_regex.0.is_match(path)
    }
}