use tokio::sync::mpsc::{unbounded_channel, UnboundedSender, UnboundedReceiver};
use serde::{Serialize, Deserialize};

use std::convert::TryInto;

use crate::protocol::*;

const BOOTSTRAP_IP: &'static str = "185.25.116.107";
const BOOTSTRAP_PORT: u16 = 33445;
const BOOTSTRAP_KEY: &'static str =
    "DA4E4ED4B697F2E9B000EEFE3A34B554ACD3F45F5C96EAEA2516DD7FF9AF7B43";

const CLIENT_NAME: &'static str = "ws-client";

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

fn get_peer_info(
    tox: &mut rstox::core::Tox,
    conference: u32,
    peer: u32
) -> Result<PeerInfo, ConferencePeerQueryError> {
    let pk = tox.get_peer_public_key(conference, peer)
        .map_err(|e| e.try_into().unwrap())?;
    let name = tox.get_peer_name(conference, peer)
        .map_err(|e| e.try_into().unwrap())?;
    let info = PeerInfo {
        number: peer,
        public_key: format!("{}", pk),
        name
    };

    Ok(info)
}

fn get_peer_list(
    tox: &mut rstox::core::Tox,
    conference: u32
) -> Result<Vec<PeerInfo>, ConferencePeerQueryError> {
    let count = tox.conference_peer_count(conference)
        .map_err(|e| e.try_into().unwrap())?;

    let mut list = Vec::with_capacity(count as usize);
    for peer in 0..count {
        list.push(get_peer_info(tox, conference, peer)?);
    }

    Ok(list)
}

fn get_conference_info(
    tox: &mut rstox::core::Tox,
    conference: u32
) -> Option<ConferenceInfo> {
    use rstox::core::errors::ConferenceTitleError as TitleError;

    let kind = tox.get_conference_type(conference)?;
    let title = match tox.get_conference_title(conference) {
        Ok(title) => title,
        Err(TitleError::InvalidLength) => "".to_owned(),
        _ => return None
    };
    let peers = get_peer_list(tox, conference).ok()?;

    Some(ConferenceInfo {
        number: conference,
        kind: kind.into(),
        title,
        peers
    })
}

