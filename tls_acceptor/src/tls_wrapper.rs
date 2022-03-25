use std::{
    pin::Pin,
    task::{Context, Poll}, sync::Arc,
};

use futures::Future;
use hyper::server::conn::AddrStream;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_rustls::{server::TlsStream, Accept, LazyConfigAcceptor, StartHandshake};

pub enum TLSWrapperState<CertRes, CertResFut> {
    AwaitingHandshake(LazyConfigAcceptor<AddrStream>, Arc<CertRes>),
    GettingCert(CertResFut),
    Handshaking(Accept<AddrStream>),
    Stream(TlsStream<AddrStream>),
}

pub struct TLSWrapper<CertRes, CertResFut> {
    state: TLSWrapperState<CertRes, CertResFut>,
}

impl<CertRes, CertResFut> TLSWrapper<CertRes, CertResFut>
where
    CertRes: Fn(StartHandshake<AddrStream>) -> CertResFut + Unpin,
    CertResFut: Future<Output = Option<Accept<AddrStream>>> + Unpin,
{
    pub fn new(io: AddrStream, cert_resolver: Arc<CertRes>) -> Self {
        let acceptor = rustls::server::Acceptor::new().unwrap();
        let lazy_config_acceptor = LazyConfigAcceptor::new(acceptor, io);
        TLSWrapper {
            state: TLSWrapperState::AwaitingHandshake(lazy_config_acceptor, cert_resolver),
        }
    }
}

impl<CertRes, CertResFut> AsyncRead for TLSWrapper<CertRes, CertResFut>
where
    CertRes: Fn(StartHandshake<AddrStream>) -> CertResFut + Unpin,
    CertResFut: Future<Output = Option<Accept<AddrStream>>> + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        let this = self.get_mut();
        match &mut this.state {
            TLSWrapperState::AwaitingHandshake(..) => Poll::Pending,
            TLSWrapperState::GettingCert(..) => Poll::Pending,
            TLSWrapperState::Handshaking(..) => Poll::Pending,
            TLSWrapperState::Stream(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl<CertRes, CertResFut> AsyncWrite for TLSWrapper<CertRes, CertResFut>
where
    CertRes: Fn(StartHandshake<AddrStream>) -> CertResFut + Unpin,
    CertResFut: Future<Output = Option<Accept<AddrStream>>> + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        let this = self.get_mut();
        match &mut this.state {
            TLSWrapperState::AwaitingHandshake(..) => Poll::Pending,
            TLSWrapperState::GettingCert(..) => Poll::Pending,
            TLSWrapperState::Handshaking(..) => Poll::Pending,
            TLSWrapperState::Stream(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        let this = self.get_mut();
        match &mut this.state {
            TLSWrapperState::AwaitingHandshake(..) => Poll::Pending,
            TLSWrapperState::GettingCert(..) => Poll::Pending,
            TLSWrapperState::Handshaking(..) => Poll::Pending,
            TLSWrapperState::Stream(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        let this = self.get_mut();
        match &mut this.state {
            TLSWrapperState::AwaitingHandshake(..) => Poll::Pending,
            TLSWrapperState::GettingCert(..) => Poll::Pending,
            TLSWrapperState::Handshaking(..) => Poll::Pending,
            TLSWrapperState::Stream(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}
