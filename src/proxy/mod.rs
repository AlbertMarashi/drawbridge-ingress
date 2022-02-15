use hyper::{Body, Uri};
use std::net::IpAddr;
use std::str::FromStr;
use hyper::header::{ HeaderValue, UPGRADE};
use hyper::{Request, Response, Client};

use crate::{IngressLoadBalancerError, Code};


fn forward_uri<B>(forward_url: &str, req: &Request<B>) -> Uri {
    let forward_uri = match req.uri().query() {
        Some(query) => format!("{}{}?{}", forward_url, req.uri().path(), query),
        None => format!("{}{}", forward_url, req.uri().path()),
    };

    Uri::from_str(forward_uri.as_str()).unwrap()
}


fn create_proxied_request<B>(client_ip: IpAddr, forward_url: &str, mut request: Request<B>) -> Request<B> {
    let x_forwarded_for_header_name = "x-forwarded-for";

    *request.uri_mut() = forward_uri(forward_url, &request);

    // Add forwarding information in the headers
    if !request.headers().contains_key(x_forwarded_for_header_name) {
        request.headers_mut().insert(x_forwarded_for_header_name, HeaderValue::from_str(client_ip.to_string().as_str()).unwrap());
    }

    request
}

pub async fn call(client_ip: IpAddr, forward_uri: &str, request: Request<Body>) -> Result<Response<Body>, IngressLoadBalancerError> {

    let is_websocket_upgrade = request.headers().contains_key(UPGRADE) && request.headers().get(UPGRADE).unwrap().to_str().unwrap().to_lowercase() == "websocket";

	let mut request = create_proxied_request(client_ip, forward_uri, request);

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