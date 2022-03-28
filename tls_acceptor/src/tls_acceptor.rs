use std::{pin::Pin, task::{Poll, Context}, sync::Arc};

use futures::{future::poll_fn, ready};
use hyper::server::{conn::{AddrIncoming, AddrStream}, accept::Accept as HyperAccept};
use rustls::{server::ClientHello, ServerConfig};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio_rustls::{server::TlsStream, LazyConfigAcceptor};

pub struct TlsAcceptor {
    receiver: UnboundedReceiver<Option<TlsStream<AddrStream>>>,
}

#[async_trait::async_trait]
pub trait ResolvesServerConf {
    async fn resolve_server_config(self: Arc<Self>, client_hello: &ClientHello) -> Option<Arc<ServerConfig>>;
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
                                Err(err) => {
                                    eprintln!("tls_acceptor: accept error: {}", err);
                                    None
                                }

                                Ok(handshake) => match resolver.resolve_server_config(&handshake.client_hello()).await {
                                    Some(config) => match handshake.into_stream(config).await {
                                        Ok(stream) => Some(stream),
                                        Err(err) => {
                                            eprintln!("tls_acceptor: handshake error: {}", err);
                                            None
                                        }
                                    }
                                    None => None
                                }
                            };

                            if let Err(e) = sender.send(result) {
                                eprintln!("tls_accceptor: error sending result: {}", e);
                            }
                        });
                    },
                    Some(Err(e)) => eprintln!("tls_accceptor: error accpting incoming: {}", e),
                    None => println!("tls_accceptor: incoming stream closed"),
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
        match ready!(self.get_mut().receiver.poll_recv(cx)) {
            Some(Some(stream)) => Poll::Ready(Some(Ok(stream))),
            _ => Poll::Pending,
        }
    }
}
