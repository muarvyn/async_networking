use mio::{Events, Interest, Poll, Token, net};
use std::error::Error;
use std::io::{Write, Read, ErrorKind};

struct Connection {
    to_write: Vec<u8>,
    stream: net::TcpStream,
}

fn poll_1(select: &mut Poll, events: &mut Events, conns: &mut Vec<Connection>) 
    -> std::io::Result<()> {
    select.poll(events, None)?;
    assert!(!events.is_empty());
    for event in &*events {
        let Token(i) = event.token();
        match conns.get_mut(i) {
            Some(conn) => {
                if !conn.to_write.is_empty() && event.is_writable() {
                    match conn.stream.write(&conn.to_write[..]) {
                        Err(ref e) if e.kind() == ErrorKind::WouldBlock => break,
                        Ok(len @ 1..=std::usize::MAX) => {
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
                                println!("Received from connection #{}:\n{}", 
                                    i, String::from_utf8_lossy(&buffer));
                            }
                            Ok(_) => {
                                println!("Connection #{} is shut down.", i);
                                conns.remove(i);
                                break;
                            }
                            Err(ref e) => panic!("Error in connection: {:?}", e),
                        }
                    }
                }
            }
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

    let address: std::net::SocketAddr = "93.184.216.34:80".parse()?;
    let stream = net::TcpStream::connect(address)?;
    let mut conn1 = Connection { to_write: 
        br"GET / HTTP/1.1
Host: example.com
Connection: close

"
        .to_vec(), stream, };
    poll.registry().register(&mut conn1.stream, Token(0), 
        Interest::READABLE|Interest::WRITABLE)?;

    let stream = net::TcpStream::connect("127.0.0.1:80".parse().unwrap())?;
    let mut conn2 = Connection { to_write: 
        br"GET /index.htm HTTP/1.1
Host: example.com
Connection: close

"
        .to_vec(), stream, };
    poll.registry().register(&mut conn2.stream, Token(1), 
        Interest::READABLE|Interest::WRITABLE)?;

    let mut conns = Vec::from([conn1, conn2]);

    while conns.len() > 0 {
        //poll_1(&mut poll, &mut events, &mut conns)?;
        if let Err(e) = poll_1(&mut poll, &mut events, &mut conns) {
            if e.kind() != ErrorKind::Interrupted {
                panic!("Cannot poll selector: {}", e);
            }
        }
    }

    Ok(())
}
