#![allow(clippy::trivial_regex)]

use hyper::service::make_service_fn;
use hyper::{service::service_fn, Body, Method, Request, Response, Server};
use lazy_static::lazy_static;
use std::convert::Infallible;
use std::net::SocketAddr;
use tokio::task::JoinHandle;

lazy_static! {
    static ref RE_URL: regex::Regex = regex::Regex::new("<URL>").unwrap();
}

pub fn get_directory(url: &str) -> Response<Body> {
    const BODY: &str = r#"{
    "keyChange": "<URL>/acme/key-change",
    "newAccount": "<URL>/acme/new-acct",
    "newNonce": "<URL>/acme/new-nonce",
    "newOrder": "<URL>/acme/new-order",
    "revokeCert": "<URL>/acme/revoke-cert",
    "meta": {
        "caaIdentities": [
        "testdir.org"
        ]
    }
    }"#;
    Response::new(Body::from(RE_URL.replace_all(BODY, url)))
}

pub fn head_new_nonce() -> Response<Body> {
    Response::builder()
        .status(204)
        .header(
            "Replay-Nonce",
            "8_uBBV3N2DBRJczhoiB46ugJKUkUHxGzVe6xIMpjHFM",
        )
        .body(Body::empty())
        .unwrap()
}

pub fn post_new_acct(url: &str) -> Response<Body> {
    const BODY: &str = r#"{
    "id": 7728515,
    "key": {
        "use": "sig",
        "kty": "EC",
        "crv": "P-256",
        "alg": "ES256",
        "x": "ttpobTRK2bw7ttGBESRO7Nb23mbIRfnRZwunL1W6wRI",
        "y": "h2Z00J37_2qRKH0-flrHEsH0xbit915Tyvd2v_CAOSk"
    },
    "contact": [
        "mailto:foo@bar.com"
    ],
    "initialIp": "90.171.37.12",
    "createdAt": "2018-12-31T17:15:40.399104457Z",
    "status": "valid"
    }"#;
    let location: String = RE_URL.replace_all("<URL>/acme/acct/7728515", url).into();
    Response::builder()
        .status(201)
        .header("Location", location)
        .body(Body::from(BODY))
        .unwrap()
}

pub fn post_new_order(url: &str) -> Response<Body> {
    const BODY: &str = r#"{
    "status": "pending",
    "expires": "2019-01-09T08:26:43.570360537Z",
    "identifiers": [
        {
        "type": "dns",
        "value": "acmetest.example.com"
        }
    ],
    "authorizations": [
        "<URL>/acme/authz/YTqpYUthlVfwBncUufE8IRWLMSRqcSs"
    ],
    "finalize": "<URL>/acme/finalize/7738992/18234324"
    }"#;
    let location: String = RE_URL
        .replace_all("<URL>/acme/order/YTqpYUthlVfwBncUufE8", url)
        .into();
    Response::builder()
        .status(201)
        .header("Location", location)
        .body(Body::from(RE_URL.replace_all(BODY, url)))
        .unwrap()
}

pub fn post_get_order(url: &str) -> Response<Body> {
    const BODY: &str = r#"{
    "status": "valid",
    "expires": "2019-01-09T08:26:43.570360537Z",
    "identifiers": [
        {
        "type": "dns",
        "value": "acmetest.example.com"
        }
    ],
    "authorizations": [
        "<URL>/acme/authz/YTqpYUthlVfwBncUufE8IRWLMSRqcSs"
    ],
    "finalize": "<URL>/acme/finalize/7738992/18234324",
    "certificate": "<URL>/acme/cert/fae41c070f967713109028"
    }"#;
    let b = RE_URL.replace_all(BODY, url).to_string();
    Response::builder().status(200).body(Body::from(b)).unwrap()
}

