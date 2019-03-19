extern crate mio;
extern crate bus;
extern crate time;

use std::io::prelude::*;
use mio::{Events, Poll, Ready, PollOpt, Token};

use mio::net::TcpStream;
use std::net::{TcpListener, SocketAddr};

use std::thread;
use std::time::Duration;
use std::sync::{Arc,Mutex};
use bus::Bus;

const MAIN_THREAD_CYCLE: Duration = Duration::from_millis(10);

fn timestamp() -> f64 {
    let timespec = time::get_time();
    let mills: f64 = timespec.sec as f64 + (timespec.nsec as f64 / 1000.0 / 1000.0 / 1000.0);
    mills
}

fn main() {

    // Set up a poll handle and container for polling events
    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    // Use the 'bus' crate to broadcast on channels; we need to serialize any
    // access to this because broadcast() and add_rx() require ownership
    let mut bus: Arc<Mutex<Bus<usize>>> = Arc::new(Mutex::new(Bus::new(1)));
 
    // Set up a global buffer for messages from console
    let global_buf: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(vec![vec![]]));


/* Set up the console stream and register with Poll. The connect() call here 
 * is implemented in mio (non-blocking), so an immediate HUP should indicate 
 * if we failed to connect (and should proceed to just terminate the program).
 */
    // Connect to the console stream
    let addr: SocketAddr = "10.200.200.5:666".parse().unwrap();
    let mut console_stream = match TcpStream::connect(&addr) {
        Ok(console_stream) => console_stream,
        Err(y) => {
            panic!("{}", y);
        },
    };

    // Register the console stream with poll
    poll.register(&console_stream, Token(0), Ready::all(), PollOpt::edge());

     // Check if we connected, otherwise die.
    poll.poll(&mut events, None);
    for event in &events {
        if event.token() == Token(0) && event.readiness().is_hup() {
            println!("[console]\tCouldn't connect to the console!");
            console_stream.shutdown(mio::tcp::Shutdown::Both);
            std::process::exit(-1);
        }
    }


/* Spawn a thread for handling the console stream, which does the following:
 *
 *  - If a socket is readable:
 *      - Check if the console HUP'ed; die if this is the case
 *      - Read some amount of data from socket into a vector
 *      - Acquire lock, push vector onto the global buffer
 *      - Broadcast on channel, causing all consumer threads to go on-CPU
 *
 * We expect the console to send new data roughly every ~16ms. Everything else 
 * going on needs to fit nicely into this time-window, otherwise we might end
 * up introducing some extra delay during mirroring.
 */
    let console_bus = bus.clone();
    let console_buf = global_buf.clone();

    thread::spawn(move || {

        println!("[console]\tStarted console socket!");

        'poll_console_sock: loop {
            poll.poll(&mut events, None);
            for event in &events {

                // If the socket is readable
                if event.token() == Token(0) && event.readiness().is_readable() {

                    // If the client hung up, break out of this loop
                    if event.readiness().is_hup() {
                        console_stream.shutdown(mio::tcp::Shutdown::Both);
                        println!("[console]\tThe console hung up our connection.");
                        break 'poll_console_sock;
                    }

                    // Read the message into a vector
                    let mut message = vec![];
                    console_stream.read_to_end(&mut message);

                    // Push the vector onto the global buffer
                    console_buf.lock().unwrap().push(message);

                    // Emit a message to all running consumer threads
                    console_bus.lock().unwrap().broadcast(1);
                    println!("[console]\t{:?}", timestamp());
                } 
            }
        }
    });


/* Start a server, then spawn a consumer thread when we accept() some client.
 * A consumer thread behaves like so:
 *
 *  - Go off-CPU until we fetch a message on the channel
 *      - Acquire lock and take access to the buffer
 *      - While our local read cursor is behind the write cursor:
 *          - Fetch a message and send it to the client
 *          - Increment our local read cursor
 *
 * Note that it's _probably_ possible for a client to become very far behind,
 * and somehow take up tons of time in-between frames (which would cause other
 * clients to fall behind, etc). There's probably some better way of dealing
 * with that via interactions between the read cursor and channel messages.
 */
    let mut consumer_token = 0;
    let listener = TcpListener::bind("127.0.0.1:666").unwrap();
    for s in listener.incoming() {
        consumer_token += 1;

        let mut read_cur = 0;
        let mut rx = bus.lock().unwrap().add_rx();
        let mut stream = TcpStream::from_stream(s.unwrap()).unwrap();
        let consumer_buf = global_buf.clone();

        // Use TCP_NODELAY - probably required to *go fast*
        stream.set_nodelay(true);

        thread::spawn(move || {
            println!("[token-{}] Thread spawned for consumer", consumer_token);
            loop {

                // We wait off-CPU here until we get a channel message
                rx.recv();

                // Block until we acquire the lock and unwrap the buffer
                let buffer = consumer_buf.lock().unwrap();
                let write_cur = buffer.len();

                // Read and send() until we catch up to the write cursor
                while read_cur < write_cur {
                    stream.write(buffer.get(read_cur).unwrap());
                    println!("[token-{}]\t{:?}", consumer_token, timestamp());
                    read_cur += 1;
                }
            }
        });
    }

    // Let the main thread just wait around, for now
    loop { thread::sleep(MAIN_THREAD_CYCLE); }

}
