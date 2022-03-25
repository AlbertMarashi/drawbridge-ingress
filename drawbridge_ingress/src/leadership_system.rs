//! # Leadership System
//! This system is responsible for managing the leadership election of the drawbridge ingress
//! cluster. It is needed to ensure that only one ingress controller is generating new certificates
//! at any given time.
//!
//! This is needed because letsencrypt challenges need to be replicate to each ingress instance
//! in order for certificate generation to work.
//!
//! ## How it works
//! 1. A task which regularly queries the kubernetes api for the list of services, ingresses, and listens for changes.
//! 2. on leadership change, the ingress controller will check for new certificates.
//! 3. on kubernetes api change, the ingress controller will check for new certificates IF it is the leader.
//! 4. on apply_state requests, we will tell the letsencrypt system to apply the state.
//!
//! kubernetes api changes will not be missed if nobody is a leader, because we will automatically check for new certificates
//! when we become the leader.
use std::{sync::Arc, hash::{Hash, Hasher}, collections::{hash_map::DefaultHasher}};

use congress::{Senator, RPCNetwork, NodeID, Peer};
use serde::{Deserialize, Serialize};
use tokio::{net::TcpStream, task::JoinHandle, time::Duration, io::{AsyncReadExt, AsyncWriteExt}};
use tokio_retry::{strategy::{jitter, ExponentialBackoff}, Retry};

use crate::{certificate_generation::{LetsEncrypt, CertificateStateMinimal}, kube_config_tracker::RoutingTable, error::IngressLoadBalancerError};

#[derive(Clone, Debug, Deserialize, Serialize)]
enum Sends {
    RequestState,
    ApplyState(CertificateStateMinimal)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
enum Receives {
    State(CertificateStateMinimal),
}

type IngressRPCNetwork = RPCNetwork<Sends, Receives, TcpStream>;
type IngressSenator = Senator<Sends, Receives, IngressRPCNetwork>;

pub struct LeadershipSystem {
    current_pod_name: String,
    rpc: Arc<IngressRPCNetwork>,
    senator: Arc<IngressSenator>,
    lets_encrypt: Arc<LetsEncrypt>,
    routing_table: Arc<RoutingTable>,
}

impl LeadershipSystem {
    pub fn new(current_pod_name: String, lets_encrypt: Arc<LetsEncrypt>, routing_table: Arc<RoutingTable> ) -> Self {
        let rpc = RPCNetwork::new(name_to_hash(&current_pod_name));
        let senator = IngressSenator::new(Duration::from_millis(2000), rpc.clone());

        LeadershipSystem {
            current_pod_name,
            rpc,
            senator,
            lets_encrypt,
            routing_table,
        }
    }

    /// ## Starts the leadership system
    ///
    /// It will firstly start listening for incoming requests on port 8000 where other
    /// peers can connect to.
    ///
    /// It should read the first 8 bytes of the connection in order to read the
    /// peer id so we know who established the connection.
    ///
    /// This peer id will then be used to instantiate a peer in the rpc network.
    ///
    /// Once the server is listening, we will start the senator, with an initial delay of
    /// 1 second.
    async fn start(self: &Arc<Self>) -> Result<JoinHandle<Result<(), IngressLoadBalancerError>>, IngressLoadBalancerError> {
        let server = tokio::net::TcpListener::bind("0.0.0.0:8000")
            .await
            .map_err(|e| IngressLoadBalancerError::Other(format!("{}", e).into()))?;

        let clone = self.clone();

        let task = tokio::task::spawn(async move {
            loop {
                let (mut stream, _) = server.accept()
                    .await
                    .map_err(|e| IngressLoadBalancerError::Other(format!("{}", e).into()))?;
                let clone = clone.clone();
                tokio::task::spawn(async move {
                    // read 8 bytes from the stream
                    let mut buf = [0u8; 8];
                    stream.read_exact(&mut buf)
                        .await
                        .map_err(|e| IngressLoadBalancerError::Other(format!("Could not read ID from stream {}", e).into()))?;

                    let peer_id = u64::from_be_bytes(buf);

                    clone.rpc.add_peer(Peer::new(peer_id, peer_id, stream))
                        .await
                        .map_err(|e| IngressLoadBalancerError::Other(format!("{:#?}", e).into()))?
                        .await
                        .map_err(|e| IngressLoadBalancerError::Other(format!("{:#?}", e).into()))?
                        .map_err(|e| IngressLoadBalancerError::Other(format!("{:#?}", e).into()))?;

                    Ok::<(), IngressLoadBalancerError>(())
                });
            }
        });

        self.senator.on_role(|role| {

        }).await;

        self.senator.on_message(|message| {

        }).await;

        Ok(task)
    }

    /// ## Adds a peer to the network
    ///
    /// It will connect to the pod via it's pod name/ip by setting up a tcp connection.
    /// The TCP connection will be used to create a peer, which will be added to the RPC
    ///
    /// When we connect, we will write 8 bytes to the stream, which is our hashed pod name.
    /// This will be used by the peer to identify who we are
    pub async fn add_peer(self: &Arc<Self>, peer_name: String) -> Result<JoinHandle<Result<(), IngressLoadBalancerError>>, IngressLoadBalancerError> {
        let retry_stategy = ExponentialBackoff::from_millis(100)
            .factor(3)
            .map(jitter)
            .take(8);

        let mut stream = Retry::spawn(retry_stategy, || {
            setup_stream(&peer_name)
        })
            .await
            .map_err(|e| IngressLoadBalancerError::Other(format!("{}", e).into()))?;

        let our_id_bytes: [u8; 8] = self.rpc.our_id.to_be_bytes();

        stream.write_all(&our_id_bytes)
            .await
            .map_err(|e| IngressLoadBalancerError::Other(format!("{}", e).into()))?;

        let task = self.rpc.add_peer(Peer::new(name_to_hash(&self.current_pod_name), name_to_hash(&peer_name), stream))
            .await
            .map_err(|e| IngressLoadBalancerError::Other(format!("{:#?}", e).into()))?;

        Ok(tokio::task::spawn(async move {
            task
                .await
                .map_err(|e| IngressLoadBalancerError::Other(format!("{:#?}", e).into()))?
                .map_err(|e| IngressLoadBalancerError::Other(format!("{:#?}", e).into()))
        }))
    }

    /// ## Removes a peer from the network
    pub async fn remove_peer(self: &Arc<Self>, peer_name: String) -> Result<(), IngressLoadBalancerError> {
        self.rpc.remove_peer(name_to_hash(&peer_name))
            .await
            .map_err(|e| IngressLoadBalancerError::Other(format!("{:#?}", e).into()))
    }
}

fn name_to_hash(name: &str) -> NodeID {
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    hasher.finish()
}

async fn setup_stream(addr: &str) -> Result<TcpStream, IngressLoadBalancerError> {
    TcpStream::connect(&addr)
        .await
        .map_err(|e| IngressLoadBalancerError::Other(format!("{}", e).into()))
}