fn run_request(tox: &mut rstox::core::Tox, request: &Request) -> Option<Response> {
    use Request as R;
    use ws_tox_protocol::Friend;

    match request {
        R::Info => {
            let tox_id = format!("{}", tox.get_address());

            let name = tox.get_name();
            let status = tox.get_status().into();
            let status_message = tox.get_status_message();

            let friends: Vec<_> = tox.get_friend_list()
                .into_iter()
                .map(|n| {
                    let public_key =
                        format!("{}", tox.get_friend_public_key(n).unwrap());
                    let name = tox.get_friend_name(n).unwrap();
                    let status = tox.get_friend_status(n).unwrap().into();
                    let status_message = tox.get_friend_status_message(n).unwrap();
                    let last_online = tox.get_friend_last_online(n).unwrap();

                    Friend {
                        number: n,
                        public_key,
                        name,
                        status,
                        status_message,
                        last_online
                    }
                })
                .collect();

            let response = Response::Info {
                tox_id,
                name,
                status,
                status_message,
                friends
            };

            return Some(response)
        },
        R::AddFriend { tox_id, message } => {
            let address: rstox::core::Address = tox_id.parse().ok()?;

            let response = tox.add_friend(&address, &message)
                .map(|()| Response::Ok)
                .unwrap_or_else(|e| Response::AddFriendError {
                    error: e.try_into().expect("unexpected friend add error")
                });

            return Some(response)
        },
        R::AddFriendNorequest { tox_id } => {
            let address: rstox::core::PublicKey = tox_id.parse().ok()?;

            let response = tox.add_friend_norequest(&address)
                .map(|()| Response::Ok)
                .unwrap_or_else(|e| Response::AddFriendError {
                    error: e.try_into().expect("unexpected friend add error")
                });

            return Some(response)
        },
        R::DeleteFriend { friend } => {
            let response = tox.delete_friend(*friend)
                .map(|()| Response::Ok)
                .unwrap_or_else(|_| Response::FriendNotFoundError);

            return Some(response)
        },
        R::GetConnectionStatus => {
            let response = Response::ConnectionStatus {
                status: tox.get_connection_status().into()
            };

            return Some(response)
        },
        R::GetAddress => {
            let response = Response::Address {
                address: format!("{}", tox.get_address())
            };

            return Some(response)
        },
        R::GetNospam => {
            unimplemented!()
        },
        R::SetNospam { nospam } => {
            unimplemented!()
        },
        R::GetPublicKey => {
            let response = Response::PublicKey {
                public_key: format!("{}", tox.get_public_key())
            };

            return Some(response)
        },
        R::SetName { name } => {
            drop(tox.set_name(name))
        },
        R::GetName => {
            let response = Response::Name {
                name: tox.get_name()
            };

            return Some(response)
        },
        R::SetStatusMessage { message } => {
            drop(tox.set_status_message(message))
        },
        R::GetStatusMessage => {
            let response = Response::StatusMessage {
                status: tox.get_status_message()
            };

            return Some(response)
        },
        R::SetStatus { status } => {
            use rstox::core::UserStatus as S;

            // TODO: Add `From` implementation into ws-tox-protocol
            let status = match status {
                UserStatus::None => S::None,
                UserStatus::Away => S::Away,
                UserStatus::Busy => S::Busy
            };

            tox.set_status(status)
        },
        R::GetStatus => {
            let response = Response::Status {
                status: tox.get_status().into()
            };

            return Some(response)
        },
        R::FriendByPublicKey { public_key } => {
            let response = public_key.parse().ok()
                .and_then(|pk| tox.friend_by_public_key(pk))
                .map(|friend| Response::Friend {
                    friend
                })
                .unwrap_or_else(|| Response::FriendNotFoundError);

            return Some(response)
        },
        R::FriendExists { friend } => {
            let response = Response::FriendExists {
                exists: tox.friend_exists(*friend)
            };

            return Some(response)
        },
        R::GetFriendPublicKey { friend } => {
            let response = tox.get_friend_public_key(*friend)
                .map(|pk| Response::PublicKey {
                    public_key: format!("{}", pk)
                })
                .unwrap_or_else(|| Response::FriendNotFoundError);

            return Some(response)
        },
        R::GetFriendLastOnline { friend } => {
            let response = tox.get_friend_last_online(*friend)
                .map(|last_online| Response::LastOnline {
                    last_online
                })
                .unwrap_or_else(|| Response::FriendNotFoundError);

            return Some(response)
        },
        R::GetFriendName { friend } => {
            let response = tox.get_friend_name(*friend)
                .map(|name| Response::Name {
                    name
                })
                .unwrap_or_else(|| Response::FriendNotFoundError);

            return Some(response)
        },
        R::GetFriendStatusMessage { friend } => {
            let response = tox.get_friend_status_message(*friend)
                .map(|status| Response::StatusMessage {
                    status
                })
                .unwrap_or_else(|| Response::FriendNotFoundError);

            return Some(response)
        },
        R::GetFriendStatus { friend } => {
            let response = tox.get_friend_status(*friend)
                .map(|status| Response::Status {
                    status: status.into()
                })
                .unwrap_or_else(|| Response::FriendNotFoundError);

            return Some(response)
        },
        R::GetFriendConnectionStatus { friend } => {
            let response = tox.get_friend_connection_status(*friend)
                .map(|status| Response::ConnectionStatus {
                    status: status.into()
                })
                .unwrap_or_else(|| Response::FriendNotFoundError);

            return Some(response)
        },
        R::SendFriendMessage { friend, kind, message } => {
            let response = tox.send_friend_message(*friend, (*kind).into(), message)
                .map(|message_id| Response::MessageSent {
                    message_id
                })
                .unwrap_or_else(|e| Response::SendFriendMessageError {
                    error: e.try_into().expect("unexpected send friend message error")
                });

            return Some(response)
        },
        R::ControlFile { friend, file_number, control } => {
            let response = tox.control_file(*friend, *file_number, (*control).into())
                .map(|_| Response::Ok)
                .unwrap_or_else(|e| Response::FileControlError {
                    error: e.try_into().expect("unexpected file control error")
                });

            return Some(response)
        },
        R::SeekFile { friend, file_number, position } => {
            let response = tox.seek_file(*friend, *file_number, *position)
                .map(|_| Response::Ok)
                .unwrap_or_else(|e| Response::FileSeekError {
                    error: e.try_into().expect("unexpected file seek error")
                });

            return Some(response)
        },
        R::GetFileId { friend, file_number } => {
            unimplemented!()
        },
        R::SendFile { friend, kind, file_size, file_name } => {
            let response = tox.send_file(*friend, (*kind).into(), *file_size, file_name)
                .map(|file_number| Response::FileNumber { file_number })
                .unwrap_or_else(|e| Response::FileSendError {
                    error: e.try_into().expect("unexpected file send error")
                });

            return Some(response)
        },
        R::SendFileChunk { friend, file_number, position, data } => {
            let response = tox.send_file_chunk(*friend, *file_number, *position, data)
                .map(|_| Response::Ok)
                .unwrap_or_else(|e| Response::FileSendChunkError {
                    error: e.try_into().expect("unexprected file chunk send error")
                });

            return Some(response)
        },
        R::NewConference => {
            let response = tox.new_conference()
                .map(|conference| Response::Conference {
                    conference
                })
                .expect("unexpected new conference error");

            return Some(response)
        },
        R::DeleteConference { conference } => {
            let response = tox.delete_conference(*conference)
                .map(|_| Response::Ok)
                .unwrap_or_else(|| unimplemented!());

            return Some(response)
        }
        R::GetPeerList { conference } => {
            let response = get_peer_list(tox, *conference)
                .map(|peers| Response::ConferencePeerList { peers })
                .unwrap_or_else(|error| Response::ConferencePeerQueryError { error });

            return Some(response)
        },
        R::ConferencePeerCount { conference } => {
            let response = tox.conference_peer_count(*conference)
                .map(|count| Response::ConferencePeerCount {
                    count
                })
                .unwrap_or_else(|e| Response::ConferencePeerQueryError {
                    error: e.try_into().unwrap()
                });

            return Some(response)
        },
        R::GetPeerName { conference, peer } => {
            let response = tox.get_peer_name(*conference, *peer)
                .map(|name| Response::ConferencePeerName {
                    name
                })
                .unwrap_or_else(|e| Response::ConferencePeerQueryError {
                    error: e.try_into().unwrap()
                });

            return Some(response)
        },
        R::GetPeerPublicKey { conference, peer } => {
            let response = tox.get_peer_public_key(*conference, *peer)
                .map(|pk| Response::ConferencePeerPublicKey {
                    public_key: format!("{}", pk)
                })
                .unwrap_or_else(|e| Response::ConferencePeerQueryError {
                    error: e.try_into().unwrap()
                });

            return Some(response)
        },
        R::IsOwnPeerNumber { conference, peer_number } => {
            let response = tox.is_own_peer_number(*conference, *peer_number)
                .map(|is_own| Response::IsOwnPeerNumber {
                    is_own
                })
                .unwrap_or_else(|e| Response::ConferencePeerQueryError {
                    error: e.try_into().unwrap()
                });

            return Some(response)
        },
        R::InviteToConference { friend, conference } => {
            let response = tox.invite_to_conference(*friend, *conference)
                .map(|_| Response::Ok)
                .unwrap_or_else(|e| Response::ConferenceInviteError {
                    error: e.try_into().unwrap()
                });

            return Some(response)
        },
        R::JoinConference { friend, cookie } => {
            let cookie = rstox::core::Cookie::from_bytes(cookie);
            let response = tox.join_conference(*friend, &cookie)
                .map(|conference| Response::Conference {
                    conference
                })
                .unwrap_or_else(|e| Response::ConferenceJoinError {
                    error: e.try_into().unwrap()
                });

            return Some(response)
        },
        R::SendConferenceMessage { conference, kind, message } => {
            let response = tox.send_conference_message(*conference, (*kind).into(), message)
                .map(|_| Response::Ok)
                .unwrap_or_else(|e| Response::ConferenceSendError {
                    error: e.try_into().unwrap()
                });

            return Some(response)
        },
        R::GetConferenceTitle { conference } => {
            let response = tox.get_conference_title(*conference)
                .map(|title| Response::ConferenceTitle {
                    title
                })
                .unwrap_or_else(|e| Response::ConferenceTitleError {
                    error: e.try_into().unwrap()
                });

            return Some(response)
        },
        R::SetConferenceTitle { conference, title } => {
            let response = tox.set_conference_title(*conference, title)
                .map(|_| Response::Ok)
                .unwrap_or_else(|e| Response::ConferenceTitleError {
                    error: e.try_into().unwrap()
                });

            return Some(response)
        },
        R::GetConferenceList => {
            let chat_list = tox.get_chatlist();

            let mut conferences = Vec::with_capacity(chat_list.len());
            for c in chat_list {
                if let Some(info) = get_conference_info(tox, c) {
                    conferences.push(info)
                }
            }

            let response = Response::ConferenceList { conferences };

            return Some(response)
        },
        R::GetConferenceType { conference } => {
            let response = tox.get_conference_type(*conference)
                .map(|kind| Response::ConferenceType {
                    kind: kind.into()
                })
                .unwrap_or_else(|| unimplemented!());

            return Some(response)
        },
        _ => drop(dbg!(request)),
    }

    None
}

fn tox_loop(request_rx: std::sync::mpsc::Receiver<Request>, mut answer_tx: UnboundedSender<Answer>) {
    use rstox::core::{Tox, ToxOptions};

    let mut tox = Tox::new(ToxOptions::new(), None).unwrap();

    tox.set_name(CLIENT_NAME).unwrap();
    let bootstrap_key = BOOTSTRAP_KEY.parse().unwrap();
    tox.bootstrap(BOOTSTRAP_IP, BOOTSTRAP_PORT, bootstrap_key).unwrap();

    dbg!(format!("Server Tox ID: {}", tox.get_address()));

    loop {
        if let Ok(req) = request_rx.try_recv() {
            if let Some(resp) = run_request(&mut tox, &req) {
                drop(answer_tx.try_send(Answer::Response(resp)))
            }
        }

        for ev in tox.iter() {
            if let Some(e) = crate::protocol::Event::from_tox_event(&ev) {
                drop(answer_tx.try_send(Answer::Event(e)))
            }
            else {
                dbg!(ev);
            }
        }

        tox.wait();
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
