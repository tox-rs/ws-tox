extern crate websocket;
extern crate native_tls;
extern crate structopt;

use crate::tox::ToxHandle;
use crate::tox::spawn_tox;
use websocket::server::{InvalidConnection, OptionalTlsAcceptor, NoTlsAcceptor};
use websocket::server::r#async::{Incoming};
use websocket::r#async::{Server, TcpStream as wsTcpStream, Stream as wsStream};
use core::fmt::Debug;

use futures::{future, Future, Sink, Stream};
use tokio::reactor::Handle as ReactorHandle;
use tokio::sync::mpsc::{unbounded_channel};
use tokio_tls::TlsStream;

use ws_tox_protocol as protocol;

use std::io::{Read, Write, Error as IoError, ErrorKind as IoErrorKind};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use structopt::StructOpt;

#[derive(StructOpt, Debug)]
#[structopt(name = "ws-tox")]
struct Opt {
    #[structopt(long = "pfx_path", parse(from_os_str), raw(required_unless = r#""insecure""#))]
    pfx_path: Option<PathBuf>,
    #[structopt(long = "pfx_pass", raw(required_unless = r#""insecure""#))]
    pfx_pass: Option<String>,
    #[structopt(long = "insecure")]
    insecure: bool,
}

//use crate::protocol::*;

//mod protocol;
mod tox;

fn spawn_future<F, I, E>(f: F, desc: &'static str)
where
    F: Future<Item = I, Error = E> + 'static + Send,
    E: Debug,
{
    tokio::spawn(
        f.map_err(move |e| println!("{}: '{:?}'", desc, e))
            .map(move |_| println!("{}: Finished.", desc)),
    );
}

struct OptionalTlsServer<S: OptionalTlsAcceptor>
{
    pub server: Server<S>,
}

trait OptionalTlsServerTrait {
    type IncomingStream: Read + Write + wsStream + Send + 'static;
    fn incoming(self) -> Incoming<Self::IncomingStream>;
}

impl OptionalTlsServerTrait for OptionalTlsServer<native_tls::TlsAcceptor> {
    type IncomingStream = TlsStream<wsTcpStream>;
    fn incoming(self) -> Incoming<Self::IncomingStream> {
        self.server.incoming()
    }
}

impl OptionalTlsServerTrait for OptionalTlsServer<NoTlsAcceptor> {
    type IncomingStream = wsTcpStream;
    fn incoming(self) -> Incoming<Self::IncomingStream> {
        self.server.incoming()
    }
}

fn run_server(server: impl OptionalTlsServerTrait)
{
    let ToxHandle { request_tx, answer_rx } = spawn_tox();

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
                spawn_future(upgrade.reject(), "Reject the second connection");
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
                .then(move |r| {
                    *connection_sink2.lock().unwrap() = None;

                    r
                });

            spawn_future(f, "Client Status");
            Ok(())
        });

    let k = f.select(p).map_err(|(e, _)| e);

    let mut runtime = tokio::runtime::Builder::new().build().unwrap();
    runtime.block_on(k).unwrap();
}

fn main() {
    let opt = Opt::from_args();

    if opt.insecure {
        let server = Server::bind("127.0.0.1:2794", &ReactorHandle::default()).unwrap();
        run_server(OptionalTlsServer{ server });
    } else {
        let mut file = std::fs::File::open(opt.pfx_path.unwrap()).expect("Could not find pfx file");
        let mut pkcs12 = vec![];
        file.read_to_end(&mut pkcs12).expect("Could not read pfx file");
        let pkcs12 = native_tls::Identity::from_pkcs12(&pkcs12, &opt.pfx_pass.unwrap())
            .expect("Could not create pkcs12");

        let acceptor = native_tls::TlsAcceptor::builder(pkcs12).build()
            .expect("Could not create TlsAcceptor");

        let server = Server::bind_secure("127.0.0.1:2794", acceptor, &ReactorHandle::default()).unwrap();
        run_server(OptionalTlsServer{ server });
    }
}
