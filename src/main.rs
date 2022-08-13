use mio::{Events, Interest, Poll, Token, net};
use std::error::Error;
use std::io::{self, Write, Read, ErrorKind};
use std::rc::{Rc, Weak};
use std::cell::RefCell;

struct Connection<T: h::Handler> {
    to_write: Vec<u8>,
    stream: net::TcpStream,
    handler: T,
}

mod h {
    use std::rc::Weak;
    use std::cell::RefCell;

    pub trait Handler {
        fn handle(&mut self, received: &[u8], len: usize);
    }

    pub enum HandlerState {}

    pub struct SomeHandler {
        pub conn: Weak<RefCell<crate::Connection<Self>>>,
    }

    impl Handler for SomeHandler {
        fn handle(&mut self, received: &[u8], len: usize) {
            println!("Received from connection: {}",
                     String::from_utf8_lossy(&received[..len]));
        }
    }
} // mod h

use h::Handler;
type Client1 = Connection<h::SomeHandler>;
type Connections = Vec<Option<Rc<RefCell<Client1>>>>;

fn poll_1(select: &mut Poll, events: &mut Events, conns: &mut Connections)
    -> std::io::Result<()> {
    select.poll(events, None)?;
    for event in &*events {
        let Token(i) = event.token();
        match conns.get_mut(i) {
            Some(Some(conn)) => {
                let mut conn = conn.borrow_mut();
                if !conn.to_write.is_empty() && event.is_writable() {
                    match conn.stream.write(&conn.to_write[..]) {
                        Err(ref e) if e.kind() == ErrorKind::WouldBlock => break,
                        Ok(len @ 1..=std::usize::MAX) => {
                            conn.stream.flush()?;
                            println!("Written {} bytes to connection #{}.", len, i);
                            conn.to_write.drain(..len);
                        }
                        Err(ref e) => panic!("Error in connection #{}:\n{:?}", i, e),
                        _ => {}
                    }
                }
                if event.is_readable() {
                    loop {
                        let mut buffer = [0u8; 2048];
                        match conn.stream.read(&mut buffer) {
                            Err(ref e) if e.kind() == ErrorKind::WouldBlock => break,
                            Ok(len) if len > 0 => {
                                //println!("Received from connection #{}:\n{}", 
                                //    i, String::from_utf8_lossy(&buffer));
                                conn.handler.handle(&buffer, len);
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
    let mut poll = Poll::new()?;
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

fn new_connection(poll: &mut Poll, addr: std::net::SocketAddr,
                  conns: &mut Connections, to_send: &[u8])
     -> io::Result<()> {
    let stream = net::TcpStream::connect(addr)?;
    let mut conn =
        Client1 { to_write: to_send.into(), stream,
                  handler: h::SomeHandler{conn:Weak::new()}};

    let token = Token(conns.len());
    poll.registry().register(&mut conn.stream, token,
        Interest::READABLE|Interest::WRITABLE)?;

    let conn = Rc::new(RefCell::new(conn));
    conn.borrow().handler.conn = Rc::downgrade(&conn);
    conns.push(Some(conn));
    Ok(())
}
