use std::{
    env,
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    str::FromStr,
    sync::mpsc::{self, TryRecvError},
    thread::spawn,
};

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

fn run(ip: SocketAddr) -> io::Result<()> {
    println!("starting on {ip}");
    let mut streams = Vec::new();

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
                Command::Broadcast(msg) => broadcast(streams, msg),
                Command::Disconnect => {
                    streams
                        .iter_mut()
                        .map(|stream| stream.shutdown(Shutdown::Both))
                        .collect::<io::Result<()>>()?;
                    streams
                }
            },
        };

        streams = receive_msgs(streams);
    }
}

fn receive_msgs(streams: Vec<TcpStream>) -> Vec<TcpStream> {
    let (retained, propagees): (Vec<_>, Vec<_>) = streams
        .into_iter()
        .filter_map(|mut stream| {
            let mut msg = [0; 128];
            let addr = stream.peer_addr().expect("connection didn't have a peer");

            match stream.read(&mut msg) {
                Ok(0) => {
                    println!("peer {addr} disconnected");
                    None
                }
                Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => Some((stream, None)),
                Err(err) => panic!("IO error: {err}"),
                Ok(_) => {
                    let s: &[u8] = msg
                        .iter()
                        .position(|c| c == &0)
                        .map(|i| &msg[..i])
                        .unwrap_or(&msg);
                    let string_msg: String = String::from_utf8_lossy(s).into();
                    println!("{addr}: {string_msg}");

                    Some((stream, Some((string_msg, addr))))
                }
            }
        })
        .unzip();

    propagees
        .into_iter()
        .filter_map(|x| x)
        .fold(retained, |acc, (msg, origin)| propagate(acc, msg, origin))
}

fn propagate(streams: Vec<TcpStream>, msg: String, origin: SocketAddr) -> Vec<TcpStream> {
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

fn broadcast(mut streams: Vec<TcpStream>, msg: String) -> Vec<TcpStream> {
    println!("broadcasting {msg}");
    streams.iter_mut().for_each(|stream| {
        println!("broadcasting {msg} to {stream:?}");
        let written = stream.write(msg.as_bytes()).expect("writing failed");
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
