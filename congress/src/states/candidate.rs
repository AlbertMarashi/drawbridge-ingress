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

    pub async fn run(&mut self) {
        // we will sleep until the next timeout
        // each iteration of the outer loop represents a new term
        loop {
            *self.senator.voted_for.lock().await = None;

            match *self.senator.role.lock().await {
                Role::Candidate => {}
                _ => break,
            }

            // setup the new term
            self.votes_granted = 1;
            *self.senator.term.lock().await += 1;
            *self.senator.voted_for.lock().await = Some(self.senator.id.clone());
            *self.senator.current_leader.lock().await = None;

            println!(
                "Node {} became candidate for term {}",
                self.senator.id,
                *self.senator.term.lock().await
            );

            // we add 1 for ourselves, plus 1 for majority (and we round up)
            self.votes_needed = self.senator.rpc.members().await.len() as u64 / 2 + 1 as u64;

            // broadcast out a request vote request to all other nodes
            self.senator.broadcast_message(MessageType::VoteRequest).await;

            let timeout = Instant::now() + get_random_timeout();

            loop {
                let timeout_fut = tokio::time::sleep_until(timeout);
                // we will select a task which resolves first
                tokio::select! {
                    // election has timed out, break to outer loop;
                    _ = timeout_fut => break,
                    msg = self.senator.rpc.recv_msg() => match msg.msg {
                        // if a term is greater than ours, we will become a follower
                        // and set our term to theirs, and leader to them.
                        MessageType::LeaderHeartbeat => if msg.term >= *self.senator.term.lock().await {
                            *self.senator.term.lock().await = msg.term;
                            *self.senator.current_leader.lock().await = Some(msg.from);
                            *self.senator.role.lock().await = Role::Follower;
                            break
                        },
                        MessageType::VoteRequest => self.senator.handle_vote_request(msg.from, msg.term).await,
                        // ignore vote responses that are less or higher than our current term
                        MessageType::VoteResponse { vote_granted } => if msg.term == *self.senator.term.lock().await && vote_granted {
                            self.votes_granted += 1;

                            if self.votes_granted >= self.votes_needed {
                                // we have enough votes to become leader
                                *self.senator.role.lock().await = Role::Leader;
                                *self.senator.current_leader.lock().await = Some(self.senator.id);
                                break
                            }
                        }
                        MessageType::Custom(..) => self.senator.handle_user_message(msg).await,
                    },
                }
            }
        }
    }
}
