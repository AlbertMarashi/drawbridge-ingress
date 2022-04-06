use std::sync::Arc;

use tokio::time::Instant;

use crate::{
    types::{MessageType, UserMsg},
    utils::get_random_timeout, Role, Senator, RPC,
};

pub struct Candidate<Msg: UserMsg, R: RPC<Msg>> {
    senator: Arc<Senator<Msg, R>>,
    votes_granted: u64,
    votes_needed: u64, // half the total number of nodes, rounded up
}

impl<Msg: UserMsg, R: RPC<Msg>> Candidate<Msg, R> {
    pub fn new(senator: Arc<Senator<Msg, R>>) -> Self {
        Self {
            senator,
            votes_granted: 0,
            votes_needed: 0,
        }
    }

    /// This will run each time we are a candidate
    pub async fn run(&mut self) {
        // setup the new term
        self.votes_granted = 1;
        *self.senator.term.write().await += 1;
        *self.senator.voted_for.lock().await = Some(self.senator.id.clone());
        *self.senator.current_leader.lock().await = None;

        println!(
            "Node {} became candidate for term {}",
            self.senator.id,
            *self.senator.term.read().await
        );

        // broadcast out a request vote request to all other nodes
        self.senator.broadcast_message(MessageType::VoteRequest).await;

        let timeout = Instant::now() + get_random_timeout();

        tokio::select! {
            _ = tokio::time::sleep_until(timeout) => return,
            _ = async {
                loop {
                    if Role::Candidate != *self.senator.role.read().await {
                        return
                    };

                    self.votes_needed = 1 + (self.senator.rpc.members().await.len() as u64 + 1) / 2;

                    if self.votes_granted >= self.votes_needed {
                        // we have enough votes to become leader
                        *self.senator.role.write().await = Role::Leader;
                        *self.senator.current_leader.lock().await = Some(self.senator.id);
                        println!("Node {} got enough votes ({} needed out of {} peers) to become leader", self.senator.id, self.votes_granted, self.senator.rpc.members().await.len() + 1);
                        return
                    }

                    let msg = self.senator.rpc.recv_msg().await;

                    match msg.msg {
                        // if a term is greater than ours, we will become a follower
                        // and set our term to theirs, and leader to them.
                        MessageType::LeaderHeartbeat => if msg.term >= { *self.senator.term.read().await } {
                            *self.senator.term.write().await = msg.term;
                            *self.senator.current_leader.lock().await = Some(msg.from);
                            *self.senator.role.write().await = Role::Follower;
                            *self.senator.voted_for.lock().await = None;
                        },
                        MessageType::VoteRequest => self.senator.handle_vote_request(msg.from, msg.term).await,
                        // if peer's term is greater than ours, revert to follower state.
                        // ignore vote responses that are less or higher than our current term
                        MessageType::VoteGranted => if msg.term == *self.senator.term.read().await {
                            self.votes_granted += 1;
                        }
                        MessageType::Custom(..) => self.senator.handle_user_message(msg).await,
                    }
                }
            } => return
        };
    }
}