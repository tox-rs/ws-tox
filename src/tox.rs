use tokio::prelude::{Async, Stream};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender, UnboundedReceiver};
use serde::{Serialize, Deserialize};

use crate::protocol::*;

pub struct ToxHandle {
    pub request_tx: std::sync::mpsc::Sender<Request>,
    pub answer_rx: UnboundedReceiver<Answer>,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum Answer {
    Response(Response),
    Event(Event),
}

fn tox_loop(request_rx: std::sync::mpsc::Receiver<Request>, mut answer_tx: UnboundedSender<Answer>) {
    loop {
        if let Ok(req) = request_rx.try_recv() {
            dbg!(req);
        }

        std::thread::sleep_ms(100)
    }
}

pub fn spawn_tox() -> ToxHandle {
    use std::sync::mpsc;

    let (request_tx, request_rx) = mpsc::channel();
    let (answer_tx, answer_rx) = unbounded_channel();

    std::thread::spawn(move || tox_loop(request_rx, answer_tx));

    ToxHandle {
        request_tx, answer_rx
    }
}
