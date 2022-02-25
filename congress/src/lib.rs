#[macro_use]
extern crate async_trait;
#[macro_use]
extern crate serde;

mod types;
mod rpc;

pub use types::{
    RPC,
    Role,
    Senator,
    Request,
    Response,
    NodeID,
};

pub use rpc::{
    RPCNetwork,
    Peer
};