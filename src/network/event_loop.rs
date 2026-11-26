//! Starts what [`Network`](super::Network) asks for and forwards each event to whoever is
//! waiting for it.

use std::collections::HashMap;

use libp2p::{
    Swarm,
    futures::StreamExt,
    kad::{Event::OutboundQueryProgressed, QueryId},
    swarm::SwarmEvent,
};
use tokio::sync::mpsc;

use super::{Behaviour, BehaviourEvent, Command, Progress};

pub struct EventLoop {
    swarm: Swarm<Behaviour>,
    commands: mpsc::Receiver<Command>,
    pending_queries: HashMap<QueryId, mpsc::UnboundedSender<Progress>>,
}

impl EventLoop {
    pub fn new(swarm: Swarm<Behaviour>, commands: mpsc::Receiver<Command>) -> Self {
        Self {
            swarm,
            commands,
            pending_queries: HashMap::new(),
        }
    }

    /// Run until the [`Network`](super::Network) is dropped.
    pub async fn run(mut self) {
        loop {
            tokio::select! {
                command = self.commands.recv() => match command {
                    Some(command) => self.handle_command(command),
                    None => return,
                },
                event = self.swarm.select_next_some() => self.handle_swarm_event(event),
            }
        }
    }

    fn handle_command(&mut self, command: Command) {
        match command {
            Command::GetClosestPeers { key, progress } => {
                let query = self.swarm.behaviour_mut().kademlia.get_closest_peers(key);
                self.pending_queries.insert(query, progress);
            }
        }
    }

    /// Forward the event of a query to whoever started it, and forget the query after its last.
    fn handle_swarm_event(&mut self, event: SwarmEvent<BehaviourEvent>) {
        let Ok(BehaviourEvent::Kademlia(OutboundQueryProgressed { id, stats, step, .. })) =
            event.try_into_behaviour_event()
        else {
            return;
        };
        let Some(progress) = self.pending_queries.get(&id) else {
            return;
        };

        // Nobody needs the event when the caller has stopped waiting for the query.
        let _ = progress.send(Progress { stats });
        if step.last {
            self.pending_queries.remove(&id);
        }
    }
}
