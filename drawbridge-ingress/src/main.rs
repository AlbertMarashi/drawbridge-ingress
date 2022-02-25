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
//!
//! ## LetsEncrypt Process
//! - we need a letsencrypt account in order to generate certificates.
//!     - account creation api is rate limited, so we want to reuse accounts.
//!     - we can store these accounts in kubernetes secrets.
//! - we need a letsencrypt certificate for each host configured in the ingress.
//!    - we can store these certificates in kubernetes secrets.
//!
#![feature(never_type)]
#![feature(try_blocks)]

use hyper::server::conn::{AddrIncoming, AddrStream};
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Request, Response, Server, StatusCode};
use kube_config_tracker::RoutingTable;
use letsencrypt_system::LetsEncrypt;
use std::net::SocketAddr;
use std::sync::Arc;
use tls::{TlsAcceptor, TlsStream};

#[macro_use]
extern crate serde;
#[macro_use]
extern crate async_trait;

mod kube_config_tracker;
mod letsencrypt_system;
mod proxy;
mod tls;
mod raft;

#[derive(Debug)]
pub enum Code {
    NonExistentHost,
    CouldNotReachBackend,
    WebsocketUpgradeError,
    HttpError,
    InternalServerError,
    TLSError,
}

impl std::fmt::Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Code::NonExistentHost => write!(f, "NonExistentHost"),
            Code::CouldNotReachBackend => write!(f, "CouldNotReachBackend"),
            Code::WebsocketUpgradeError => write!(f, "WebsocketUpgradeError"),
            Code::HttpError => write!(f, "HttpError"),
            Code::InternalServerError => write!(f, "InternalServerError"),
            Code::TLSError => write!(f, "TLSError"),
        }
    }
}

#[derive(Debug)]
pub enum IngressLoadBalancerError {
    General(Code, Box<str>),
    HyperError(hyper::Error),
}

impl std::fmt::Display for IngressLoadBalancerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IngressLoadBalancerError::General(code, msg) => write!(f, "Error: {}: {}", code, msg),
            IngressLoadBalancerError::HyperError(err) => write!(f, "Error: {}", err),
        }
    }
}

impl std::error::Error for IngressLoadBalancerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            IngressLoadBalancerError::General(_, _) => None,
            IngressLoadBalancerError::HyperError(err) => Some(err),
        }
    }
}

impl IngressLoadBalancerError {
    pub fn general_as_result<M>(code: Code, msg: M) -> Result<!, Self>
    where
        M: Into<Box<str>>,
    {
        Err(Self::General(code, msg.into()))
    }

    pub fn general<M>(code: Code, msg: M) -> Self
    where
        M: Into<Box<str>>,
    {
        Self::General(code, msg.into())
    }
}

type Error = Box<dyn std::error::Error + Send + Sync + 'static>;

#[tokio::main]
async fn main() {
    let routing_table = Arc::new(RoutingTable::new());
    let lets_encrypt = Arc::new(LetsEncrypt::setup(routing_table.clone()).await);

    // start a task which listens for changes to the kubernetes api
    // and updates the routing table accordingly
    tokio::spawn(routing_table.clone().start_watching());
    let lets_encrypt_clone = lets_encrypt.clone();

    routing_table
        .subscribe(Box::new(move || {
            let lets_encrypt_clone = lets_encrypt_clone.clone();

            tokio::task::spawn(async move {
                lets_encrypt_clone.check_for_new_certificates().await;
            });
        }))
        .await;

    let lets_encrypt_clone = lets_encrypt.clone();
    let rt = routing_table.clone();

    let proxy_service_http = make_service_fn(move |socket: &AddrStream| {
        let rt = rt.clone();
        let lets_encrypt = lets_encrypt_clone.clone();
        let remote_addr = socket.remote_addr();
        async move {
            Ok::<_, Error>(service_fn(move |req| {
                proxy_request(rt.clone(), req, remote_addr.clone(), lets_encrypt.clone())
            }))
        }
    });

    let lets_encrypt_clone = lets_encrypt.clone();
    let rt = routing_table.clone();

    let proxy_service_https = make_service_fn(move |socket: &TlsStream| {
        let rt = rt.clone();
        let lets_encrypt = lets_encrypt_clone.clone();
        let remote_addr = socket.remote_addr();
        async move {
            Ok::<_, Error>(service_fn(move |req| {
                proxy_request(rt.clone(), req, remote_addr.clone(), lets_encrypt.clone())
            }))
        }
    });

    let tls_cfg = {
        let mut cfg = rustls::ServerConfig::builder()
            .with_safe_default_cipher_suites()
            .with_safe_default_kx_groups()
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_cert_resolver(lets_encrypt.clone());

        cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        Arc::new(cfg)
    };

    let addr_http = SocketAddr::from(([0, 0, 0, 0], 80));
    let addr_https = SocketAddr::from(([0, 0, 0, 0], 443));

    let https_incoming = AddrIncoming::bind(&addr_https).unwrap();
    let https_server =
        Server::builder(TlsAcceptor::new(tls_cfg, https_incoming)).serve(proxy_service_https);

    let http_server = Server::bind(&addr_http).serve(proxy_service_http);

    let servers_fut = futures::future::join(http_server, https_server);

    let (_, _) = servers_fut.await;
}

async fn proxy_request(
    rt: Arc<RoutingTable>,
    req: Request<Body>,
    remote_addr: SocketAddr,
    lets_encrypt: Arc<LetsEncrypt>,
) -> Result<Response<Body>, !> {
    // http2 uses ":authority" as the host header, and http uses "host"
    // so we need to check both

    let result: Result<Response<Body>, IngressLoadBalancerError> = try {
        let host = {
            let host = req.headers().get("host");
            let authority = req.headers().get(":authority");

            if let Some(h) = host {
                h.to_str().map_err(|_| {
                    IngressLoadBalancerError::general(
                        Code::NonExistentHost,
                        "Could not parse host header",
                    )
                })?
            } else if let Some(a) = authority {
                a.to_str().map_err(|_| {
                    IngressLoadBalancerError::general(
                        Code::NonExistentHost,
                        "Could not parse host header",
                    )
                })?
            } else {
                Err(IngressLoadBalancerError::general(
                    Code::NonExistentHost,
                    "no host header found",
                ))?
            }
        };

        // get the path from the uri
        let path = req.uri().path();

        // print path
        println!("{} {}", host, path);

        if let Some(res) = lets_encrypt.handle_if_challenge(host, path).await {
            return Ok(res);
        }

        // if the URL is /health-check then return a 200
        if path == "/health-check" {
            let mut response = Response::new(Body::empty());
            *response.status_mut() = StatusCode::OK;
            return Ok(response);
        }

        // get the backend for the host and path
        let backend = rt.get_backend(&host, &path).await?;

        proxy::call(remote_addr.ip(), &format!("http://{}", backend), req).await?
    };

    match result {
        Ok(response) => Ok(response),
        Err(e) => {
            let mut response = Response::new(format!("Ingress Error\n{:#?}", e).into());
            *response.status_mut() = StatusCode::BAD_GATEWAY;
            Ok(response)
        }
    }
}
