use std::sync::Arc;

use tokio::{time::Instant};

use crate::{Senator, Request, Response, utils::get_random_timeout, Role, RPC, types::{UserReq, UserRes, RequestType, ResponseType, VoteRequest}};


pub struct Candidate<Req: UserReq, Res: UserRes, R: RPC<Req, Res>> {
    senator: Arc<Senator<Req, Res, R>>,
    votes_granted: u64,
    votes_needed: u64, // half the total number of nodes, rounded up
}

impl<Req: UserReq, Res: UserRes, R: RPC<Req, Res>> Candidate<Req, Res, R> {

    pub fn new(senator: Arc<Senator<Req, Res, R>>) -> Self {
        Self {
            senator,
            votes_granted: 0,
            votes_needed: 0,
        }
    }

    pub async fn run(&mut self) {
        // we will sleep until the next timeout
        // each iteration of the outer loop represents a new term
        loop {
            *self.senator.voted_for.lock().await = None;

            match *self.senator.role.lock().await {
                Role::Candidate => {}
                _ => break
            }

            // setup the new term
            self.votes_granted = 1;
            *self.senator.term.lock().await += 1;
            *self.senator.voted_for.lock().await = Some(self.senator.id.clone());
            *self.senator.current_leader.lock().await = None;

            println!("Node {} became candidate for term {}", self.senator.id, *self.senator.term.lock().await);

            // we add 1 for ourselves, plus 1 for majority (and we round up)
            self.votes_needed = self.senator.rpc.members().await.len() as u64 / 2 + 1 as u64;

            // broadcast out a request vote request to all other nodes
            let mut pending_votes = self.senator.broadcast_request(RequestType::RequestVote).await;
            let timeout = { self.senator.next_timeout.lock().await.clone() };

            loop {
                let timeout_fut = tokio::time::sleep_until(timeout);
                // we will select a task which resolves first
                tokio::select! {
                    _ = timeout_fut => {
                        *self.senator.next_timeout.lock().await = Instant::now() + get_random_timeout();
                        break
                    }, // election has timed out, break to outer loop;
                    Some(Request { term, peer_id, req_id, msg }) = self.senator.rpc.recv() => match msg {
                        RequestType::Heartbeat => {
                            // we will reply with a heartbeat
                            // but first set the timeout to the next timeout
                            if term > *self.senator.term.lock().await {
                                *self.senator.term.lock().await = term;
                                *self.senator.current_leader.lock().await = Some(peer_id);
                                *self.senator.role.lock().await = Role::Follower;
                            }
                            match self.senator.rpc.send_response(Response {
                                peer_id,
                                req_id,
                                term,
                                msg: ResponseType::Heartbeat
                            }).await {
                                Ok(_) => {},
                                Err(e) => {
                                    println!("{:?}", e);
                                }
                            }
                        }
                        RequestType::RequestVote => {
                            let res = self.senator.handle_vote_requst(VoteRequest {
                                peer_id,
                                term,
                                req_id,
                            }).await;

                            if let Err(e) = self.senator.rpc.send_response(res).await {
                                println!("{:?}", e);
                            }
                        },
                        RequestType::Custom(..) => {}
                    },
                    Some(Response { term, msg, ..}) = pending_votes.recv() => match msg {
                        ResponseType::RequestVote { vote_granted } => {
                            if term == *self.senator.term.lock().await {
                                if vote_granted {
                                    self.votes_granted += 1;

                                    if self.votes_granted >= self.votes_needed {
                                        // we have enough votes to become leader
                                        *self.senator.role.lock().await = Role::Leader;
                                        break;
                                    }
                                }
                            } else if term > *self.senator.term.lock().await {
                                // we have received a vote from a higher term
                                // we will become a follower
                                *self.senator.role.lock().await = Role::Follower;
                                break;
                            }
                        },
                        ResponseType::Heartbeat => {
                            // if the heartbeat term is greater than or equal to our current term,
                            // we will become a follower, otherwise we will ignore it
                            // and update the timeout
                            if term >= *self.senator.term.lock().await {
                                *self.senator.role.lock().await = Role::Follower;
                                *self.senator.next_timeout.lock().await = Instant::now() + get_random_timeout();
                                break
                            }
                        },
                        ResponseType::Custom(..) => {}
                    }
                }
            }
        }
    }
}