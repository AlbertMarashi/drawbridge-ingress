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
use std::{sync::Arc, hash::{Hash, Hasher}, collections::{hash_map::DefaultHasher, HashMap}};

use congress::{Senator, RPCNetwork, NodeID, Peer, Role, MessageType};
use letsencrypt::{account::ServesChallenge, challenge::Http01Challenge};
use serde::{Deserialize, Serialize};
use tokio::{net::TcpStream, task::JoinHandle, time::Duration, io::{AsyncReadExt, AsyncWriteExt}, sync::Mutex};
use tokio_retry::{strategy::{jitter, FibonacciBackoff}, Retry};

use crate::{kube_config_tracker::RoutingTable, error::IngressLoadBalancerError, certificate_state::{CertificateState, CertKey}, lets_encrypt::CertGenerator};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum IngressMessage {
    RequestState,
    ApplyChallenge(Http01Challenge),
    ApplyCerts(HashMap<String, CertKey>),
}

type IngressRPCNetwork = RPCNetwork<IngressMessage, TcpStream>;
type IngressSenator = Senator<IngressMessage, IngressRPCNetwork>;

pub struct LeadershipSystem {
    pub certificate_state: Arc<CertificateState>,
    current_pod_name: String,
    pub rpc: Arc<IngressRPCNetwork>,
    pub senator: Arc<IngressSenator>,
    cert_generator: Mutex<Option<Arc<CertGenerator>>>,
    routing_table: Arc<RoutingTable>,
}

impl LeadershipSystem {
    pub fn new(current_pod_name: String, routing_table: Arc<RoutingTable> ) -> Arc<Self> {
        let rpc = RPCNetwork::new(name_to_hash(&current_pod_name));
        let senator = IngressSenator::new(Duration::from_secs(10), rpc.clone());

        Arc::new(LeadershipSystem {
            certificate_state: Arc::new(CertificateState::new()),
            current_pod_name,
            rpc,
            senator,
            cert_generator: Mutex::new(None),
            routing_table,
        })
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
    pub async fn start(self: &Arc<Self>) -> Result<JoinHandle<Result<(), IngressLoadBalancerError>>, IngressLoadBalancerError> {
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
                    let res: Result<(), IngressLoadBalancerError> = try {
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
                    };

                    if let Err(e) = res {
                        println!("Peer failed {}", e);
                    }
                });
            }
        });

        let clone = self.clone();

        self.senator.on_role(move|_| {
            let clone = clone.clone();
            tokio::task::spawn(async move {
                if let Err(e) = clone.handle_change().await {
                    dbg!(e);
                }
            });
        }).await;

        let clone = self.clone();

        self.senator.on_message(move |message| {
            let clone = clone.clone();
            println!("6");
            tokio::task::spawn(async move {
                dbg!(7, &message);
                match message.msg {
                    MessageType::Custom(custom) => match custom {
                        IngressMessage::ApplyChallenge(challenge) => clone.certificate_state.apply_challenge(challenge).await,
                        IngressMessage::ApplyCerts(certs) => *clone.certificate_state.certs.write().await = certs,
                        IngressMessage::RequestState if message.term <= *clone.senator.term.lock().await => clone.share_state().await,
                        _ => return
                    },
                    _ => return
                }
            });
        }).await;

        self.senator.start();

        Ok(task)
    }

    pub async fn share_state(&self) {
        match *self.senator.role.lock().await {
            Role::Leader => {
                self.senator.broadcast_message(MessageType::Custom(IngressMessage::ApplyCerts(self.certificate_state.certs.read().await.clone()))).await;
            }
            _ => return
        }
    }

    pub async fn handle_change(self: Arc<Self>) -> Result<(), IngressLoadBalancerError> {
        let generator = match *self.senator.role.lock().await {
            Role::Leader => {
                let mut cert_generator = self.cert_generator.lock().await;
                match &mut *cert_generator {
                    Some(generator) => generator.clone(),
                    opt @ None => {
                        let cert_generator = CertGenerator::create(self.routing_table.clone(), self.certificate_state.clone()).await;
                        *opt = Some(cert_generator.clone());
                        cert_generator
                    }
                }
            },
            // if we are a follower, request the state from the leader instead...
            Role::Follower if *self.senator.term.lock().await != 0 => {
                return Ok(self.senator.broadcast_message(MessageType::Custom(IngressMessage::RequestState)).await);
            },
            _ => return Ok(())
        };

        dbg!(9);
        generator.check_for_new_certificates(self.clone()).await?;
        dbg!(10);
        self.share_state().await;
        Ok(())
    }

    /// ## Adds a peer to the network
    ///
    /// It will connect to the pod via it's pod name/ip by setting up a tcp connection.
    /// The TCP connection will be used to create a peer, which will be added to the RPC
    ///
    /// When we connect, we will write 8 bytes to the stream, which is our hashed pod name.
    /// This will be used by the peer to identify who we are
    pub async fn add_peer(self: Arc<Self>, addr: String, peer_name: String) -> Result<JoinHandle<Result<(), IngressLoadBalancerError>>, IngressLoadBalancerError> {
        let retry_stategy = FibonacciBackoff::from_millis(100)
            .factor(1)
            .map(jitter)
            .take(10);

        let conn_addr = format!("{}:{}", addr, 8000);
        let mut stream = Retry::spawn(retry_stategy, || {
            setup_stream(&conn_addr)
        })
            .await
            .map_err(|e| IngressLoadBalancerError::Other(format!("Could not establish stream: {}", e).into()))?;

        let our_id_bytes: [u8; 8] = self.rpc.our_id.to_be_bytes();

        stream.write_all(&our_id_bytes)
            .await
            .map_err(|e| IngressLoadBalancerError::Other(format!("{}", e).into()))?;

        dbg!(&self.current_pod_name, &peer_name, name_to_hash(&self.current_pod_name), name_to_hash(&peer_name));

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
    pub async fn remove_peer(self: Arc<Self>, peer_name: String) -> Result<(), IngressLoadBalancerError> {
        self.rpc.remove_peer(name_to_hash(&peer_name))
            .await
            .map_err(|e| IngressLoadBalancerError::Other(format!("{:#?}", e).into()))
    }
}

#[async_trait::async_trait]
impl ServesChallenge for LeadershipSystem {
    async fn serve_challenge(self: &Arc<Self>, challenge: Http01Challenge) {
        println!("serving challenge for {}", challenge.domain);
        self.certificate_state.apply_challenge(challenge.clone()).await;
        self.senator.broadcast_message(MessageType::Custom(IngressMessage::ApplyChallenge(challenge))).await;
        // wait a second for the other nodes to apply the challenge
        tokio::time::sleep(Duration::from_secs(1)).await;
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