pub fn post_authz(url: &str) -> Response<Body> {
    const BODY: &str = r#"{
        "identifier": {
            "type": "dns",
            "value": "acmetest.algesten.se"
        },
        "status": "ready",
        "expires": "2019-01-09T08:26:43Z",
        "challenges": [
        {
            "type": "http-01",
            "status": "pending",
            "url": "<URL>/acme/challenge/YTqpYUthlVfwBncUufE8IRWLMSRqcSs/216789597",
            "token": "MUi-gqeOJdRkSb_YR2eaMxQBqf6al8dgt_dOttSWb0w"
        },
        {
            "type": "tls-alpn-01",
            "status": "pending",
            "url": "<URL>/acme/challenge/YTqpYUthlVfwBncUufE8IRWLMSRqcSs/216789598",
            "token": "WCdRWkCy4THTD_j5IH4ISAzr59lFIg5wzYmKxuOJ1lU"
        },
        {
            "type": "dns-01",
            "status": "pending",
            "url": "<URL>/acme/challenge/YTqpYUthlVfwBncUufE8IRWLMSRqcSs/216789599",
            "token": "RRo2ZcXAEqxKvMH8RGcATjSK1KknLEUmauwfQ5i3gG8"
        }
        ]
    }"#;
    Response::builder()
        .status(201)
        .body(Body::from(RE_URL.replace_all(BODY, url)))
        .unwrap()
}

pub fn post_finalize(_url: &str) -> Response<Body> {
    Response::builder().status(200).body(Body::empty()).unwrap()
}

pub fn post_certificate(_url: &str) -> Response<Body> {
    Response::builder()
        .status(200)
        .body("CERT HERE".into())
        .unwrap()
}

pub fn post_challenge(url: &str) -> Response<Body> {
    const BODY: &str = r#"{
        "status": "valid",
        "token": "MUi-gqeOJdRkSb_YR2eaMxQBqf6al8dgt_dOttSWb0w",
        "url": "<URL>/acme/challenge/YTqpYUthlVfwBncUufE8IRWLMSRqcSs/216789597",
        "type": "http-01"
    }"#;

    Response::builder()
        .status(200)
        .body(Body::from(RE_URL.replace_all(BODY, url)))
        .unwrap()
}

pub fn route_request(req: Request<Body>, url: &str) -> Response<Body> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/directory") => get_directory(url),
        (&Method::HEAD, "/acme/new-nonce") => head_new_nonce(),
        (&Method::POST, "/acme/new-acct") => post_new_acct(url),
        (&Method::POST, "/acme/new-order") => post_new_order(url),
        (&Method::POST, "/acme/order/YTqpYUthlVfwBncUufE8") => post_get_order(url),
        (&Method::POST, "/acme/authz/YTqpYUthlVfwBncUufE8IRWLMSRqcSs") => post_authz(url),
        (&Method::POST, "/acme/finalize/7738992/18234324") => post_finalize(url),
        (&Method::POST, "/acme/cert/fae41c070f967713109028") => post_certificate(url),
        (&Method::POST, "/acme/challenge/YTqpYUthlVfwBncUufE8IRWLMSRqcSs/216789597") => post_challenge(url),
        (_, _) => Response::builder().status(404).body(Body::empty()).unwrap(),
    }
}

pub fn with_directory_server() -> (JoinHandle<()>, String) {
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));

    let url = format!("http://127.0.0.1:3000");
    let dir_url = format!("{}/directory", url);

    // And a MakeService to handle each connection...
    let make_service = make_service_fn(move |_conn| {
        let url = url.clone();
        async {
            Ok::<_, Infallible>(service_fn(move |req| {
                let url = url.clone();
                async move { Ok::<_, Infallible>(route_request(req, &url)) }
            }))
        }
    });

    // start a task to run the server until the task is dropped
    let server = Server::bind(&addr).serve(make_service);
    let handle = tokio::task::spawn(async move {
        if let Err(e) = server.await {
            eprintln!("Server error: {}", e);
        }
    });

    (handle, dir_url)
}
