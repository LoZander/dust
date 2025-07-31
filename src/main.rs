use std::{
    env,
    io::{self, Read, Write},
    net::{self, Shutdown, SocketAddr, TcpListener, TcpStream},
    str::FromStr,
    sync::mpsc::{self, TryRecvError},
    thread::spawn,
};

use msg::Msg;
use queue::Queue;

mod msg;
mod queue;

/// Listens for incoming connections and returns a channel over which these are sent.
fn listen(ip: impl Into<SocketAddr>) -> io::Result<mpsc::Receiver<TcpStream>> {
    let listener = TcpListener::bind(ip.into())?;
    let (tx, rx) = mpsc::channel();

    spawn(move || -> io::Result<()> {
        loop {
            let (socket, _) = listener.accept()?;
            socket
                .set_nonblocking(true)
                .expect("setting nonblocking failed");
            println!("new connection {socket:?}");
            tx.send(socket).unwrap();
        }
    });

    Ok(rx)
}

#[derive(Debug, Clone)]
enum Command {
    Connect(SocketAddr),
    Broadcast(Msg),
    Disconnect,
}

#[derive(Debug, thiserror::Error)]
enum ParseCommandError {
    #[error("invalid command `{0}`")]
    InvalidCommand(String),
    #[error("missing seperator")]
    MissingSep,
    #[error(transparent)]
    TryFromStringToMsgError(#[from] msg::TryFromStringToMsgError),
    #[error(transparent)]
    AddrParseError(#[from] net::AddrParseError),
}

impl FromStr for Command {
    type Err = ParseCommandError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (cmd, args) = s
            .trim()
            .split_once(" ")
            .ok_or(ParseCommandError::MissingSep)?;

        match cmd {
            "broadcast" => Ok(Command::Broadcast(args.to_string().try_into()?)),
            "connect" => {
                let addr: SocketAddr = args.parse()?;
                Ok(Command::Connect(addr))
            }
            "disconnect" => Ok(Command::Disconnect),
            c => Err(ParseCommandError::InvalidCommand(c.to_string())),
        }
    }
}

/// Reads terminal input and returns a channel over which these inputs are sent.
fn read_input() -> io::Result<mpsc::Receiver<Command>> {
    let (tx, rx) = mpsc::channel();
    spawn(move || -> io::Result<()> {
        loop {
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;

            let command = input.parse().unwrap();

            tx.send(command).unwrap();
        }
    });

    Ok(rx)
}

/// Runs the p2p peer on the given socket.
fn run(ip: SocketAddr) -> io::Result<()> {
    println!("starting on {ip}");
    let mut peers = Vec::new();
    let mut seen: Queue<Msg> = Queue::new(16);

    let in_comms = listen(ip)?;
    let cmds = read_input()?;

    loop {
        match in_comms.try_recv() {
            Err(TryRecvError::Empty) => (),
            Err(TryRecvError::Disconnected) => todo!(),
            Ok(comm) => {
                println!("new peer {}", comm.peer_addr().unwrap());
                peers.push(comm);
            }
        };

        peers = match cmds.try_recv() {
            Err(TryRecvError::Empty) => peers,
            Err(TryRecvError::Disconnected) => todo!(),
            Ok(cmd) => match cmd {
                Command::Connect(addr) => {
                    connect(&mut peers, addr)?;
                    peers
                }
                Command::Broadcast(msg) => {
                    seen.push(msg.clone());
                    broadcast(peers, msg)
                }
                Command::Disconnect => {
                    peers
                        .iter_mut()
                        .map(|stream| stream.shutdown(Shutdown::Both))
                        .collect::<io::Result<()>>()?;
                    peers
                }
            },
        };

        peers = receive_msgs(peers, &mut seen);
    }
}

fn receive_msgs(peers: Vec<TcpStream>, seen: &mut Queue<Msg>) -> Vec<TcpStream> {
    let (retained, propagees): (Vec<_>, Vec<_>) = peers
        .into_iter()
        .filter_map(|stream| process_msg(stream, seen))
        .unzip();

    propagees
        .into_iter()
        .filter_map(|x| x)
        .fold(retained, |acc, (msg, origin)| propagate(acc, msg, origin))
}

fn process_msg(
    mut stream: TcpStream,
    seen: &mut Queue<Msg>,
) -> Option<(TcpStream, Option<(Msg, SocketAddr)>)> {
    let mut msg = [0; msg::CAPACITY];
    let addr = stream.peer_addr().expect("connection didn't have a peer");

    match stream.read(&mut msg) {
        Ok(0) => {
            println!("peer {addr} disconnected");
            None
        }
        Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => Some((stream, None)),
        Err(err) => panic!("IO error: {err}"),
        Ok(_) => {
            let m: Msg = msg.try_into().unwrap();

            if seen.contains(&m) {
                return Some((stream, None));
            }

            seen.push(m.clone());

            println!("{addr}: {}", m.text);

            Some((stream, Some((m, addr))))
        }
    }
}

/// Propagates a message `msg` received from a peer `origin` to the other peers.
fn propagate(peers: Vec<TcpStream>, msg: Msg, origin: SocketAddr) -> Vec<TcpStream> {
    let (mut origins, rest): (Vec<_>, Vec<_>) = peers
        .into_iter()
        .partition(|stream| stream.peer_addr().unwrap() == origin);
    let mut rest = broadcast(
        rest.into_iter()
            .filter(|stream| stream.peer_addr().unwrap() != origin)
            .collect(),
        msg,
    );

    rest.append(&mut origins);

    rest
}

/// Connects to a given peer.
fn connect(peers: &mut Vec<TcpStream>, addr: SocketAddr) -> io::Result<()> {
    let conn = TcpStream::connect(addr)?;
    conn.set_nonblocking(true)
        .expect("setting nonblocking failed");
    println!("connecting {conn:?}");
    peers.push(conn);
    Ok(())
}

/// Broadcasts a message to peers.
fn broadcast(mut peers: Vec<TcpStream>, msg: Msg) -> Vec<TcpStream> {
    println!("broadcasting {msg:?}");
    peers.iter_mut().for_each(|stream| {
        println!("broadcasting {msg:?} to {stream:?}");
        let written = stream
            .write(&msg.clone().into_bytes())
            .expect("writing message failed");
        println!("written {written} bytes");
    });

    peers
}

fn main() {
    let args: Vec<String> = env::args().collect();
    dbg!(&args);
    let ip: SocketAddr = args.get(1).unwrap().parse().unwrap();
    let _ = run(ip);
}
