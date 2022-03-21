use std::sync::Arc;

use tokio::time::Instant;

use crate::{Senator, Request, Response, utils::get_random_timeout, Role, RPC, types::{UserReq, UserRes, RequestType, ResponseType, VoteRequest}};


pub struct Follower<Req: UserReq, Res: UserRes, R: RPC<Req, Res>> {
    senator: Arc<Senator<Req, Res, R>>
}

impl<Req: UserReq, Res: UserRes, R: RPC<Req, Res>> Follower<Req, Res, R> {

    pub fn new(senator: Arc<Senator<Req, Res, R>>) -> Self {
        Self { senator }
    }

    pub async fn run(self) {
        println!("Node {} became follower for term {}", self.senator.id, *self.senator.term.lock().await);
        *self.senator.next_timeout.lock().await = Instant::now() + get_random_timeout();
        loop {
            match *self.senator.role.lock().await {
                Role::Follower => {}
                _ => break
            }

            let timeout = { self.senator.next_timeout.lock().await.clone() };

            let timeout_fut = tokio::time::sleep_until(timeout);

            // we will select a task which resolves first
            tokio::select! {
                // we will sleep until the next timeout
                _ = timeout_fut => *self.senator.role.lock().await = Role::Candidate,
                Some(Request { peer_id, req_id, term, msg }) = self.senator.rpc.recv() => match msg {
                    RequestType::Heartbeat => {
                        // we will reply with a heartbeat response
                        // but first set the timeout to the next timeout
                        if term >= *self.senator.term.lock().await {
                            *self.senator.next_timeout.lock().await = Instant::now() + get_random_timeout();
                            *self.senator.term.lock().await = term;
                            *self.senator.current_leader.lock().await = Some(peer_id);
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

                        continue;
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
                }
            }
        }
    }
}