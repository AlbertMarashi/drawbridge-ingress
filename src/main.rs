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
#![feature(never_type)]

use std::{sync::Arc};
use std::net::SocketAddr;
use hyper::server::conn::AddrStream;
use hyper::{Body, Request, Response, StatusCode, Server};
use hyper::service::{make_service_fn, service_fn};
use kube_config_tracker::RoutingTable;

mod kube_config_tracker;
mod proxy;

#[derive(Debug)]
pub enum Code {
    NonExistentHost,
    CouldNotReachBackend,
    WebsocketUpgradeError,
    HttpError,
    InternalServerError,
}

#[derive(Debug)]
pub enum IngressLoadBalancerError {
    General(Code, Box<str>),
    HyperError(hyper::Error),
}

impl IngressLoadBalancerError {
    pub fn general_as_result<M>(code: Code, msg: M) -> Result<!, Self>
    where M: Into<Box<str>> {
        Err(Self::General(code, msg.into()))
    }

    pub fn general<M> (code: Code, msg: M) -> Self
    where M: Into<Box<str>> {
        Self::General(code, msg.into())
    }
}

type Error = Box<dyn std::error::Error + Send + Sync + 'static>;

#[tokio::main]
async fn main() {

    let rt = Arc::new(kube_config_tracker::RoutingTable::new());

    // start a task which listens for changes to the kubernetes api
    // and updates the routing table accordingly
    tokio::spawn(rt.clone().start_watching());

    let addr = SocketAddr::from(([0,0,0,0], 80));

    let proxy_service = make_service_fn(move |socket: &AddrStream| {
        let rt = rt.clone();
        let remote_addr = socket.remote_addr();
        async move {
            Ok::<_, Error>(service_fn(move |req| {
                proxy(rt.clone(), req, remote_addr.clone())
            }))
        }
    });

    let server = Server::bind(&addr).serve(proxy_service);

    if let Err(e) = server.await {
        println!("server error: {}", e);
    }
}

async fn proxy (rt: Arc<RoutingTable>, req: Request<Body>, remote_addr: SocketAddr) -> Result<Response<Body>, !> {
    let result = proxy_request(rt, req, remote_addr).await;

    match result {
        Ok(response) => Ok(response),
        Err(e) => {
            let mut response = Response::new(format!("Ingress Error\n{:#?}", e).into());
            *response.status_mut() = StatusCode::BAD_GATEWAY;
            Ok(response)
        }
    }
}

async fn proxy_request (rt: Arc<RoutingTable>, req: Request<Body>, remote_addr: SocketAddr) -> Result<Response<Body>, IngressLoadBalancerError> {
    // http2 uses ":authority" as the host header, and http uses "host"
    // so we need to check both

    let host = {
        let host = req.headers()
            .get("host");
        let authority = req.headers()
            .get(":authority");

        if let Some(h) = host {
            h.to_str().map_err(|_| IngressLoadBalancerError::general(Code::NonExistentHost, "Could not parse host header"))?
        } else if let Some(a) = authority {
            a.to_str().map_err(|_| IngressLoadBalancerError::general(Code::NonExistentHost, "Could not parse host header"))?
        } else {
            Err(IngressLoadBalancerError::general(Code::NonExistentHost, "no host header found"))?
        }
    };


    // get the path from the uri
    let path = req
        .uri()
        .path();

    // print path
    println!("{} {}", host, path);

    // if the URL is /health-check then return a 200
    if path == "/health-check" {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::OK;
        return Ok(response);
    }

    // get the backend for the host and path
    let backend = rt.get_backend(&host, &path)
        .await?;

    proxy::call(remote_addr.ip(), &format!("http://{}", backend), req).await
}
