use std::{collections::HashMap, fmt::Debug, hash::Hash};

use tokio::sync::{Mutex, oneshot::{self, Sender, Receiver}};

pub struct EventHandler<K, T>{
    map: Mutex<HashMap<K, Sender<T>>>,
}

impl<K, T> EventHandler<K, T> where
    T: Debug + Send + 'static,
    K: Hash + Eq {

    pub fn new() -> EventHandler<K, T> {
        EventHandler {
            map: Mutex::new(HashMap::new()),
        }
    }

    pub async fn emit(&self, event: &K, value: T) -> Result<(), T> {
        let mut map = self.map.lock().await;
        let handler = map.remove(&event);

        if let Some(handler) = handler {
            Ok(handler.send(value)?)
        } else {
            Err(value)
        }
    }

    pub async fn on(&self, event: K) -> Receiver<T> {
        let mut map = self.map.lock().await;
        let (tx, rx) = oneshot::channel();
        map.insert(event, tx);
        rx
    }
}
