extern crate mio;
extern crate bus;
extern crate time;
extern crate byteorder;

#[macro_use]
extern crate lazy_static;

use std::collections::HashMap;

use std::io::Cursor;
use std::io::SeekFrom;
use byteorder::{ReadBytesExt, BigEndian, LittleEndian};

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

// For more complex synchronization between threads
const SLEEP: usize          = 0x00;
const UPDATE: usize         = 0xd0;
const FLUSH: usize          = 0xd1;

// Slippi commands
const EVENT_PAYLOADS: u8    = 0x35; 
const GAME_START: u8        = 0x36; 
const PRE_FRAME: u8         = 0x37; 
const POST_FRAME: u8        = 0x38; 
const GAME_END: u8          = 0x39; 

// Hashmap for Slippi commands. Needs lazy evaluation, and also needs to have
// a static lifetime so we can reference it wherever/whenever. I assume that,
// because we need to change this later during runtime, this also means we 
// need to use a Mutex to serialize writes to this :^( 
lazy_static! {
    static ref SLIP_CMD: Mutex<HashMap<u8, u16>> = Mutex::new({
        let mut m = HashMap::new();
        m.insert(GAME_START, 0);
        m.insert(PRE_FRAME, 0);
        m.insert(POST_FRAME, 0);
        m.insert(GAME_END, 0);
        m
    });
}

fn parse_message(msg: &Vec<u8>) {

    // Ignore HELO packets
    if (msg.len() == 5) && (msg == b"HELO\x00") {
        println!("got HELO");
        return; 
    }

    let mut cur = 0;
    let mut rdr = Cursor::new(msg);

    loop {

        let cmd = rdr.read_u8().unwrap();
        match cmd {
            EVENT_PAYLOADS  => {
                let size = rdr.read_u8().unwrap();
                let num = (size - 1) / 0x3;
                for i in 0..num {
                    let k = rdr.read_u8().unwrap();
                    let l = rdr.read_u16::<LittleEndian>().unwrap();
                    SLIP_CMD.lock().unwrap().insert(k, l);
                    println!("[console]\tFound command {:x}, len 0x{:x}", k, l);
                }
            },
            GAME_START      => {
                println!("[console]\tDetected GAME_START");
            },
            GAME_END        => {
                println!("[console]\tDetected GAME_END");
            },

            _               => {
            },
        };
    }
}

fn main() {

    // Set up a poll handle and container for polling events
    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    // Use the 'bus' crate to broadcast on channels; we need to serialize any
    // access to this because broadcast() and add_rx() require ownership
    let mut bus: Arc<Mutex<Bus<usize>>> = Arc::new(Mutex::new(Bus::new(UPDATE)));
 
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
 *
 * One interesting problem is: how do we deal managing the global buffer?
 * Ideally, we flush on GAME_END commands and reset the cursors for consumers.
 * How do we synchronize this? We technically need to wait for consumers to
 * finish sending GAME_END to their remote clients.
 */
    let console_bus = bus.clone();
    let console_buf = global_buf.clone();

    thread::spawn(move || {

        println!("[console]\tStarted console socket!");

        'console_loop: loop {
            poll.poll(&mut events, None);
            for event in &events {

                // If the socket is readable
                if event.token() == Token(0) && event.readiness().is_readable() {

                    // If the client hung up, break out of this loop
                    if event.readiness().is_hup() {
                        console_stream.shutdown(mio::tcp::Shutdown::Both);
                        println!("[console]\tThe console hung up our connection.");
                        break 'console_loop;
                    }

                    // Read the message into a vector
                    let mut message = vec![];
                    console_stream.read_to_end(&mut message);

                    // Parse Slippi commands within the message
                    //parse_message(&message);

                    // Push the vector onto the global buffer
                    console_buf.lock().unwrap().push(message);

                    // Emit a message to update all running consumer threads
                    console_bus.lock().unwrap().broadcast(UPDATE);
                    //println!("[console]\t{:?}", timestamp());
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
    let mut tid = 0;
    let listener = TcpListener::bind("127.0.0.1:666").unwrap();
    for s in listener.incoming() {
        tid += 1;

        let mut state = SLEEP;
        let mut read_cur = 0;
        let mut rx = bus.lock().unwrap().add_rx();
        let mut stream = TcpStream::from_stream(s.unwrap()).unwrap();
        let consumer_buf = global_buf.clone();

        // Use TCP_NODELAY - probably required to *go fast*
        stream.set_nodelay(true);

        thread::spawn(move || {
            println!("[token-{}] Thread spawned for consumer", tid);
            'consumer_loop: loop {

                // We wait off-CPU here until we get a channel message
                println!("{:?}", rx.recv());
                match rx.recv().unwrap() {
                    UPDATE      => {
                        println!("[token-{}]\tUPDATE", tid);
                        state = UPDATE;
                    },
                    _           => {
                        println!("[token-{}]\tUnknown sync message", tid);
                        break 'consumer_loop;
                    },
                };

                // Block until we acquire the lock and unwrap the buffer
                let buffer = consumer_buf.lock().unwrap();
                let write_cur = buffer.len();

                // Read and send() until we catch up to the write cursor
                while read_cur < write_cur {
                    match stream.write(buffer.get(read_cur).unwrap()) {
                        Ok(n) => {},
                        Err(y) => {
                            println!("[token-{}]\tDisconnected ({})", tid, y);
                            break 'consumer_loop;
                        },
                    };
                    //println!("[token-{}]\t{:?}", tid, timestamp());
                    read_cur += 1;
                }
            }
        });
    }

    // Let the main thread just wait around, for now
    loop { thread::sleep(MAIN_THREAD_CYCLE); }

}
