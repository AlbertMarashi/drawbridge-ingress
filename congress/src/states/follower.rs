use std::sync::Arc;

use tokio::time::Instant;

use crate::{
    types::{Message, MessageType, UserMsg},
    utils::get_random_timeout,
    Role, Senator, RPC,
};

pub struct Follower<Msg: UserMsg, R: RPC<Msg>> {
    senator: Arc<Senator<Msg, R>>,
}

impl<Msg: UserMsg, R: RPC<Msg>> Follower<Msg, R> {
    pub fn new(senator: Arc<Senator<Msg, R>>) -> Self {
        Self { senator }
    }

    pub async fn run(self) {
        println!(
            "Node {} became follower for term {}",
            self.senator.id,
            *self.senator.term.lock().await
        );
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
                msg @ Message { term, from, ..} = self.senator.rpc.recv_msg() => match msg.msg {
                    // we will reply with a heartbeat response
                    // but first set the timeout to the next timeout
                    MessageType::LeaderHeartbeat => if term >= *self.senator.term.lock().await {
                        *self.senator.term.lock().await = term;
                        *self.senator.current_leader.lock().await = Some(from);
                        *self.senator.next_timeout.lock().await = Instant::now() + get_random_timeout();
                    },
                    MessageType::VoteRequest => self.senator.handle_vote_request(from, term).await,
                    MessageType::Custom(..) => self.senator.handle_user_message(msg).await,
                    _ => {}
                }
            }
        }
    }
}
