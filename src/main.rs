extern crate mio;
extern crate spmc;

use std::io::prelude::*;
use mio::{Events, Poll, Ready, PollOpt, Token};

use mio::net::TcpStream;
use std::net::{TcpListener, SocketAddr};

use std::thread;
use std::time::Duration;
use std::sync::{Arc,Mutex};

const MAIN_THREAD_CYCLE: Duration = Duration::from_millis(10);

fn main() {

/* Set up some state before actually starting to do any I/O:
 *  - Various mio objects
 *  - Thread synchronization things
 *  - A buffer for threads to share
 */

    // Set up a poll handle and container for events
    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    // Set up a single-producer, multiple-consumer channel
    let (tx, rx) = spmc::channel();
 
    // Set up a global buffer for messages from console
    let global_buf: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(vec![vec![]]));


/* Set up the console stream and register with Poll. The connect() call is
 * non-blocking with mio, so an immediate HUP indicates that we couldn't 
 * connect to the server (and should just terminate the program).
 */

    // Connect to the console stream
    //let addr: SocketAddr = "127.0.0.1:8000".parse().unwrap();
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

/* This thread handles the console stream by doing the following:
 *
 *  - If a socket is readable:
 *      - Check if the console HUP'ed; die if this is the case
 *      - Create a Vec, read data from the socket into it
 *      - Acquire lock, push Vec onto the global buffer
 *      - Emit a message on our spmc channel to tell consumer threads
 *
 *  - If a socket isn't readable:
 *      - Call thread::yield_now() and go off-CPU (maybe this is slow?)
 *
 */

    let console_buf = global_buf.clone();
    thread::spawn(move || {
        println!("[console]\tStarted console socket!");

        // Main loop - wait around for messages on the socket
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
                    //println!("[console]\tSocket ready to read");

                    // Read a message off the stream and append it to the buffer
                    let mut message = vec![];
                    console_stream.read_to_end(&mut message);

                    // Push a message onto the buffer; emit message on spmc channel
                    console_buf.lock().unwrap().push(message);
                    tx.send(true);
                } 

                // If the socket isn't readable, let's go off-CPU for now
                else {
                    thread::yield_now();
                }
            }
        }
    });



/* This thread handles consumer streams by doing the following:
 *
 *  - Go off-CPU until we fetch a message on the spmc channel
 *      - Acquire lock, fetch the latest write cursor
 *      - While our local read cursor is behind the write cursor:
 *          - Fetch a message and send it to the client
 *          - Increment our local read cursor
 *      - Go off-CPU again (maybe un-necessary?)
 *
 */


    let mut consumer_token = 0;
    let listener = TcpListener::bind("127.0.0.1:666").unwrap();
    for s in listener.incoming() {

        // Local read cursor
        let mut read_cur = 0;

        // Token for this thread's socket
        consumer_token += 1;

        // This thread's reference to the global buffer
        let consumer_buf = global_buf.clone();

        // This thread's copy of the receiving end of the spmc channel
        let rx = rx.clone();

        // Stream associated with this thread
        let mut stream = TcpStream::from_stream(s.unwrap()).unwrap();
        stream.set_nodelay(true);

        // Poll/event data associated with this thread
        let consumer_poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(1024);

        //consumer_poll.register(&stream, Token(consumer_token), Ready::all(), 
        //                       PollOpt::edge()).unwrap();

        thread::spawn(move || {
            println!("[token-{}] Thread spawned for consumer", consumer_token);
            loop {

                // Wait for an spmc channel message; needed to serialize things.
                // I think that this puts us off-CPU (which is good!)
                rx.recv();

                // Block until we acquire the lock and unwrap the buffer
                let buffer = consumer_buf.lock().unwrap();
                let write_cur = buffer.len();

                // Do reads until we catch up to the write cursor
                while read_cur < write_cur {

                    stream.write(buffer.get(read_cur).unwrap());

                    //println!("Got message number {}: {:?}", read_cur, 
                    //    buffer.get(read_cur).unwrap().iter());

                    // Increment our local read cursor and send data to client
                    read_cur += 1;
                }

                // Go off-CPU
                thread::yield_now();
            }
        });
    }

    // Let the main thread just wait around, or something
    loop { thread::sleep(MAIN_THREAD_CYCLE); }

}
