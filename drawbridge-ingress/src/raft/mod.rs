// mod memstore;

// use async_raft::{AppData, AppDataResponse};

// use crate::letsencrypt_system::CertificateStateMinimal;

// #[derive(Debug, Clone, Deserialize, Serialize)]
// pub struct ClientRequest {
//     /// The ID of the client which has sent the request.
//     pub client: String,
//     /// The serial number of this request.
//     pub serial: u64,

//     pub client_data: CertificateStateMinimal,
// }

// impl AppData for ClientRequest {}

// #[derive(Debug, Clone, Deserialize, Serialize)]
// pub struct ClientResponse {
//     pub client_data: CertificateStateMinimal,
// }
// impl AppDataResponse for ClientResponse {}

// mod network {
//     use std::{collections::HashMap, sync::Arc};

//     use async_raft::{
//         raft::{
//             AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest,
//             InstallSnapshotResponse, VoteRequest, VoteResponse,
//         },
//         AppData, NodeId, Raft, RaftNetwork,
//     };
//     use rand::Rng;
//     use tokio::{
//         io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
//         sync::Mutex,
//     };

//     use super::{memstore::MemStore, ClientRequest, ClientResponse};

//     type DrawbridgeRaft = Raft<ClientRequest, ClientResponse, DrawbridgeNetwork, MemStore>;

//     pub struct DrawbridgeNetwork {
//         pub clients: Mutex<HashMap<NodeId, Client>>,
//     }

//     pub struct Client {
//         pub id: NodeId,
//         pub request_serial: u64,
//         pub writer: Box<dyn AsyncWrite + Send + Sync + Unpin>,
//         pub reader: Box<dyn AsyncRead + Send + Sync + Unpin>,
//     }

//     #[async_trait]
//     impl<D: AppData> RaftNetwork<D> for DrawbridgeNetwork {
//         async fn append_entries(
//             &self,
//             target: NodeId,
//             rpc: AppendEntriesRequest<D>,
//         ) -> Result<AppendEntriesResponse, anyhow::Error> {
//             let mut clients = self.clients.lock().await;

//             let client = clients
//                 .get_mut(&target)
//                 .ok_or_else(|| anyhow::anyhow!("No client with id {}", target))?;

//             let json = serde_json::to_string(&rpc).unwrap();

//             client.writer.write_all(json.as_bytes()).await?;

//             let mut buf = Vec::new();
//             client.reader.read_to_end(&mut buf).await?;
//             let response: AppendEntriesResponse = serde_json::from_slice(&buf).unwrap();

//             Ok(response)
//         }

//         async fn install_snapshot(
//             &self,
//             target: NodeId,
//             rpc: InstallSnapshotRequest,
//         ) -> Result<InstallSnapshotResponse, anyhow::Error> {
//             unimplemented!()
//         }

//         async fn vote(
//             &self,
//             target: NodeId,
//             rpc: VoteRequest,
//         ) -> Result<VoteResponse, anyhow::Error> {
//             unimplemented!()
//         }
//     }

//     fn create_network() -> DrawbridgeRaft {
//         let network = Arc::new(DrawbridgeNetwork {
//             clients: Mutex::new(HashMap::new()),
//         });
//         let node_id: NodeId = rand::thread_rng().gen();
//         let config = async_raft::Config::build("drawbridge".into())
//             .validate()
//             .expect("Failed to build raft config");

//         let storage = MemStore::new(node_id);

//         let raft = Raft::new(node_id, Arc::new(config), network, Arc::new(storage));

//         raft
//     }
// }
