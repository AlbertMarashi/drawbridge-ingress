use std::{sync::Arc, time::Duration};

use congress::{Senator, Peer, RPCNetwork, RPC, MessageType, Message, Role};
use futures::join;
use serde::{Deserialize, Serialize};
use tokio::{io::{duplex, DuplexStream}, time::{Instant, timeout}};

#[derive(Clone, Deserialize, Serialize, Debug)]
enum MyMessage {
    A,
    B,
}

type SenatorType = Senator<MyMessage, RPCNetwork<MyMessage, DuplexStream>>;

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

    let rpc_a = RPCNetwork::new(1);
    let rpc_b = RPCNetwork::new(2);

    rpc_a.add_peer(Peer::new(1, 2, a_to_b_stream)).await.unwrap();
    rpc_b.add_peer(Peer::new(1, 1, b_to_a_stream)).await.unwrap();

    let a: Arc<SenatorType> = Senator::new(Duration::from_millis(100), rpc_a); // peer 1
    let b: Arc<SenatorType> = Senator::new(Duration::from_millis(100), rpc_b); // peer 2

    a.start();
    b.start();

    // wait for a second
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    let leader_a = a.current_leader.lock().await.expect(&"expected a to have a leader");
    let leader_b = b.current_leader.lock().await.expect(&"expected b to have a leader");

    assert_eq!(leader_a, leader_b);
}

#[tokio::test]
async fn gets_role_updates(){
    let ( a_to_b_stream, b_to_a_stream) = duplex(128);

    let rpc_a = RPCNetwork::new(1);
    let rpc_b = RPCNetwork::new(2);

    rpc_a.add_peer(Peer::new(1, 2, a_to_b_stream)).await.unwrap();
    rpc_b.add_peer(Peer::new(1, 1, b_to_a_stream)).await.unwrap();

    let a: Arc<SenatorType> = Senator::new(Duration::from_millis(100), rpc_a); // peer 1
    let b: Arc<SenatorType> = Senator::new(Duration::from_millis(100), rpc_b); // peer 2

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

    let rpc_a = RPCNetwork::new(1);
    let rpc_b = RPCNetwork::new(2);

    rpc_a.add_peer(Peer::new(1, 2, a_to_b_stream)).await.unwrap();
    rpc_b.add_peer(Peer::new(1, 1, b_to_a_stream)).await.unwrap();

    let a: Arc<SenatorType> = Senator::new(Duration::from_millis(100), rpc_a); // peer 1
    let b: Arc<SenatorType> = Senator::new(Duration::from_millis(100), rpc_b); // peer 2

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    a.broadcast_message(MessageType::Custom(MyMessage::A)).await;

    b.on_message(move |msg| {
        match msg {
            Message {
                msg: MessageType::Custom(MyMessage::A),
                ..
            } => tx.send(()).unwrap(),
            _ => (),
        }
    }).await;

    a.start();
    b.start();

    timeout(Duration::from_millis(1000), rx.recv()).await.expect("timed out").expect("error receiving");
}

#[tokio::test]
async fn deleting_a_peer_works() {
    let ( a_to_b_stream, b_to_a_stream) = duplex(128);

    let rpc_a = RPCNetwork::new(1);
    let rpc_b = RPCNetwork::new(2);

    rpc_a.add_peer(Peer::new(1, 2, a_to_b_stream)).await.unwrap();
    rpc_b.add_peer(Peer::new(1, 1, b_to_a_stream)).await.unwrap();

    let a: Arc<SenatorType> = Senator::new(Duration::from_millis(100), rpc_a.clone()); // peer 1
    let b: Arc<SenatorType> = Senator::new(Duration::from_millis(100), rpc_b.clone()); // peer 2

    a.start();
    b.start();

    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    rpc_a.remove_peer(2).await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    assert!(rpc_a.members().await.len() == 0);
    assert!(rpc_b.members().await.len() == 0);
}


