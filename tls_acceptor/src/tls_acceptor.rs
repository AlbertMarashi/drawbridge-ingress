use std::{pin::Pin, task::{Poll, Context}, sync::Arc};

use futures::{future::poll_fn};
use hyper::server::{conn::{AddrIncoming, AddrStream}, accept::Accept as HyperAccept};
use rustls::{server::ClientHello, ServerConfig};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio_rustls::{server::TlsStream, LazyConfigAcceptor};

pub struct TlsAcceptor {
    receiver: UnboundedReceiver<Result<TlsStream<AddrStream>, std::io::Error>>,
}

#[async_trait::async_trait]
pub trait ResolvesServerConf {
    async fn resolve_server_config(&self, client_hello: &ClientHello) -> Option<Arc<ServerConfig>>;
}

impl TlsAcceptor {
    pub fn new <R: ResolvesServerConf + Send + Sync + 'static> (mut incoming: AddrIncoming, resolver: Arc<R>) -> TlsAcceptor {
        let (sender, receiver) = unbounded_channel();
        tokio::task::spawn(async move {
            loop {
                let resolver = resolver.clone();
                let sender = sender.clone();
                match poll_fn(|ctx| Pin::new(&mut incoming).poll_accept(ctx)).await {
                    Some(Ok(stream)) => {
                        tokio::task::spawn(async move {
                            let acceptor = rustls::server::Acceptor::new().unwrap();
                            let result = match LazyConfigAcceptor::new(acceptor, stream).await {
                                Err(err) => Err(err),
                                Ok(handshake) => match resolver.resolve_server_config(&handshake.client_hello()).await {
                                    Some(config) => handshake.into_stream(config).await,
                                    None => Err(std::io::Error::new(std::io::ErrorKind::Other, match handshake.client_hello().server_name() {
                                        Some(name) => format!("No server config found for server name: {}", name),
                                        None => "No server config found, because no host was provided".to_string(),
                                    })),
                                }
                            };

                            if let Err(e) = sender.send(result) {
                                eprintln!("error sending result: {}", e);
                            }
                        });
                    },
                    Some(Err(e)) => {
                        if let Err(e) = sender.send(Err(e)) {
                            eprintln!("error sending result: {}", e);
                        }
                    },
                    None => println!("incoming stream closed"),
                };


            }
        });

        TlsAcceptor {
            receiver
        }
    }
}

impl HyperAccept for TlsAcceptor {
    type Conn = TlsStream<AddrStream>;
    type Error = std::io::Error;

    fn poll_accept(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        self.get_mut().receiver.poll_recv(cx)
    }
}
