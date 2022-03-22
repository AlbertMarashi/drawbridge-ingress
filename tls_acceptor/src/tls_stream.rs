use std::{
    pin::Pin,
    task::{Context, Poll}, sync::Arc,
};

use futures::Future;
use hyper::server::conn::AddrStream;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_rustls::{server::TlsStream, Accept, LazyConfigAcceptor, StartHandshake};

pub enum TLSStreamState<CertRes, CertResFut> {
    AwaitingHandshake(LazyConfigAcceptor<AddrStream>, Arc<CertRes>),
    GettingCert(CertResFut),
    Handshaking(Accept<AddrStream>),
    Stream(TlsStream<AddrStream>),
}

pub struct TLSStream<CertRes, CertResFut> {
    state: TLSStreamState<CertRes, CertResFut>,
}

impl<CertRes, CertResFut> TLSStream<CertRes, CertResFut>
where
    CertRes: Fn(StartHandshake<AddrStream>) -> CertResFut + Unpin,
    CertResFut: Future<Output = Accept<AddrStream>> + Unpin,
{
    pub fn new(io: AddrStream, cert_resolver: Arc<CertRes>) -> Self {
        let acceptor = rustls::server::Acceptor::new().unwrap();
        let lazy_config_acceptor = LazyConfigAcceptor::new(acceptor, io);
        TLSStream {
            state: TLSStreamState::AwaitingHandshake(lazy_config_acceptor, cert_resolver),
        }
    }
}

impl<CertRes, CertResFut> AsyncRead for TLSStream<CertRes, CertResFut>
where
    CertRes: Fn(StartHandshake<AddrStream>) -> CertResFut + Unpin,
    CertResFut: Future<Output = Accept<AddrStream>> + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        let this = self.get_mut();
        match &mut this.state {
            TLSStreamState::AwaitingHandshake(acceptor, cert_resolver) => {
                match Pin::new(acceptor).poll(cx) {
                    Poll::Ready(Ok(start_handshake)) => {
                        let cert_resolver = cert_resolver(start_handshake);
                        this.state = TLSStreamState::GettingCert(cert_resolver);
                        Pin::new(this).poll_read(cx, buf)
                    }
                    Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                    Poll::Pending => Poll::Pending,
                }
            }
            TLSStreamState::GettingCert(cert_resolver) => match Pin::new(cert_resolver).poll(cx) {
                Poll::Ready(acceptor) => {
                    this.state = TLSStreamState::Handshaking(acceptor);
                    Pin::new(this).poll_read(cx, buf)
                }
                Poll::Pending => Poll::Pending,
            },
            TLSStreamState::Handshaking(acceptor) => match Pin::new(acceptor).poll(cx) {
                Poll::Ready(Ok(stream)) => {
                    this.state = TLSStreamState::Stream(stream);
                    Pin::new(this).poll_read(cx, buf)
                }
                Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                Poll::Pending => Poll::Pending,
            },
            TLSStreamState::Stream(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl<CertRes, CertResFut> AsyncWrite for TLSStream<CertRes, CertResFut>
where
    CertRes: Fn(StartHandshake<AddrStream>) -> CertResFut + Unpin,
    CertResFut: Future<Output = Accept<AddrStream>> + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        let this = self.get_mut();
        match &mut this.state {
            TLSStreamState::AwaitingHandshake(acceptor, cert_resolver) => {
                match Pin::new(acceptor).poll(cx) {
                    Poll::Ready(Ok(start_handshake)) => {
                        let cert_resolver = cert_resolver(start_handshake);
                        this.state = TLSStreamState::GettingCert(cert_resolver);
                        Pin::new(this).poll_write(cx, buf)
                    }
                    Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                    Poll::Pending => Poll::Pending,
                }
            }
            TLSStreamState::GettingCert(cert_resolver) => match Pin::new(cert_resolver).poll(cx) {
                Poll::Ready(acceptor) => {
                    this.state = TLSStreamState::Handshaking(acceptor);
                    Pin::new(this).poll_write(cx, buf)
                }
                Poll::Pending => Poll::Pending,
            },
            TLSStreamState::Handshaking(acceptor) => match Pin::new(acceptor).poll(cx) {
                Poll::Ready(Ok(stream)) => {
                    this.state = TLSStreamState::Stream(stream);
                    Pin::new(this).poll_write(cx, buf)
                }
                Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                Poll::Pending => Poll::Pending,
            },
            TLSStreamState::Stream(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        let this = self.get_mut();
        match &mut this.state {
            TLSStreamState::AwaitingHandshake(acceptor, cert_resolver) => {
                match Pin::new(acceptor).poll(cx) {
                    Poll::Ready(Ok(start_handshake)) => {
                        let cert_resolver = cert_resolver(start_handshake);
                        this.state = TLSStreamState::GettingCert(cert_resolver);
                        Pin::new(this).poll_flush(cx)
                    }
                    Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                    Poll::Pending => Poll::Pending,
                }
            }
            TLSStreamState::GettingCert(cert_resolver) => match Pin::new(cert_resolver).poll(cx) {
                Poll::Ready(acceptor) => {
                    this.state = TLSStreamState::Handshaking(acceptor);
                    Pin::new(this).poll_flush(cx)
                }
                Poll::Pending => Poll::Pending,
            },
            TLSStreamState::Handshaking(acceptor) => match Pin::new(acceptor).poll(cx) {
                Poll::Ready(Ok(stream)) => {
                    this.state = TLSStreamState::Stream(stream);
                    Pin::new(this).poll_flush(cx)
                }
                Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                Poll::Pending => Poll::Pending,
            },
            TLSStreamState::Stream(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        let this = self.get_mut();
        match &mut this.state {
            TLSStreamState::AwaitingHandshake(acceptor, cert_resolver) => {
                match Pin::new(acceptor).poll(cx) {
                    Poll::Ready(Ok(start_handshake)) => {
                        let cert_resolver = cert_resolver(start_handshake);
                        this.state = TLSStreamState::GettingCert(cert_resolver);
                        Pin::new(this).poll_shutdown(cx)
                    }
                    Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                    Poll::Pending => Poll::Pending,
                }
            }
            TLSStreamState::GettingCert(cert_resolver) => match Pin::new(cert_resolver).poll(cx) {
                Poll::Ready(acceptor) => {
                    this.state = TLSStreamState::Handshaking(acceptor);
                    Pin::new(this).poll_shutdown(cx)
                }
                Poll::Pending => Poll::Pending,
            },
            TLSStreamState::Handshaking(acceptor) => match Pin::new(acceptor).poll(cx) {
                Poll::Ready(Ok(stream)) => {
                    this.state = TLSStreamState::Stream(stream);
                    Pin::new(this).poll_shutdown(cx)
                }
                Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                Poll::Pending => Poll::Pending,
            },
            TLSStreamState::Stream(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}
