use std::{pin::Pin, task::{Poll, Context}, sync::Arc};

use futures::Future;
use hyper::server::{conn::{AddrIncoming, AddrStream}};
use tokio_rustls::{StartHandshake, Accept};

use crate::tls_stream::TLSStream;

pub struct TlsAcceptor<CertRes> {
    resolver: Arc<CertRes>,
    incoming: AddrIncoming,
}

impl<CertRes> TlsAcceptor<CertRes> {
    pub fn new(incoming: AddrIncoming, resolver: CertRes) -> TlsAcceptor<CertRes> {
        TlsAcceptor {
            incoming,
            resolver: Arc::new(resolver),
        }
    }
}

impl<CertRes, CertResFut> hyper::server::accept::Accept for TlsAcceptor<CertRes>
where
    CertRes: Fn(StartHandshake<AddrStream>) -> CertResFut + Unpin,
    CertResFut: Future<Output = Accept<AddrStream>> + Unpin,
{
    type Conn = TLSStream<CertRes, CertResFut>;
    type Error = std::io::Error;

    fn poll_accept(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Self::Conn, Self::Error>>> {
        let pin = self.get_mut();
        match Pin::new(&mut pin.incoming).poll_accept(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Ok(sock))) => Poll::Ready(Some(Ok(TLSStream::new(sock, pin.resolver.clone())))),
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e))),
            Poll::Ready(None) => Poll::Ready(None),
        }
    }
}
