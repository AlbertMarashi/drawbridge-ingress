use std::{io::Cursor, sync::Arc};

use congress::{Senator, NodeID, Peer, RPCNetwork};
use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite, duplex, DuplexStream};


#[tokio::test]
async fn can_create_rpc_network() {
    let (a_to_b, b_to_a) = duplex(128);
    let (b_to_a, a_to_b) = duplex(128);

    let a_network = RPCNetwork::new();
    let b_network = RPCNetwork::new();

    a_network.add_client(1, Peer::new(1, a_to_b));
    b_network.add_client(2, Peer::new(2, b_to_a));

    #[derive(Clone, Deserialize, Serialize, Debug)]
    enum State {
        A,
        B,
    }

    #[derive(Clone, Deserialize, Serialize, Debug)]
    enum StateRes {
        X,
        Y
    }

    type SenatorType = Senator<RPCNetwork<DuplexStream>, State, StateRes>;

    let a: Arc<SenatorType> = Senator::new(2, a_network, State::A);
    let b: Arc<SenatorType> = Senator::new(1, b_network, State::A);


}

#[test]
fn can_create_senator() {
    let mut rng = rand::thread_rng();
    let id: NodeID = rng.gen();

    // let senator = Senator::new(id, )
}

struct FakeStream {
    writer: Cursor<Vec<u8>>,
    reader: Cursor<Vec<u8>>,
}

impl FakeStream {
    fn new() -> Self {
        FakeStream {
            writer: Cursor::new(Vec::new()),
            reader: Cursor::new(Vec::new()),
        }
    }
}

impl AsyncRead for FakeStream {
    fn poll_read(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>, buf: &mut tokio::io::ReadBuf<'_>) -> std::task::Poll<std::io::Result<()>> {
        let reader = std::pin::Pin::new(&mut self.reader);
        reader.poll_read(cx, buf)
    }
}

impl AsyncWrite for FakeStream {
    fn poll_flush(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), std::io::Error>> {
        let writer = std::pin::Pin::new(&mut self.writer);
        writer.poll_flush(cx)
    }

    fn poll_shutdown(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), std::io::Error>> {
        let writer = std::pin::Pin::new(&mut self.writer);
        writer.poll_shutdown(cx)
    }

    fn poll_write(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>, buf: &[u8]) -> std::task::Poll<Result<usize, std::io::Error>> {
        let writer = std::pin::Pin::new(&mut self.writer);
        writer.poll_write(cx, buf)
    }

    fn poll_write_vectored(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>, bufs: &[std::io::IoSlice<'_>]) -> std::task::Poll<Result<usize, std::io::Error>> {
        let writer = std::pin::Pin::new(&mut self.writer);
        writer.poll_write_vectored(cx, bufs)
    }
}