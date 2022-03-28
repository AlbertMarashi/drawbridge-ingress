//! # Drawbridge-Ingress
//!
//! A load balancer written in rust made for routing traffic to kubernetes services.
//!
//! ## How it works
//! 1. A task which regularly queries the kubernetes api for the list of services, ingresses, and listens for changes.
//! 2. letsencrypt is used to generate certificates for the hosts configured in the ingress.
//! 3. Constructs a routing rable based on the loaded kubernetes ingress configurations.
//! 4. listen on :80 and :443 for incoming http and https requests. The requests are routed to the appropriate service
//! according to the routing table, and a reverse proxy is used to forward the request to the service.
//! 5. incoming https requests are matched against the appropriate SSL certificate generated by letsencrypt.
//!
#![feature(never_type)]
#![feature(try_blocks)]
use hyper::server::conn::AddrIncoming;
use hyper::service::{make_service_fn, service_fn};
use hyper::Server;
use kube_config_tracker::{ChangeType, RoutingTable};
use proxy::proxy_request;
use std::net::SocketAddr;
use std::sync::Arc;
use tls_acceptor::tls_acceptor::TlsAcceptor;

use crate::error::{Code, IngressLoadBalancerError};

mod lets_encrypt;
mod error;
mod kube_config_tracker;
mod leadership_system;
mod proxy;
mod certificate_state;

type Error = Box<dyn std::error::Error + Send + Sync + 'static>;

fn get_current_pod_name() -> String {
    std::env::var_os("CURRENT_POD_NAME")
        .unwrap()
        .to_string_lossy()
        .to_string()
}

#[tokio::main]
async fn main() -> Result<(), IngressLoadBalancerError> {
    let routing_table = Arc::new(RoutingTable::new());
    let leadership_system = leadership_system::LeadershipSystem::new(
        get_current_pod_name(),
        routing_table.clone(),
    );

    let leadership_system_clone = leadership_system.clone();
    // watch for changes to the backends and peers.
    // if we are the leader, we will generate new certificates.
    // if we are not the leader, we will just ignore
    // if there are new peers, we will let the senator know
    // if peers were removed we will let the senator know
    routing_table
        .subscribe(Box::new(move |change_type| {
            let leadership_system = leadership_system_clone.clone();
            match change_type {
                ChangeType::BackendChanged => {
                    tokio::task::spawn(leadership_system.handle_change());
                }
                ChangeType::PeerAdded { addr, name } => if name != get_current_pod_name() {
                    tokio::task::spawn(leadership_system.add_peer(addr, name));
                }
                ChangeType::PeerRemoved { name } => if name != get_current_pod_name() {
                    tokio::task::spawn(leadership_system.remove_peer(name));
                }
            }
        }))
        .await;

    // start a task which listens for changes to the kubernetes api
    // and updates the routing table accordingly
    tokio::spawn(routing_table.clone().start_watching());

    let leadership_system_clone = leadership_system.clone();
    let proxy_service_handler = Arc::new(move || {
        let routing_table = routing_table.clone();
        let leadership_system = leadership_system_clone.clone();
        async move {
            Ok::<_, Error>(service_fn(move |req| {
                proxy_request(routing_table.clone(), req, leadership_system.certificate_state.clone())
            }))
        }
    });
    let proxy_service_handler_clone = proxy_service_handler.clone();
    let proxy_service_http = make_service_fn(move |_| proxy_service_handler_clone.clone()());
    let proxy_service_https = make_service_fn(move |_| proxy_service_handler.clone()());

    let https_incoming = AddrIncoming::bind(&SocketAddr::from(([0, 0, 0, 0], 443))).unwrap();
    let incoming_tls_acceptor = TlsAcceptor::new(https_incoming, leadership_system.certificate_state.clone());
    let http_server_task = tokio::task::spawn(Server::bind(&SocketAddr::from(([0, 0, 0, 0], 80))).serve(proxy_service_http));
    let https_server_task = tokio::task::spawn(Server::builder(incoming_tls_acceptor).serve(proxy_service_https));

    let leader_join_task = leadership_system.start().await?;

    tokio::select! {
        http_server = http_server_task => http_server.unwrap().unwrap(),
        https_server = https_server_task => https_server.unwrap().unwrap(),
        leader_join = leader_join_task => leader_join.unwrap().unwrap(),
    }

    Ok(())
}
