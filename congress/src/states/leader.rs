use std::{sync::Arc, time::Duration};

use tokio::time::Instant;

use crate::{
    types::{MessageType, UserMsg},
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
            *self.senator.term.read().await
        );

        loop {
            if Role::Leader != *self.senator.role.read().await {
                return
            };
            let timeout = Instant::now() + Duration::from_millis(50);
            self.senator.broadcast_message(MessageType::LeaderHeartbeat).await;

            tokio::select! {
                _ = tokio::time::sleep_until(timeout) => continue,
                _ = async {
                    loop {
                        if Role::Leader != *self.senator.role.read().await {
                            return
                        };

                        let msg = self.senator.rpc.recv_msg().await;

                        match msg.msg {
                            // we will reply with a heartbeat
                            // if the term is greater than ours, we will become a follower
                            MessageType::LeaderHeartbeat => if msg.term > *self.senator.term.read().await {
                                *self.senator.term.write().await = msg.term;
                                *self.senator.role.write().await = Role::Follower;
                            },
                            MessageType::VoteRequest => self.senator.handle_vote_request(msg.from, msg.term).await,
                            MessageType::Custom(..) => self.senator.handle_user_message(msg).await,
                            MessageType::VoteGranted => {} // we've already won
                        }
                    }
                } => return
            };
        }
    }
}
