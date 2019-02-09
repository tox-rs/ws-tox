use serde::{Serialize, Deserialize};

pub enum Message {
    Request(Request),
    Response(Response),
    Event(Event),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum Request {
    SendMessage { friend: u32, message: String },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum Response {
    Ok { },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum Event {
}
