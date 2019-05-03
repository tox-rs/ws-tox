extern crate websocket;

use crate::tox::ToxHandle;
use crate::tox::spawn_tox;
use websocket::server::InvalidConnection;
use core::fmt::Debug;

use futures::{future, Future, Sink, Stream};
use tokio::runtime::TaskExecutor;
use tokio::reactor::Handle as ReactorHandle;

use ws_tox_protocol as protocol;

use std::io::{Error as IoError, ErrorKind as IoErrorKind};

//use crate::protocol::*;

//mod protocol;
mod tox;

fn spawn_future<F, I, E>(f: F, desc: &'static str, executor: &TaskExecutor)
where
    F: Future<Item = I, Error = E> + 'static + Send,
    E: Debug,
{
    executor.spawn(
        f.map_err(move |e| println!("{}: '{:?}'", desc, e))
            .map(move |_| println!("{}: Finished.", desc)),
    );
}

fn main() {
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc::{unbounded_channel};

    let mut runtime = tokio::runtime::Builder::new().build().unwrap();
    let executor = runtime.executor();

    let ToxHandle { request_tx, answer_rx } = spawn_tox();

    let server = websocket::r#async::Server::bind("127.0.0.1:2794", &ReactorHandle::default()).unwrap();
    let tox_tx = Arc::new(Mutex::new(request_tx));

    let connection_sink = Arc::new(Mutex::new(None));
    let connection_sink2 = connection_sink.clone();

    let p = answer_rx
        .map_err(|_| IoError::new(IoErrorKind::Other, "answer_rx dropped"))
        .for_each(move |r| {
            if let Some(ref mut sink) = *connection_sink2.lock().unwrap() {
                let sink : &mut tokio::sync::mpsc::UnboundedSender<websocket::OwnedMessage> = sink;
                let answer = serde_json::to_string(&r).unwrap();
                sink.try_send(websocket::OwnedMessage::Text(answer))
                    .map_err(|_|  IoError::new(IoErrorKind::Other, "connection_sink dropped"))?;
            }

            Ok(())
        });

    let f = server
        .incoming()
        .then(future::ok) // wrap good and bad events into future::ok
        .filter(|event| {
            match event {
                Ok(_) => true, // a good connection
                Err(InvalidConnection { ref error, .. }) => {
                    println!("Bad client: {}", error);
                    false // we want to save the stream if a client cannot make a valid handshake
                }
            }
        })
        .and_then(|event| event) // unwrap good connections
        .map_err(|_| IoError::new(IoErrorKind::Other, "invalid connection"))
        .for_each(move |(upgrade, addr)| {
            let connection_sink = connection_sink.clone();
            let connection_sink2 = connection_sink.clone();
            let tox_tx = tox_tx.clone();

            println!("Got a connection from: {}", addr);
            if connection_sink.lock().unwrap().is_some() {
                spawn_future(upgrade.reject(), "Reject the second connection", &executor);
                return Ok(());
            }

            // accept the request to be a ws connection if it does
            let f = upgrade
                .accept()
                .map_err(|e| IoError::new(IoErrorKind::Other,
                    format!("websocket accept err: {}", e)
                ))
                .and_then(move |(s, _h)| {
                    let (sink, stream) = s.split();
                    let tox_tx = (*tox_tx.lock().unwrap()).clone();

                    let (tx, rx) = unbounded_channel();

                    *connection_sink.lock().unwrap() = Some(tx);
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
                            tox_tx.send(req)
                                .map_err(|_| IoError::new(IoErrorKind::Other, "tox_tx dropped"))
                        });

                    let from_tox = rx
                        .map_err(|_| IoError::new(IoErrorKind::Other, "from_tox rx dropped"))
                        .forward(sink.sink_map_err(|e| IoError::new(IoErrorKind::Other,
                            format!("websocket write err: {}", e)
                        )))
                        .map(|_| ());

                    to_tox.select(from_tox)
                        .map(|_| ())
                        .map_err(|(e, _)| e)
                })
                .and_then(move |_| {
                    *connection_sink2.lock().unwrap() = None;

                    Ok(())
                });

            spawn_future(f, "Client Status", &executor);
            Ok(())
        });

    let k = f.select(p).map_err(|(e, _)| e);

    runtime.block_on(k).unwrap();
}
