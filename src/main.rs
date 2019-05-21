use crate::tox::ToxHandle;
use crate::tox::spawn_tox;

use websocket::server::InvalidConnection;
use websocket::server::r#async::Server;

use futures::{future, Future, Sink, Stream};
use tokio::reactor::Handle as ReactorHandle;

use ws_tox_protocol as protocol;

use std::fmt::Debug;
use std::io::{Error as IoError, ErrorKind as IoErrorKind};

mod tox;

fn spawn_future<F, I, E>(f: F, desc: &'static str)
where
    F: Future<Item = I, Error = E> + 'static + Send,
    E: Debug,
{
    tokio::spawn(
        f.map_err(move |e| eprintln!("{}: '{:?}'", desc, e))
            .map(move |_| eprintln!("{}: Finished.", desc)),
    );
}

fn main() {
    let server = Server::bind("127.0.0.1:2794", &ReactorHandle::default()).unwrap();

    let f = server
        .incoming()
        .then(future::ok) // wrap good and bad events into future::ok
        .filter(|event| {
            match event {
                Ok(_) => true, // a good connection
                Err(InvalidConnection { ref error, .. }) => {
                    eprintln!("Bad client: {}", error);
                    false // we want to save the stream if a client cannot make a valid handshake
                }
            }
        })
        .and_then(|event| event) // unwrap good connections
        .map_err(|_| IoError::new(IoErrorKind::Other, "invalid connection"))
        .for_each(move |(upgrade, addr)| {
            eprintln!("Got a connection from: {}", addr);

            let uri = upgrade.uri();
            let secret_key =
                if uri.get(0..4) == Some("/ws/") {
                    uri.get(4..).and_then(|sk| sk.parse().ok())
                }
                else { None };

            let ToxHandle { request_tx, answer_rx, guard } = spawn_tox(secret_key);

            let f = upgrade
                .accept()
                .map_err(|e| IoError::new(IoErrorKind::Other,
                    format!("websocket accept err: {}", e)
                ))
                .and_then(move |(s, _h)| {
                    let (sink, stream) = s.split();

                    let to_tox = stream
                        .take_while(|m| Ok(!m.is_close()))
                        .filter_map(|m| {
                            use websocket::OwnedMessage;

                            match m {
                                OwnedMessage::Text(t) => {
                                    serde_json::from_str(&t).ok()
                                },
                                _ => None,
                            }
                        })
                        .map_err(|e| IoError::new(IoErrorKind::Other,
                            format!("websocket read err: {}", e)
                        ))
                        .for_each(move |req: protocol::Request| {
                            request_tx.send(req)
                                .map_err(|_| IoError::new(IoErrorKind::Other, "tox_tx dropped"))
                        });

                    let from_tox = answer_rx
                        .map_err(|_| IoError::new(IoErrorKind::Other, "answer_rx dropped"))
                        .map(move |r| {
                            let answer = serde_json::to_string(&r).unwrap();
                            websocket::OwnedMessage::Text(answer)
                        })
                        .forward(sink.sink_map_err(|e| IoError::new(IoErrorKind::Other,
                            format!("websocket write err: {}", e)
                        )))
                        .map(|_| ());;

                    to_tox.select(from_tox)
                        .map(|_| ())
                        .map_err(|(e, _)| e)
                })
                .then(move |r| {
                    drop(guard);

                    r
                });

            spawn_future(f, "Client Status");
            Ok(())
        });

    let mut runtime = tokio::runtime::Builder::new().build().unwrap();
    runtime.block_on(f).unwrap();
}
