use std::{sync::Arc, time::Duration};

use tokio::time::Instant;

use crate::{
    types::{Message, MessageType, UserReq, UserRes},
    Request, Response, Role, Senator, RPC,
};

pub struct Leader<Req: UserReq, Res: UserRes, R: RPC<Req, Res>> {
    senator: Arc<Senator<Req, Res, R>>,
}

impl<Req: UserReq, Res: UserRes, R: RPC<Req, Res>> Leader<Req, Res, R> {
    pub fn new(senator: Arc<Senator<Req, Res, R>>) -> Self {
        Self { senator }
    }

    pub async fn run(self) {
        println!("Node {:?} became leader for term {}", self.senator.id, *self.senator.term.lock().await);

        loop {
            match *self.senator.role.lock().await {
                Role::Leader => {}
                _ => break,
            }
            let timeout = Instant::now() + Duration::from_millis(50);

            self.senator.broadcast_request(Request::Heartbeat).await;

            loop {
                let timeout_fut = tokio::time::sleep_until(timeout);
                tokio::select! {
                    // we will send another heartbeat after each timeout
                    _ = timeout_fut => break,
                    Message { term, from, msg, ..} = self.senator.rpc.recv_msg() => match msg {
                        MessageType::Request(Request::Heartbeat) => {
                            // we will reply with a heartbeat
                            // if the term is greater than ours, we will become a follower
                            // but first set the timeout to the next timeout
                            if term > *self.senator.term.lock().await {
                                *self.senator.term.lock().await = term;
                                *self.senator.role.lock().await = Role::Follower;
                            }

                            self.senator.rpc.send_msg(Message {
                                from: self.senator.id,
                                to: from,
                                term,
                                msg: MessageType::Response(Response::Heartbeat)
                            }).await;
                        },
                        MessageType::Request(Request::VoteRequest) => self.senator.handle_vote_request(from, term).await,
                        MessageType::Request(Request::Custom(req)) => self.senator.handle_user_request(req).await,
                        MessageType::Response(Response::Heartbeat) => {},
                        MessageType::Response(Response::Custom(..)) => {},
                        MessageType::Response(Response::Vote { .. }) => {},
                    }
                };
            }
        }
    }
}
