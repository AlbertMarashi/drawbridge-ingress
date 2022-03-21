use events::EventHandler;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    sync::{mpsc, Mutex, RwLock},
    task::JoinHandle,
    time::{timeout, Instant},
};
use uuid::Uuid;

use crate::{
    states::{candidate::Candidate, follower::Follower, leader::Leader},
    types::{Peer, RequestType, ResponseType, Stream, UserReq, UserRes, VoteRequest, Message},
    utils::get_random_timeout,
    Error, NodeID, Request, Response, Role, Senator, RPC,
};

impl<Req: UserReq, Res: UserRes, R: RPC<Req, Res>> Senator<Req, Res, R> {
    pub fn start(self: &Arc<Self>) -> JoinHandle<()> {
        let senator = self.clone();

        tokio::spawn(async move {
            loop {
                // each loop is a new term
                let senator = senator.clone();
                let role = (*senator.role.lock().await).clone();

                match role {
                    Role::Leader => Leader::new(senator).run().await,
                    Role::Follower => Follower::new(senator).run().await,
                    Role::Candidate => Candidate::new(senator).run().await,
                }
            }
        })
    }

    pub async fn handle_vote_requst(self: &Arc<Self>, vote: VoteRequest) -> Response<Res> {
        // if the candidates term is less than ours, we will reject the vote
        if vote.term < *self.term.lock().await {
            return Response {
                term: *self.term.lock().await,
                peer_id: vote.peer_id,
                req_id: vote.req_id,
                msg: ResponseType::RequestVote {
                    vote_granted: false,
                },
            };
        }

        // if we observe a term greater than our own, we will become a follower
        if vote.term > *self.term.lock().await {
            *self.term.lock().await = vote.term;
            *self.role.lock().await = Role::Follower;
            *self.next_timeout.lock().await = Instant::now() + get_random_timeout();
        }

        let mut voted_for = self.voted_for.lock().await;

        match *voted_for {
            // if we have already voted for this peer, we will accept
            Some(peer_id) if vote.peer_id == peer_id => Response {
                term: *self.term.lock().await,
                peer_id: vote.peer_id,
                req_id: vote.req_id,
                msg: ResponseType::RequestVote { vote_granted: true },
            },
            // if we have already voted for someone else, we will reject
            Some(_) => Response {
                term: *self.term.lock().await,
                peer_id: vote.peer_id,
                req_id: vote.req_id,
                msg: ResponseType::RequestVote {
                    vote_granted: false,
                },
            },
            // if we have not voted for anyone, we will accept
            None => {
                *voted_for = Some(vote.peer_id);
                Response {
                    term: *self.term.lock().await,
                    peer_id: vote.peer_id,
                    req_id: vote.req_id,
                    msg: ResponseType::RequestVote { vote_granted: true },
                }
            }
        }
    }

    pub async fn broadcast_request(
        self: &Arc<Self>,
        req: RequestType<Req>,
    ) -> mpsc::UnboundedReceiver<Response<Res>> {
        let term = self.term.lock().await.clone();
        let clients = self.rpc.members().await;

        let (tx, rx) = mpsc::unbounded_channel();

        for peer_id in clients.into_iter() {
            let tx = tx.clone();
            let req = req.clone();
            let senator = self.clone();
            tokio::spawn(async move {
                let res = senator.rpc.send_request(Request {
                    peer_id,
                    req_id: Uuid::new_v4(),
                    term,
                    msg: req.clone(),
                }).await;

                match res {
                    Ok(res) => if let Err(e) = tx.send(res) {
                        println!("From Node {}: Receiver may have been dropped from timeout: {:?}", senator.id, e);
                    },
                    Err(err) => return eprintln!("A broadcast request failed: {:?}", err),
                }
            });
        }

        rx
    }

    pub fn new(id: NodeID, rpc: Arc<R>) -> Arc<Self> {
        Arc::new(Senator {
            id,
            rpc,
            role: Mutex::new(Role::Follower),
            term: Mutex::new(0),
            last_state: Mutex::new(None),
            voted_for: Mutex::new(None),
            next_timeout: Mutex::new(Instant::now() + get_random_timeout()),
            current_leader: Mutex::new(None),
            phantom_res: std::marker::PhantomData,
        })
    }
}

pub struct RPCNetwork<Req: UserReq, Res: UserRes, S: Stream> {
    pub peers: RwLock<HashMap<u64, Arc<Peer<S>>>>,
    pub phantom_req: std::marker::PhantomData<Req>,
    pub event_handler: EventHandler<(NodeID, Uuid), Response<Res>>,
    pub req_recv: Mutex<mpsc::UnboundedReceiver<Request<Req>>>,
    pub req_send: mpsc::UnboundedSender<Request<Req>>,
}

#[async_trait]
impl<Req: UserReq, Res: UserRes, S: Stream> RPC<Req, Res> for RPCNetwork<Req, Res, S> {
    async fn recv(&self) -> Option<Request<Req>> {
        self.req_recv.lock().await.recv().await
    }

    async fn send_request(&self, req: Request<Req>) -> Result<Response<Res>, Error> {
        let peer = self
            .peers
            .read()
            .await
            .get(&req.peer_id)
            .ok_or(Error::PeerNotFound)?
            .clone();

        // setup event handler
        let recv = self.event_handler.on((peer.id, req.req_id)).await;

        // send request
        peer.send_request::<Req, Res>(req).await?;

        // wait for response
        // with timeout
        let res = recv
            .await
            .map_err(|_| Error::ChannelError)?;

        Ok(res)
    }

    async fn send_response(&self, res: Response<Res>) -> Result<(), Error> {
        let peer = self.peers.read().await.get(&res.peer_id).unwrap().clone();
        peer.send_response::<Req, Res>(res).await
    }

    async fn members(&self) -> Vec<NodeID> {
        self.peers.read().await.keys().map(|key| *key).collect()
    }
}

impl<Req: UserReq, Res: UserRes, S: Stream> RPCNetwork<Req, Res, S> {
    pub fn new() -> Arc<Self> {
        let (req_send, req_recv) = mpsc::unbounded_channel();
        Arc::new(RPCNetwork {
            peers: RwLock::new(HashMap::new()),
            phantom_req: std::marker::PhantomData,
            event_handler: EventHandler::new(),
            req_recv: Mutex::new(req_recv),
            req_send,
        })
    }

    pub async fn add_peer(self: &Arc<Self>, client: Arc<Peer<S>>) -> JoinHandle<()> {
        self.peers.write().await.insert(client.id, client.clone());

        let client = client.clone();
        let rpc = self.clone();

        tokio::task::spawn(async move {
            loop {
                match client.read_msg().await {
                    Result::Ok(Message::Request(req)) => match rpc.req_send.send(req) {
                        Ok(..) => {}
                        Err(err) => {
                            eprintln!("Deleting peer, Failed to send request: {:?}", err);
                            break
                        }
                    },
                    Result::Ok(Message::Response(res)) => match rpc.event_handler.emit(&(res.peer_id, res.req_id), res).await {
                        Ok(..) => {},
                        Err(err) => {
                            eprintln!("Deleting peer, An event handler failed: {:?}", err);
                            break
                        }
                    },
                    Result::Err(e) => {
                        eprintln!("Deleting peer, couldn't read message from peer: {:?}", e);
                        break
                    },
                }
            }

            rpc.peers.write().await.remove(&client.id);
        })
    }

    pub async fn clear_outgoing(&self) {
        self.peers.write().await.clear();
    }
}
