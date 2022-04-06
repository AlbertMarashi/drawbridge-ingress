use futures::join;
use hyper::{Body, Uri, StatusCode};
use std::str::FromStr;
use std::sync::Arc;
use hyper::header::{UPGRADE, HOST};
use hyper::{Request, Response, Client};

use crate::certificate_state::CertificateState;
use crate::kube_config_tracker::RoutingTable;
use crate::{IngressLoadBalancerError, Code};

pub async fn proxy_request(
    rt: Arc<RoutingTable>,
    req: Request<Body>,
    cert_state: Arc<CertificateState>,
) -> Result<Response<Body>, !> {

    let result: Result<Response<Body>, IngressLoadBalancerError> = call_proxy(req, &rt, &cert_state).await;

    match result {
        Ok(response) => Ok(response),
        Err(e) => {
            let mut response = Response::new(format!("Ingress Error\n{:#?}", e).into());
            eprintln!("{:#?}", e);
            *response.status_mut() = StatusCode::BAD_GATEWAY;
            Ok(response)
        }
    }
}

fn forward_uri<B>(forward_url: &str, req: &Request<B>) -> Result<Uri, IngressLoadBalancerError> {
    let path_and_query = match req.uri().query() {
        Some(query) => format!("{}?{}", req.uri().path(), query),
        None => format!("{}", req.uri().path()),
    };

    Uri::from_str(format!("{}{}", forward_url, path_and_query).as_str())
        .map_err(|e| IngressLoadBalancerError::Other(format!("{:#?}", e).into()))
}

pub async fn call_proxy(mut request: Request<Body>, rt: &RoutingTable, cert_state: &CertificateState) -> Result<Response<Body>, IngressLoadBalancerError> {
    let headers = request.headers();

    let host = match (headers.get(HOST), request.uri().authority()) {
        (Some(host), _) => host.to_str().map_err(|_| {
            IngressLoadBalancerError::general(
                Code::NonExistentHost,
                "Could not parse host header",
            )
        })?,
        (_, Some(authority)) => authority.host(),
        (None, None) => {
            eprintln!("No host header or authority in request");
            Err(IngressLoadBalancerError::general(
                Code::NonExistentHost,
                "no host or authority header found",
            ))?
        }
    };

    // get the path from the uri
    let path = request.uri().path();

    if let Some(res) = cert_state.handle_if_challenge(host, path).await {
        // print path
        println!("Matched Challenge: {}{}", host, path);
        return Ok(res);
    }

    // print path
    println!("{} {:?} {}{}", request.method(), request.uri().scheme(), host, path);

    // if the URL is /health-check then return a 200
    if path == "/health-check" {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::OK;
        return Ok(response);
    }

    // get the backend for the host and path
    let backend = rt.get_backend(&host, &path).await?;


    let is_websocket_upgrade = request.headers().contains_key(UPGRADE) && request.headers().get(UPGRADE).unwrap().to_str().unwrap().to_lowercase() == "websocket";

    let client = Client::new();

    if is_websocket_upgrade {
        // is there a cleaner way of proxying the websockets here? maybe by possibly avoiding creating proxy reqs and responses
        let prox_req = {
            let headers = request.headers().clone();

            let mut proxied_ws = Request::builder()
                .uri(forward_uri(&format!("http://{}", backend), &request)?)
                .method(request.method().clone());

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

        // let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        // println!("{}", String::from_utf8_lossy(&body.to_vec()));

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
                    println!("Error when upgrading client or server websockets: {:#?}", e);
                    return;
                },
            };

            println!("proxying request");

            // we need to proxy the client stream to the server stream
            // and vice versa into two different tasks
            let (mut client_read, mut client_write) = tokio::io::split(client_stream);
            let (mut server_read, mut server_write) = tokio::io::split(server_stream);

            let _ = join!{
                tokio::task::spawn(async move {
                    loop {
                        match tokio::io::copy(&mut client_read, &mut server_write).await {
                            _ => break,
                        }
                    }
                }),
                tokio::task::spawn(async move {
                    loop {
                        match tokio::io::copy(&mut server_read, &mut client_write).await {
                            _ => break
                        }
                    }
                })
            };
        });

        return Ok(prox_res);
    }

    // ensure the URI is forwarded correctly
    *request.uri_mut() = forward_uri(&format!("http://{}", &backend), &request)?;
    *request.version_mut() = hyper::Version::HTTP_11;

    let response = client.request(request)
        .await
        .map_err(|e| IngressLoadBalancerError::HyperError(e))?;

    Ok(response)
}
