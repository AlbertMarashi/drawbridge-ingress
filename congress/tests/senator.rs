use std::{sync::Arc, time::Duration};

use congress::{Senator, Peer, RPCNetwork, RPC, MessageType, Message, Role};
use serde::{Deserialize, Serialize};
use tokio::{io::{duplex, DuplexStream}, time::{timeout}, sync::{mpsc}};

#[derive(Clone, Deserialize, Serialize, Debug)]
enum MyMessage {
    A,
    B,
}

type SenatorType = Senator<MyMessage, RPCNetwork<MyMessage, DuplexStream>>;

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

    let a: Arc<SenatorType> = Senator::new(Duration::from_millis(0), rpc_a.clone()); // peer 1
    let b: Arc<SenatorType> = Senator::new(Duration::from_millis(0), rpc_b.clone()); // peer 2

    a.start();
    b.start();

    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    rpc_a.remove_peer(2).await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;

    assert!(rpc_a.members().await.len() == 0);
    assert!(rpc_b.members().await.len() == 0);

    // assert that both become leaders of themselves
    assert_eq!(a.current_leader.lock().await.unwrap(), a.id);
    assert_eq!(b.current_leader.lock().await.unwrap(), b.id);
}


#[tokio::test]
async fn duplicate_peers_automatically_resolve() {
    let ( a_to_b_stream1, b_to_a_stream1) = duplex(128);
    let ( a_to_b_stream2, b_to_a_stream2) = duplex(128);

    let rpc_a = RPCNetwork::new(1);
    let rpc_b = RPCNetwork::new(2);

    rpc_a.add_peer(Peer::new(1, 2, a_to_b_stream1)).await.unwrap();
    rpc_b.add_peer(Peer::new(1, 1, b_to_a_stream1)).await.unwrap();

    rpc_a.add_peer(Peer::new(2, 2, a_to_b_stream2)).await.unwrap();
    rpc_b.add_peer(Peer::new(2, 1, b_to_a_stream2)).await.unwrap();

    assert_eq!(rpc_a.members().await.len(), 1);
    assert_eq!(rpc_b.members().await.len(), 1);

    let a: Arc<SenatorType> = Senator::new(Duration::from_millis(0), rpc_a.clone()); // peer 1
    let b: Arc<SenatorType> = Senator::new(Duration::from_millis(0), rpc_b.clone()); // peer 2

    a.start();
    b.start();

    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;

    assert_eq!(rpc_a.members().await.len(), 1);
    assert_eq!(rpc_b.members().await.len(), 1);

    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    rpc_a.remove_peer(2).await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    assert_eq!(rpc_a.members().await.len(), 0);
    assert_eq!(rpc_b.members().await.len(), 0);
}


#[tokio::test]
async fn duplicate_peers_automatically_resolve_when_established_by_is_inverted() {
    let ( a_to_b_stream1, b_to_a_stream1) = duplex(128);
    let ( a_to_b_stream2, b_to_a_stream2) = duplex(128);

    let rpc_a = RPCNetwork::new(1);
    let rpc_b = RPCNetwork::new(2);

    rpc_a.add_peer(Peer::new(2, 2, a_to_b_stream1)).await.unwrap();
    rpc_b.add_peer(Peer::new(2, 1, b_to_a_stream1)).await.unwrap();

    rpc_a.add_peer(Peer::new(1, 2, a_to_b_stream2)).await.unwrap();
    rpc_b.add_peer(Peer::new(1, 1, b_to_a_stream2)).await.unwrap();

    let a: Arc<SenatorType> = Senator::new(Duration::from_millis(0), rpc_a.clone()); // peer 1
    let b: Arc<SenatorType> = Senator::new(Duration::from_millis(0), rpc_b.clone()); // peer 2

    a.start();
    b.start();

    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    println!("{:?}", rpc_a.members().await);
    assert_eq!(rpc_a.members().await.len(), 1);
    assert_eq!(rpc_b.members().await.len(), 1);

    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    rpc_a.remove_peer(2).await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    assert_eq!(rpc_a.members().await.len(), 0);
    assert_eq!(rpc_b.members().await.len(), 0);
}

#[tokio::test]
async fn setting_a_timeout_delays_leadership() {
    let ( a_to_b_stream, b_to_a_stream) = duplex(128);

    let rpc_a = RPCNetwork::new(1);
    let rpc_b = RPCNetwork::new(2);

    rpc_a.add_peer(Peer::new(1, 2, a_to_b_stream)).await.unwrap();
    rpc_b.add_peer(Peer::new(1, 1, b_to_a_stream)).await.unwrap();

    let a: Arc<SenatorType> = Senator::new(Duration::from_millis(2000), rpc_a.clone()); // peer 1
    let b: Arc<SenatorType> = Senator::new(Duration::from_millis(2000), rpc_b.clone()); // peer 2

    a.start();
    b.start();

    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    assert_eq!(rpc_a.members().await.len(), 1);
    assert_eq!(rpc_b.members().await.len(), 1);

    // assert we are not a leader
    if let Role::Leader = *a.role.read().await {
        panic!("a should not be leader");
    };

    if let Role::Leader = *b.role.read().await {
        panic!("b should not be leader");
    };
}


