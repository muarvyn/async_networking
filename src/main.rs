use mio::{Events, Interest, Token, net};
use std::error::Error;
use std::io::{self, Write, Read, ErrorKind, IoSlice};
use std::collections::*;

struct Connection<T: h::Handler> {
    to_write: VecDeque<u8>,
    stream: net::TcpStream,
    handler: T,
}

use std::task::Poll;
impl<T: h::Handler>  Connection<T> {
    fn write_if_pending(&mut self) -> io::Result<Poll<()>> {
        if self.to_write.is_empty() { return io::Result::Ok(Poll::Ready(())) }

        let (first, second) = self.to_write.as_slices();
        match self.stream.write_vectored(&[IoSlice::new(first), IoSlice::new(second)]) {
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => Ok(Poll::Pending),
            Ok(len @ 1..=std::usize::MAX) => {
                self.stream.flush()?;
                //println!("Written {} bytes to connection #{}.", len);
                self.to_write.drain(..len);
                if self.to_write.is_empty() {
                    io::Result::Ok(Poll::Ready(()))
                }
                else {
                    io::Result::Ok(Poll::Pending)
                }
            }
            Err(e) => io::Result::Err(e),
            _ => io::Result::Ok(Poll::Pending)
        }
    }
}

mod h {
    use std::io::Write;

    pub trait Handler {
        fn handle(&mut self, received: &[u8], len: usize, stream: impl Write);
    }

    pub enum HandlerState {}

    pub struct SomeHandler;
    impl Handler for SomeHandler {
        fn handle(&mut self, received: &[u8], len: usize, stream: impl Write) {
            println!("Received from connection: {}",
                     String::from_utf8_lossy(&received[..len]));
        }
    }
} // mod h

use h::Handler;
type Client1 = Connection<h::SomeHandler>;
type Connections = Vec<Option<Client1>>;

fn poll_1(select: &mut mio::Poll, events: &mut Events, conns: &mut Connections)
    -> std::io::Result<()> {
    select.poll(events, None)?;
    for event in &*events {
        let Token(i) = event.token();
        match conns.get_mut(i) {
            Some(Some(conn)) => {
                if event.is_writable() {
                    let _ = conn.write_if_pending()?;
                }
                if event.is_readable() {
                    loop {
                        let mut buffer = [0u8; 2048];
                        match conn.stream.read(&mut buffer) {
                            Err(ref e) if e.kind() == ErrorKind::WouldBlock => break,
                            Ok(len) if len > 0 => {
                                conn.handler.handle(&buffer, len, &conn.stream);
                            }
                            Ok(_) => {
                                println!("Connection #{} is shut down.", i);
                                conns[i].take();
                                break;
                            }
                            Err(ref e) => panic!("Error in connection: {:?}", e),
                        }
                    }
                }
            }
            Some(None) => { println!("WARNING! Connection #{} is gone.", i); }
            None => { panic!("Connection #{} not found.", i); }
        }
        println!("Handled event. writable={}, readable={}",
            event.is_writable(), event.is_readable());
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut poll = mio::Poll::new()?;
    let mut events = Events::with_capacity(128);

    let address: std::net::SocketAddr = "127.0.0.1:7878".parse()?;

    let mut conns = Vec::new();
    new_connection(&mut poll, address, &mut conns,
&br"echo GREEN connection kept alive and sleep 2 sec
keepalive
sleep 2
echo GREEN connection slept 2 sec and exits
exit".to_vec()
    )?;

    new_connection(&mut poll, address,
        &mut conns,
        &b"echo this is RED connection\nsleep 5\necho RED connection slept 5 sec"
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
        Client1 { to_write: VecDeque::new(), stream, handler: h::SomeHandler};
    conn.to_write.extend(to_send);

    let token = Token(conns.len());
    poll.registry().register(&mut conn.stream, token,
        Interest::READABLE|Interest::WRITABLE)?;

    conns.push(Some(conn));
    Ok(())
}
