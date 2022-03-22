use std::sync::Arc;

use tokio::time::Instant;

use crate::{
    types::{UserReq, UserRes, MessageType, Message},
    utils::get_random_timeout,
    Request, Response, Role, Senator, RPC,
};

pub struct Follower<Req: UserReq, Res: UserRes, R: RPC<Req, Res>> {
    senator: Arc<Senator<Req, Res, R>>,
}

impl<Req: UserReq, Res: UserRes, R: RPC<Req, Res>> Follower<Req, Res, R> {
    pub fn new(senator: Arc<Senator<Req, Res, R>>) -> Self {
        Self { senator }
    }

    pub async fn run(self) {
        println!(
            "Node {} became follower for term {}",
            self.senator.id,
            *self.senator.term.lock().await
        );
        *self.senator.next_timeout.lock().await = Instant::now() + get_random_timeout();
        loop {
            match *self.senator.role.lock().await {
                Role::Follower => {}
                _ => break,
            }

            let timeout = { self.senator.next_timeout.lock().await.clone() };

            let timeout_fut = tokio::time::sleep_until(timeout);

            // we will select a task which resolves first
            tokio::select! {
                // we will sleep until the next timeout
                _ = timeout_fut => *self.senator.role.lock().await = Role::Candidate,
                Message { term, from, msg, ..} = self.senator.rpc.recv_msg() => match msg {
                    MessageType::Request(Request::Heartbeat) => {
                        // we will reply with a heartbeat response
                        // but first set the timeout to the next timeout
                        if term >= *self.senator.term.lock().await {
                            *self.senator.term.lock().await = term;
                            *self.senator.current_leader.lock().await = Some(from);
                            *self.senator.next_timeout.lock().await = Instant::now() + get_random_timeout();
                        }

                        self.senator.rpc.send_msg(Message {
                            from: self.senator.id,
                            to: from,
                            term,
                            msg: MessageType::Response(Response::Heartbeat),
                        }).await;
                    },
                    MessageType::Request(Request::VoteRequest) => self.senator.handle_vote_request(from, term).await,
                    MessageType::Request(Request::Custom(req)) => self.senator.handle_user_request(req).await,
                    _ => {}
                }
            }
        }
    }
}
