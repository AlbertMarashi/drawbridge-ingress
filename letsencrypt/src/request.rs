use hyper::{Client, client::HttpConnector};
use hyper_tls::HttpsConnector;

pub fn get_client() -> Client<HttpsConnector<HttpConnector>> {
    let connector = hyper_tls::HttpsConnector::new();
    let client = Client::builder().build::<_, hyper::Body>(connector);
    client
}