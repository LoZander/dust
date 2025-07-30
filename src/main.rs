use std::{
    collections::VecDeque,
    env,
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    str::FromStr,
    sync::mpsc::{self, TryRecvError},
    thread::spawn,
};

use uuid::Uuid;

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
    Broadcast(String),
    Disconnect,
}

#[derive(Debug, thiserror::Error)]
#[error("parse error")]
struct ParseCommandError;

impl FromStr for Command {
    type Err = ParseCommandError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (cmd, args) = s.trim().split_once(" ").ok_or(ParseCommandError)?;

        match cmd {
            "broadcast" => Ok(Command::Broadcast(args.to_string())),
            "connect" => {
                let addr: SocketAddr = args.parse().map_err(|_| ParseCommandError)?;
                Ok(Command::Connect(addr))
            }
            "disconnect" => Ok(Command::Disconnect),
            _ => Err(ParseCommandError),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Msg {
    text: String,
    uuid: Uuid,
}

const UUID_SIZE: usize = 16;
const SEP_SIZE: usize = 1;
const CAPACITY: usize = 128;

#[derive(Debug, thiserror::Error)]
#[error("failed to convert `String` to `Msg`")]
struct TryFromStringToMsgError;

impl TryFrom<String> for Msg {
    type Error = TryFromStringToMsgError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let within_capacity = value.len() + SEP_SIZE + UUID_SIZE <= CAPACITY;

        let uuid = Uuid::new_v4();

        assert!(!uuid.as_bytes()[0] != 0, "Uuid started with 0!");

        if within_capacity {
            Ok(Self {
                text: value,
                uuid: Uuid::new_v4(),
            })
        } else {
            Err(TryFromStringToMsgError)
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum TryFromArrayToMsgError {
    #[error("missing seperator")]
    MissingSep,
    #[error("uuid error: `{0}`")]
    CorruptUuid(#[from] uuid::Error),
}

impl TryFrom<[u8; CAPACITY]> for Msg {
    type Error = TryFromArrayToMsgError;

    fn try_from(value: [u8; CAPACITY]) -> Result<Self, Self::Error> {
        let sep = value
            .iter()
            .position(|c| c == &0)
            .ok_or(TryFromArrayToMsgError::MissingSep)?;

        let text_bytes = &value[..sep];
        let uuid_bytes = &value[(sep + SEP_SIZE)..(sep + SEP_SIZE + UUID_SIZE)];

        let text = String::from_utf8_lossy(text_bytes).to_string();
        let uuid = Uuid::from_slice(uuid_bytes).unwrap();

        Ok(Self { text, uuid })
    }
}

impl Msg {
    fn into_bytes(self) -> [u8; CAPACITY] {
        let mut bytes = [0; CAPACITY];
        let length = self.text.len();
        self.text
            .into_bytes()
            .into_iter()
            .enumerate()
            .for_each(|(i, b)| bytes[i] = b);
        self.uuid
            .into_bytes()
            .into_iter()
            .enumerate()
            .for_each(|(i, b)| bytes[i + length + SEP_SIZE] = b);

        bytes
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct Queue<T> {
    size: usize,
    elements: VecDeque<T>,
}

impl<T> Queue<T> {
    fn new(size: usize) -> Self {
        Self {
            size,
            elements: VecDeque::new(),
        }
    }

    fn push(&mut self, item: T) -> Option<T> {
        self.elements.push_back(item);

        if self.elements.len() > self.size {
            self.elements.pop_front()
        } else {
            None
        }
    }
}

impl<T: PartialEq> Queue<T> {
    fn contains(&self, item: &T) -> bool {
        self.elements.contains(item)
    }
}

fn run(ip: SocketAddr) -> io::Result<()> {
    println!("starting on {ip}");
    let mut streams = Vec::new();
    let mut seen: Queue<Msg> = Queue::new(16);

    let in_comms = listen(ip)?;
    let cmds = read_input()?;

    loop {
        match in_comms.try_recv() {
            Err(TryRecvError::Empty) => (),
            Err(TryRecvError::Disconnected) => todo!(),
            Ok(comm) => {
                println!("new peer {}", comm.peer_addr().unwrap());
                streams.push(comm);
            }
        };

        streams = match cmds.try_recv() {
            Err(TryRecvError::Empty) => streams,
            Err(TryRecvError::Disconnected) => todo!(),
            Ok(cmd) => match cmd {
                Command::Connect(addr) => {
                    connect(&mut streams, addr)?;
                    streams
                }
                Command::Broadcast(s) => {
                    let msg: Msg = s.try_into().unwrap();

                    seen.push(msg.clone());

                    broadcast(streams, msg)
                }
                Command::Disconnect => {
                    streams
                        .iter_mut()
                        .map(|stream| stream.shutdown(Shutdown::Both))
                        .collect::<io::Result<()>>()?;
                    streams
                }
            },
        };

        streams = receive_msgs(streams, &mut seen);
    }
}

fn receive_msgs(streams: Vec<TcpStream>, seen: &mut Queue<Msg>) -> Vec<TcpStream> {
    let (retained, propagees): (Vec<_>, Vec<_>) = streams
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
    let mut msg = [0; CAPACITY];
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

fn propagate(streams: Vec<TcpStream>, msg: Msg, origin: SocketAddr) -> Vec<TcpStream> {
    let (mut origins, rest): (Vec<_>, Vec<_>) = streams
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

fn connect(streams: &mut Vec<TcpStream>, addr: SocketAddr) -> io::Result<()> {
    let conn = TcpStream::connect(addr)?;
    conn.set_nonblocking(true)
        .expect("setting nonblocking failed");
    println!("connecting {conn:?}");
    streams.push(conn);
    Ok(())
}

fn broadcast(mut streams: Vec<TcpStream>, msg: Msg) -> Vec<TcpStream> {
    println!("broadcasting {msg:?}");
    streams.iter_mut().for_each(|stream| {
        println!("broadcasting {msg:?} to {stream:?}");
        let written = stream
            .write(&msg.clone().into_bytes())
            .expect("writing message failed");
        println!("written {written} bytes");
    });

    streams
}

fn main() {
    let args: Vec<String> = env::args().collect();
    dbg!(&args);
    let ip: SocketAddr = args.get(1).unwrap().parse().unwrap();
    let _ = run(ip);
}
