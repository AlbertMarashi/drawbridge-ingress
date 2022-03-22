use std::{sync::Arc, time::Duration};

use congress::{Senator, Peer, RPCNetwork, Request, UserMessage};
use serde::{Deserialize, Serialize};
use tokio::{io::{duplex, DuplexStream}, time::{Instant, timeout}};

#[derive(Clone, Deserialize, Serialize, Debug)]
enum MyReq {
    A,
    B,
}

#[derive(Clone, Deserialize, Serialize, Debug)]
enum MyRes {
    X,
    Y
}

type SenatorType = Senator<MyReq, MyRes, RPCNetwork<MyReq, MyRes, DuplexStream>>;

#[tokio::test]
async fn test_timers() {
    let future_timeout = Instant::now() + Duration::from_secs(1);
    dbg!(future_timeout.duration_since(Instant::now()));
    tokio::time::sleep_until(future_timeout).await;
    dbg!(future_timeout.checked_duration_since(Instant::now()));
}

#[tokio::test]
async fn can_create_rpc_network() {
    let ( a_to_b_stream, b_to_a_stream) = duplex(128);

    let rpc_a = RPCNetwork::new();
    let rpc_b = RPCNetwork::new();

    rpc_a.add_peer(Peer::new(2, a_to_b_stream)).await;
    rpc_b.add_peer(Peer::new(1, b_to_a_stream)).await;

    let a: Arc<SenatorType> = Senator::new(1, rpc_a); // peer 1
    let b: Arc<SenatorType> = Senator::new(2, rpc_b); // peer 2

    a.start();
    b.start();

    // wait for a second
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    let leader_a = a.current_leader.lock().await.expect(&"expected a to have a leader");
    let leader_b = b.current_leader.lock().await.expect(&"expected b to have a leader");

    assert_eq!(leader_a, leader_b);
}

#[tokio::test]
async fn gets_role_updates(){
    let ( a_to_b_stream, b_to_a_stream) = duplex(128);

    let rpc_a = RPCNetwork::new();
    let rpc_b = RPCNetwork::new();

    rpc_a.add_peer(Peer::new(2, a_to_b_stream)).await;
    rpc_b.add_peer(Peer::new(1, b_to_a_stream)).await;

    let a: Arc<SenatorType> = Senator::new(1, rpc_a); // peer 1
    let b: Arc<SenatorType> = Senator::new(2, rpc_b); // peer 2

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    let clone = tx.clone();
    a.on_role(move |role| {
        match role {
            congress::Role::Leader => clone.send(()).unwrap(),
            _ => (),
        }
    }).await;
    b.on_role(move |role| {
        match role {
            congress::Role::Leader => tx.send(()).unwrap(),
            _ => (),
        }
    }).await;

    a.start();
    b.start();

    timeout(Duration::from_millis(1000), rx.recv()).await.unwrap().unwrap();
}



#[tokio::test]
async fn can_push_custom_messages(){
    let ( a_to_b_stream, b_to_a_stream) = duplex(128);

    let rpc_a = RPCNetwork::new();
    let rpc_b = RPCNetwork::new();

    rpc_a.add_peer(Peer::new(2, a_to_b_stream)).await;
    rpc_b.add_peer(Peer::new(1, b_to_a_stream)).await;

    let a: Arc<SenatorType> = Senator::new(1, rpc_a); // peer 1
    let b: Arc<SenatorType> = Senator::new(2, rpc_b); // peer 2

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    a.broadcast_request(Request::Custom(MyReq::A)).await;

    b.on_message(move |msg| {
        match msg {
            UserMessage::Request(MyReq::A) => tx.send(()).unwrap(),
            _ => (),
        }
    }).await;

    a.start();
    b.start();

    timeout(Duration::from_millis(1000), rx.recv()).await.expect("timed out").expect("error receiving");
}
