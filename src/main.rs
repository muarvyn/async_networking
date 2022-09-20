use mio::{Events, Interest, Token, net};
use std::error::Error;
use std::io::{self, Write, Read, ErrorKind, IoSlice};
use std::collections::*;
use futures::future::poll_fn;
use futures::task::{waker_ref, ArcWake};

pub struct Session {
    to_write: VecDeque<u8>,
    stream: net::TcpStream,
}

use std::task::{Poll, Context};
use std::future::Future;

impl Session {
    pub fn write_if_pending(&mut self) -> io::Result<Poll<()>> {
        if self.to_write.is_empty() { return Ok(Poll::Ready(())) }

        let (first, second) = self.to_write.as_slices();
        match self.stream.write_vectored(&[IoSlice::new(first), IoSlice::new(second)]) {
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => Ok(Poll::Pending),
            Ok(len @ 1..=std::usize::MAX) => {
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

    pub fn read<'a>(&'a mut self, buffer: &'a mut [u8])
        -> impl Future<Output = io::Result<usize>> + 'a {
        //let buffer_cell = RefCell::new(buffer);
        //let mut written = 0 as usize;
        poll_fn(|_context| {
            //let mut arr = &mut *buffer_cell.borrow_mut();
            //let (_allocated, free) = arr.split_at_mut(written);
            match self.stream.read(buffer) {
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => Poll::Pending,
                Ok(len) => Poll::Ready(Ok(len)),
                Err(e) => Poll::Ready(Err(e)),
            }
        })
    }

}

use std::cell::RefCell;

thread_local! {
    static POLL: RefCell<mio::Poll> = RefCell::new(mio::Poll::new().unwrap());
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut events = Events::with_capacity(128);
    let address: std::net::SocketAddr = "127.0.0.1:7878".parse()?;
    let mut conns = Vec::new();

    let session_1 = new_session(address, Token(conns.len()),
&br"echo name GREEN
echo will sleep for 2 sec
keepalive
sleep 2
echo Slept 2 sec
"
    .to_vec()
    )?;

    let future_1 = Box::pin(interaction_1(session_1));
    conns.push(Some(future_1));

    let session_2 = new_session(address, Token(conns.len()),
        &b"echo name RED\necho will sleep 5 sec\nsleep 5\necho Slept 5 sec\nkeepalive\n"
        .to_vec())?;
    let future_2 = Box::pin(interaction_1(session_2));
    conns.push(Some(future_2));

    let mut waker_arc = Arc::new(DummyWaker);
    let waker = waker_ref(&mut waker_arc);

    let mut context = Context::from_waker(&*waker);

    'outer:
    while conns.iter().find(|x|x.is_some()).is_some() {
        if let Err(e) = POLL.with(|poll|
            poll.borrow_mut().poll(&mut events, None)) {
            if e.kind() != ErrorKind::Interrupted {
                panic!("Cannot poll selector: {}", e);
            }
        }

        for event in &events {
            let Token(i) = event.token();
            match conns.get_mut(i) {
                Some(Some(ref mut future) ) => {
                    match future.as_mut().poll(&mut context) {
                        Poll::Ready(Ok(())) => {
                            drop(conns[i].take());
                            println!("Session #{} is closed.", i);
                        }
                        Poll::Ready(Err(e)) => {
                            println!("Error in session #{}: {}", i, e.to_string());
                            break 'outer;
                        }
                        Poll::Pending => {}
                    }

                }
                Some(None) => { println!("WARNING! Connection #{} is gone.", i); }
                None => { panic!("Connection #{} not found.", i); }
            }
        }
    }

    Ok(())
}

async fn interaction_1(mut conn: Session) -> io::Result<()> {
    let mut buffer = [0u8; 2048];
    let mut name = "unnamed".to_string();
    let mut write_is_pending = true;

    'outer:
    loop {
        if write_is_pending {
            write_is_pending = conn.write_if_pending()? == Poll::Pending;
        }

        let len = conn.read(&mut buffer).await?;
        let buffer = String::from_utf8_lossy(&buffer[..len]);
        let mut it = buffer.lines();

        while let Some(commandline) = it.next() {
            if get_name(commandline, &mut name) { break 'outer };
        }
    }

    let mut count = 1 as usize;
    loop {
        if write_is_pending {
            write_is_pending = conn.write_if_pending()? == Poll::Pending;
        }

        let len = conn.read(&mut buffer).await?;
        if len == 0 { break }
        let buffer = String::from_utf8_lossy(&buffer[..len]);
        let mut it = buffer.lines();

        while let Some(commandline) = it.next() {
            println!("Connection {}. Received message #{}:'{}'", name.as_str(), count, commandline);
            if count == 2 {
                let reply = format!("echo Hello from {} peer!\nexit\n", name);
                conn.append_to_write(reply.as_bytes());
                write_is_pending = true;
            }
            count += 1;
        }
    }

    Ok(())
}

fn new_session(addr: std::net::SocketAddr, token: Token, to_send: &[u8])
     -> io::Result<Session> {
    let stream = net::TcpStream::connect(addr)?;
    let mut conn = Session { to_write: VecDeque::new(), stream };
    conn.to_write.extend(to_send);

    POLL.with(|poll| poll.borrow_mut().registry()
        .register(&mut conn.stream, token, Interest::READABLE|Interest::WRITABLE)
    )?;

    Ok(conn)
}

fn split_command(s: &str) -> (&str, &str) {
    let s = s.trim_matches(char::from(0)).trim();
    s.split_at(s.find(" ").unwrap_or(s.len()))
}

fn get_name(s: &str, name: &mut String) -> bool {
    let (command, args) = crate::split_command(s);
    match command {
        "name" => {
            name.replace_range(.., args);
            true
        }
        _ => false,
    }
}

#[derive(Clone)]
struct DummyWaker;

use std::sync::Arc;
impl ArcWake for DummyWaker {
    fn wake_by_ref(_arc_self: &Arc<Self>) {
        unimplemented!();
    }

}
