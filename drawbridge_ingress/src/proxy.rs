use hyper::{Body, Uri, StatusCode};
use std::str::FromStr;
use std::sync::Arc;
use hyper::header::{UPGRADE};
use hyper::{Request, Response, Client};

use crate::certificate_state::CertificateState;
use crate::kube_config_tracker::RoutingTable;
use crate::{IngressLoadBalancerError, Code};

pub async fn proxy_request(
    rt: Arc<RoutingTable>,
    req: Request<Body>,
    cert_state: Arc<CertificateState>,
) -> Result<Response<Body>, !> {
    // http2 uses ":authority" as the host header, and http uses "host"
    // so we need to check both

    let result: Result<Response<Body>, IngressLoadBalancerError> = try {
        let headers = req.headers();
        let host = match (headers.get("host"), headers.get(":authority")) {
            (Some(host), _) => host.to_str().map_err(|_| {
                IngressLoadBalancerError::general(
                    Code::NonExistentHost,
                    "Could not parse host header",
                )
            })?,
            (_, Some(authority)) => authority.to_str().map_err(|_| {
                IngressLoadBalancerError::general(
                    Code::NonExistentHost,
                    "Could not parse authority header",
                )
            })?,
            (None, None) => Err(IngressLoadBalancerError::general(
                Code::NonExistentHost,
                "no host header found",
            ))?
        };

        // get the path from the uri
        let path = req.uri().path();

        // print path
        println!("{} {}", host, path);

        if let Some(res) = cert_state.handle_if_challenge(host, path).await {
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

        call_proxy( &format!("http://{}", backend), req).await?
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


fn forward_uri<B>(forward_url: &str, req: &Request<B>) -> Uri {
    let forward_uri = match req.uri().query() {
        Some(query) => format!("{}{}?{}", forward_url, req.uri().path(), query),
        None => format!("{}{}", forward_url, req.uri().path()),
    };

    Uri::from_str(forward_uri.as_str()).unwrap()
}

pub async fn call_proxy(forward_url: &str, mut request: Request<Body>) -> Result<Response<Body>, IngressLoadBalancerError> {
    let is_websocket_upgrade = request.headers().contains_key(UPGRADE) && request.headers().get(UPGRADE).unwrap().to_str().unwrap().to_lowercase() == "websocket";

    // ensure the URI is forwarded correctly
    *request.uri_mut() = forward_uri(forward_url, &request);

    println!("{:#?}", request);

    let client = Client::new();

    if is_websocket_upgrade {
        let prox_req = {
            let headers = request.headers().clone();
            let uri = request.uri().clone();
            let method = request.method().clone();

            let mut proxied_ws = Request::builder()
                .uri(uri)
                .method(method);

            let prox_headers = proxied_ws.headers_mut().unwrap();
            *prox_headers = headers;
            proxied_ws.body(Body::empty())
                .map_err(|_| IngressLoadBalancerError::general(Code::InternalServerError, "Error creating proxied request"))?
        };

        println!("proxy req {:#?}, request {:#?}", prox_req, request);

        let mut response = client.request(prox_req)
            .await
            .map_err(|e| IngressLoadBalancerError::HyperError(e))?;

        println!("response {:#?}", response);

        let prox_res = {
            let headers = response.headers().clone();
            let status = response.status().clone();
            let version = response.version().clone();


            let mut proxied_ws = Response::builder()
                .status(status)
                .version(version);

            let prox_headers = proxied_ws.headers_mut().unwrap();
            *prox_headers = headers;
            proxied_ws.body(Body::empty())
                .map_err(|_| IngressLoadBalancerError::general(Code::InternalServerError, "Error creating proxied response"))?
        };

        tokio::task::spawn(async move {
            let client_stream = match hyper::upgrade::on(&mut request).await {
                Ok(client_stream) => Ok(client_stream),
                Err(e) => Err(IngressLoadBalancerError::general(Code::WebsocketUpgradeError, format!{"Error when upgrading client websockets: {:#?}", e})),
            };

            let server_stream = match hyper::upgrade::on(&mut response).await {
                Ok(server_stream) => Ok(server_stream),
                Err(e) => Err(IngressLoadBalancerError::general(Code::WebsocketUpgradeError, format!{"Error when upgrading server websockets: {:#?}", e})),
            };

            let (client_stream, server_stream) = match (client_stream, server_stream) {
                (Ok(client_stream), Ok(server_stream)) => (client_stream, server_stream),
                (Err(e), _) | (_, Err(e)) => {
                    println!("Error when upgrading client websockets: {:#?}", e);
                    println!("Error when upgrading server websockets: {:#?}", e);
                    eprintln!("Error creating client or server stream");
                    return;
                },
            };

            // we need to proxy the client stream to the server stream
            // and vice versa into two different tasks
            let (mut client_read, mut client_write) = tokio::io::split(client_stream);
            let (mut server_read, mut server_write) = tokio::io::split(server_stream);

            tokio::task::spawn(async move {
                loop {
                    tokio::io::copy(&mut client_read, &mut server_write).await.unwrap();
                }
            });

            tokio::task::spawn(async move {
                loop {
                    tokio::io::copy(&mut server_read, &mut client_write).await.unwrap();
                }
            });
        });

        println!("Proxying websocket request");

        return Ok(prox_res);
    }

    let response = client.request(request)
        .await
        .map_err(|e| IngressLoadBalancerError::HyperError(e))?;

    Ok(response)
}