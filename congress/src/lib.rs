#[macro_use] extern crate async_trait;
#[macro_use] extern crate serde;
#[macro_use] extern crate derivative;

mod types;
mod peer;
mod senator;
mod states;
pub mod utils;

pub use types::{
    RPC,
    Role,
    Senator,
    Request,
    Response,
    NodeID,
    Peer,
    UserReq,
    UserRes,
    MessageType,
    Message
};

pub use senator::RPCNetwork;

#[derive(Debug)]
pub enum Error {
    IO(std::io::Error),
    Other(String),
    CouldNotSerialize,
    CouldNotDeserialize,
    InvalidMessageType,
    UnexpectedEOF,
    ResponseTimeout,
    ChannelError,
    PeerNotFound
}