use mio::{Events, Interest, Token, net};
use std::error::Error;
use std::io::{self, Write, Read, ErrorKind, IoSlice};
use std::collections::*;

pub struct Connection {
    to_write: VecDeque<u8>,
    stream: net::TcpStream,
    handler: Option<Box<dyn h::Handler>>,
}

use std::task::Poll;
impl Connection {
    pub fn write_if_pending(&mut self) -> io::Result<Poll<()>> {
        if self.to_write.is_empty() { return Ok(Poll::Ready(())) }

        let (first, second) = self.to_write.as_slices();
        match self.stream.write_vectored(&[IoSlice::new(first), IoSlice::new(second)]) {
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => Ok(Poll::Pending),
            Ok(len @ 1..=std::usize::MAX) => {
                self.stream.flush()?;
                //println!("Written {} bytes to connection #{}.", len);
                self.to_write.drain(..len);
                if self.to_write.is_empty() {
                    self.stream.flush()?;
                    Ok(Poll::Ready(()))
                }
                else {
                    Ok(Poll::Pending)
                }
            }
            Err(e) => io::Result::Err(e),
            _ => Ok(Poll::Pending)
        }
    }

    pub fn append_to_write(&mut self, to_write: &[u8]) {
        self.to_write.extend(to_write);
    }
}

mod h {
    pub trait Handler {
        fn handle(self: Box<Self>, received: &[u8], len: usize, stream: &mut crate::Connection)
            -> Box<dyn Handler>;
    }

    pub struct InitialHandler {}

    pub struct ForwardHandler {
        name: String,
        count: usize,
    }

    impl Handler for InitialHandler {
        fn handle(self: Box<Self>, received: &[u8], len: usize, stream: &mut crate::Connection)
            -> Box<dyn Handler> {
//            println!("Received from connection: {}",
//                     String::from_utf8_lossy(&received[..len]));
            let buffer = String::from_utf8_lossy(&received[..len]);
            let (command, args) = crate::split_command(&buffer);
            match command {
                "name" => {
                    let name = args.to_string();
                    println!("Connection {} is opened.", name);
                    Box::new(ForwardHandler { name, count: 1, })
                }
                "" => self,
                _ => {
                    eprintln!("Error: command 'name' expected first.");
                    //stream.append_to_write(b"echo Error: command 'name' expected first.");
                    self
                }
            }
        }
    }

    impl Handler for ForwardHandler {
        fn handle(self: Box<Self>, received: &[u8], len: usize, stream: &mut crate::Connection)
            -> Box<dyn Handler> {
            let buffer = String::from_utf8_lossy(&received[..len]);
            println!("Connection {}. Received message #{}:'{}'",
                     self.name.as_str(), self.count, buffer);
            Box::new(ForwardHandler { name: self.name, count: self.count+1, })
        }
    }
} // mod h

type Connections = Vec<Option<Connection>>;

fn poll_1(select: &mut mio::Poll, events: &mut Events, conns: &mut Connections)
    -> std::io::Result<()> {
    select.poll(events, None)?;
    for event in &*events {
        let Token(i) = event.token();
        let mut shutdown = false;
        match conns.get_mut(i) {
            Some(Some(ref mut conn)) => {
                if event.is_readable() {
                    loop {
                        let mut buffer = [0u8; 2048];
                        match conn.stream.read(&mut buffer) {
                            Err(ref e) if e.kind() == ErrorKind::WouldBlock => break,
                            Ok(len) if len > 0 => {
                                let handler = conn.handler.take().unwrap();
                                conn.handler = Some(
                                    handler.handle(&buffer, len, conn)
                                    );
                            }
                            Ok(_) => {
                                shutdown = true;
                                break;
                            }
                            Err(ref e) => panic!("Error in connection: {:?}", e),
                        }
                    }
                }
                if event.is_writable() {
                    let _ = conn.write_if_pending()?;
                }
                if shutdown {
                    println!("Connection #{} is shut down.", i);
                    drop(conns[i].take());
                }
            }
            Some(None) => { println!("WARNING! Connection #{} is gone.", i); }
            None => { panic!("Connection #{} not found.", i); }
        }
//        println!("Handled event. writable={}, readable={}",
//            event.is_writable(), event.is_readable());
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut poll = mio::Poll::new()?;
    let mut events = Events::with_capacity(128);

    let address: std::net::SocketAddr = "127.0.0.1:7878".parse()?;

    let mut conns = Vec::new();
    new_connection(&mut poll, address, &mut conns,
&br"echo name GREEN
echo will keepalive and sleep for 2 sec
keepalive
sleep 2
echo Slept 2 sec and will exit
exit".to_vec()
    )?;

    new_connection(&mut poll, address,
        &mut conns,
        &b"echo name RED\necho will sleep 5 sec\nsleep 5\necho Slept 5 sec"
        .to_vec())?;

    while conns.iter().find(|x|x.is_some()).is_some() {
        if let Err(e) = poll_1(&mut poll, &mut events, &mut conns) {
            if e.kind() != ErrorKind::Interrupted {
                panic!("Cannot poll selector: {}", e);
            }
        }
    }

    Ok(())
}

fn new_connection(poll: &mut mio::Poll, addr: std::net::SocketAddr,
                  conns: &mut Connections, to_send: &[u8])
     -> io::Result<()> {
    let stream = net::TcpStream::connect(addr)?;
    let mut conn =
        Connection { to_write: VecDeque::new(), stream,
                     handler: Some(Box::new(h::InitialHandler{}))
                   };
    conn.to_write.extend(to_send);

    let token = Token(conns.len());
    poll.registry().register(&mut conn.stream, token,
        Interest::READABLE|Interest::WRITABLE)?;

    conns.push(Some(conn));
    Ok(())
}

fn split_command(s: &str) -> (&str, &str) {
    let s = s.trim_matches(char::from(0)).trim();
    s.split_at(s.find(" ").unwrap_or(s.len()))
}