#[tokio::test]
async fn duplicate_peers_automatically_resolve() {
    let ( a_to_b_stream1, b_to_a_stream1) = duplex(128);
    let ( a_to_b_stream2, b_to_a_stream2) = duplex(128);

    let rpc_a = RPCNetwork::new(1);
    let rpc_b = RPCNetwork::new(2);

    let peer_1_1_handle = rpc_a.add_peer(Peer::new(1, 2, a_to_b_stream1)).await.unwrap();
    let peer_2_1_handle = rpc_b.add_peer(Peer::new(1, 1, b_to_a_stream1)).await.unwrap();

    let peer_1_2_handle = rpc_a.add_peer(Peer::new(2, 2, a_to_b_stream2)).await.unwrap();
    let peer_2_2_handle = rpc_b.add_peer(Peer::new(2, 1, b_to_a_stream2)).await.unwrap();

    assert_eq!(rpc_a.members().await.len(), 1);
    assert_eq!(rpc_b.members().await.len(), 1);

    let a: Arc<SenatorType> = Senator::new(Duration::from_millis(100), rpc_a.clone()); // peer 1
    let b: Arc<SenatorType> = Senator::new(Duration::from_millis(100), rpc_b.clone()); // peer 2

    a.start();
    b.start();

    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    assert_eq!(rpc_a.members().await.len(), 1);
    assert_eq!(rpc_b.members().await.len(), 1);

    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    rpc_a.remove_peer(2).await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    assert_eq!(rpc_a.members().await.len(), 0);
    assert_eq!(rpc_b.members().await.len(), 0);

    let fut = async move { join!(peer_1_1_handle, peer_1_2_handle, peer_2_1_handle, peer_2_2_handle) };

    let _ = timeout(Duration::from_millis(1000), fut).await.unwrap();
}


#[tokio::test]
async fn duplicate_peers_automatically_resolve_when_established_by_is_inverted() {
    let ( a_to_b_stream1, b_to_a_stream1) = duplex(128);
    let ( a_to_b_stream2, b_to_a_stream2) = duplex(128);

    let rpc_a = RPCNetwork::new(1);
    let rpc_b = RPCNetwork::new(2);

    let peer_1_1_handle = rpc_a.add_peer(Peer::new(2, 2, a_to_b_stream1)).await.unwrap();
    let peer_2_1_handle = rpc_b.add_peer(Peer::new(2, 1, b_to_a_stream1)).await.unwrap();

    let peer_1_2_handle = rpc_a.add_peer(Peer::new(1, 2, a_to_b_stream2)).await.unwrap();
    let peer_2_2_handle = rpc_b.add_peer(Peer::new(1, 1, b_to_a_stream2)).await.unwrap();


    let a: Arc<SenatorType> = Senator::new(Duration::from_millis(100), rpc_a.clone()); // peer 1
    let b: Arc<SenatorType> = Senator::new(Duration::from_millis(100), rpc_b.clone()); // peer 2

    a.start();
    b.start();

    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    assert_eq!(rpc_a.members().await.len(), 1);
    assert_eq!(rpc_b.members().await.len(), 1);

    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    rpc_a.remove_peer(2).await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    assert_eq!(rpc_a.members().await.len(), 0);
    assert_eq!(rpc_b.members().await.len(), 0);

    let fut = async move { join!(peer_1_1_handle, peer_1_2_handle, peer_2_1_handle, peer_2_2_handle) };

    let _ = timeout(Duration::from_millis(1000), fut).await.unwrap();
}

#[tokio::test]
async fn setting_a_timeout_delays_leadership() {
    let ( a_to_b_stream, b_to_a_stream) = duplex(128);

    let rpc_a = RPCNetwork::new(1);
    let rpc_b = RPCNetwork::new(2);

    let _peer_1_1_handle = rpc_a.add_peer(Peer::new(1, 2, a_to_b_stream)).await.unwrap();
    let _peer_2_1_handle = rpc_b.add_peer(Peer::new(1, 1, b_to_a_stream)).await.unwrap();

    let a: Arc<SenatorType> = Senator::new(Duration::from_millis(2000), rpc_a.clone()); // peer 1
    let b: Arc<SenatorType> = Senator::new(Duration::from_millis(2000), rpc_b.clone()); // peer 2

    a.start();
    b.start();

    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    assert_eq!(rpc_a.members().await.len(), 1);
    assert_eq!(rpc_b.members().await.len(), 1);

    // assert we are not a leader
    if let Role::Leader = *a.role.lock().await {
        panic!("a is leader");
    }
    if let Role::Leader = *b.role.lock().await {
        panic!("b is leader");
    }

    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
}