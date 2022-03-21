use almost_raft::Message;

#[derive(Debug)]
struct Peer {

}

#[async_trait]
impl almost_raft::Node for Peer {
    type NodeType = Peer;
    async fn send_message(&self, msg: Message<Self::NodeType>) {
        unimplemented!()
    }

    fn node_id(&self) -> &String {
        unimplemented!()
    }
}

fn create_leader(nodes: Vec<Peer>) {

    use almost_raft::election::RaftElectionState;

    let (tx, mut from_raft) = tokio::sync::mpsc::channel(10);
    let self_id = uuid::Uuid::new_v4().to_string();
    let min_node = (nodes.len() / 2) as usize;
    let max_node = nodes.len() as usize;

    let (state, tx_to_raft) = RaftElectionState::init(
        self_id,
        500, // election timeout
        50, // heartbeat interval
        50, // message timeout
        nodes, // peers
        tx.clone(), // transmitter
        max_node,
        min_node, // min nodes
    );

    use almost_raft::election::raft_election;
    tokio::spawn(raft_election(state));
}