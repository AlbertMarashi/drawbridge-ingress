use std::{sync::Arc, time::Duration};

use tokio::time::Instant;

use crate::{Senator, Request, Response, utils::get_random_timeout, Role, RPC, types::{UserReq, UserRes, ResponseType, RequestType, VoteRequest}};


pub struct Leader<Req: UserReq, Res: UserRes, R: RPC<Req, Res>> {
    senator: Arc<Senator<Req, Res, R>>
}

impl<Req: UserReq, Res: UserRes, R: RPC<Req, Res>> Leader<Req, Res, R> {

    pub fn new(senator: Arc<Senator<Req, Res, R>>) -> Self {
        Self { senator }
    }

    pub async fn run(self) {
        println!("Node {:?} became leader", self.senator.id);
        { *self.senator.current_leader.lock().await = Some(self.senator.id); }
        loop {
            match *self.senator.role.lock().await {
                Role::Leader => {}
                _ => break
            }
            let timeout = Instant::now() + Duration::from_millis(50);
            let mut broadcast = self.senator.broadcast_request(RequestType::Heartbeat).await;

            loop {
                let timeout_fut = tokio::time::sleep_until(timeout);
                tokio::select! {
                    // we will send another heartbeat after each timeout
                    _ = timeout_fut => break,
                    Some(Request { term, peer_id, req_id, msg }) = self.senator.rpc.recv() => match msg {
                        RequestType::Heartbeat => {
                            // we will reply with a heartbeat
                            // if the term is greater than ours, we will become a follower
                            // but first set the timeout to the next timeout
                            if term > *self.senator.term.lock().await {
                                *self.senator.term.lock().await = term;
                                *self.senator.role.lock().await = Role::Follower;
                                *self.senator.next_timeout.lock().await = Instant::now() + get_random_timeout();
                            }

                            if let Err(e) = self.senator.rpc.send_response(Response {
                                term,
                                peer_id,
                                req_id,
                                msg: ResponseType::Heartbeat
                            }).await {
                                println!("{:?}", e);
                            }

                            break
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

                            break
                        },
                        RequestType::Custom(..) => {}
                    },
                    Some(Response { msg, ..}) = broadcast.recv() => match msg {
                        ResponseType::Heartbeat => {},
                        _ => panic!("Received unexpected response")
                    }
                }
            }
        }
    }
}