/// set a_senator as leader, term 1
/// set b_senator as candidate, term 0
/// add eachother as peers
/// start both
/// wait for 2000 ms
/// assert a_senator is leader
/// assert b_senator is follower
#[tokio::test]
async fn leaders_cancel_candidates_with_lower_term() {
    let ( a_to_b_stream1, b_to_a_stream1) = duplex(128);

    let rpc_a = RPCNetwork::new(1);
    let rpc_b = RPCNetwork::new(2);

    rpc_a.add_peer(Peer::new(2, 2, a_to_b_stream1)).await.unwrap();
    rpc_b.add_peer(Peer::new(2, 1, b_to_a_stream1)).await.unwrap();

    let a: Arc<SenatorType> = Senator::new(Duration::from_millis(0), rpc_a.clone()); // peer 1
    let b: Arc<SenatorType> = Senator::new(Duration::from_millis(0), rpc_b.clone()); // peer 2

    *a.term.write().await = 1;
    *a.role.write().await = Role::Leader;
    *b.role.write().await = Role::Candidate;

    a.start();
    b.start();

    tokio::time::sleep(std::time::Duration::from_millis(2000)).await;

    assert_eq!(rpc_a.members().await.len(), 1);
    assert_eq!(rpc_b.members().await.len(), 1);

    // assert we are not a leader
    assert_eq!(*a.role.read().await, Role::Leader);
    assert_eq!(*b.role.read().await, Role::Follower);
}

/// senator_a is candidate, term 1
/// senator_b is candidate, term 0
///
/// they will both request votes from eachother
/// however, the candidate with the lower term will vote for
/// the other, and turn into a follower (in theory) because
/// the other candidate's term is higher
///
/// assert that senator_a is a leader, and senator_b is a follower
#[tokio::test]
async fn candidates_cancel_candidates_with_lower_term() {
    let ( a_to_b_stream1, b_to_a_stream1) = duplex(128);

    let rpc_a = RPCNetwork::new(1);
    let rpc_b = RPCNetwork::new(2);

    rpc_a.add_peer(Peer::new(2, 2, a_to_b_stream1)).await.unwrap();
    rpc_b.add_peer(Peer::new(2, 1, b_to_a_stream1)).await.unwrap();

    let a: Arc<SenatorType> = Senator::new(Duration::from_millis(0), rpc_a.clone()); // peer 1
    let b: Arc<SenatorType> = Senator::new(Duration::from_millis(0), rpc_b.clone()); // peer 2

    *a.term.write().await = 1;
    *a.role.write().await = Role::Candidate;
    *b.term.write().await = 0;
    *b.role.write().await = Role::Candidate;

    a.start();
    b.start();

    tokio::time::sleep(std::time::Duration::from_millis(2000)).await;

    assert_eq!(rpc_a.members().await.len(), 1);
    assert_eq!(rpc_b.members().await.len(), 1);

    // assert we are not a leader
    assert_eq!(*a.role.read().await, Role::Leader);
    assert_eq!(*b.role.read().await, Role::Follower);
}

/// senator_a is a leader, term 1
/// senator_b is a leader, term 0
///
/// they both send heartbeats to eachother
/// senator_a stays leader, senator_b becomes a follower
#[tokio::test]
async fn leaders_cancel_leaders_with_lower_term() {
    let ( a_to_b_stream1, b_to_a_stream1) = duplex(500);

    let rpc_a = RPCNetwork::new(1);
    let rpc_b = RPCNetwork::new(2);

    let b_peer = Peer::new(2, 2, a_to_b_stream1);
    let a_peer = Peer::new(2, 1, b_to_a_stream1);

    rpc_a.add_peer(b_peer.clone()).await.unwrap();
    rpc_b.add_peer(a_peer).await.unwrap();

    let a: Arc<SenatorType> = Senator::new(Duration::from_millis(0), rpc_a.clone()); // peer 1
    let b: Arc<SenatorType> = Senator::new(Duration::from_millis(0), rpc_b.clone()); // peer 2

    *a.term.write().await = 1;
    *a.role.write().await = Role::Leader;
    *b.term.write().await = 0;
    *b.role.write().await = Role::Leader;

    let (tx, mut rx) = mpsc::unbounded_channel();
    a.on_message(move |message| {
        tx.clone().send(message.from_role).unwrap();
    }).await;

    b_peer.send_msg::<MyMessage>(Message {
        term: 1,
        from: 1,
        to: 2,
        from_role: Role::Leader,
        msg: MessageType::LeaderHeartbeat
    }).await.unwrap();

    a.start();
    b.start();

    b.broadcast_message(MessageType::Custom(MyMessage::A)).await;

    let role = rx.recv().await.unwrap();

    assert_eq!(*b.role.read().await, Role::Follower);
    assert_eq!(role, Role::Follower);
    assert_eq!(*b.term.read().await, 1);

    tokio::time::sleep(std::time::Duration::from_millis(1600)).await;

    assert_eq!(rpc_a.members().await.len(), 1);
    assert_eq!(rpc_b.members().await.len(), 1);

    assert_eq!(*a.role.write().await, Role::Leader);
    assert_eq!(*b.role.write().await, Role::Follower);
}