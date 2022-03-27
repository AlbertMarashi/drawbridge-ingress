use std::{sync::Arc, time::Duration};

use tokio::time::Instant;

use crate::{
    types::{Message, MessageType, UserMsg},
    Role, Senator, RPC,
};

pub struct Leader<Msg: UserMsg, R: RPC<Msg>> {
    senator: Arc<Senator<Msg, R>>,
}

impl<Msg: UserMsg, R: RPC<Msg>> Leader<Msg, R> {
    pub fn new(senator: Arc<Senator<Msg, R>>) -> Self {
        Self { senator }
    }

    pub async fn run(self) {
        println!(
            "Node {:?} became leader for term {}",
            self.senator.id,
            *self.senator.term.lock().await
        );

        loop {
            match *self.senator.role.lock().await {
                Role::Leader => {}
                _ => break,
            }
            let timeout = Instant::now() + Duration::from_millis(50);

            self.senator.broadcast_message(MessageType::LeaderHeartbeat).await;

            loop {
                let timeout_fut = tokio::time::sleep_until(timeout);
                tokio::select! {
                    // we will send another heartbeat after each timeout
                    _ = timeout_fut => break,
                    msg @ Message { term, from, ..} = self.senator.rpc.recv_msg() => match msg.msg {
                        // we will reply with a heartbeat
                        // if the term is greater than ours, we will become a follower
                        // but first set the timeout to the next timeout
                        MessageType::LeaderHeartbeat => if term > *self.senator.term.lock().await {
                            *self.senator.term.lock().await = term;
                            *self.senator.role.lock().await = Role::Follower;
                        },
                        MessageType::VoteRequest => self.senator.handle_vote_request(from, term).await,
                        MessageType::Custom(..) => self.senator.handle_user_message(msg).await,
                        MessageType::VoteResponse { .. } => {},
                    }
                };
            }
        }
    }
}
