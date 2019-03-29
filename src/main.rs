extern crate bus;
extern crate byteorder;
extern crate mio;
extern crate time;
extern crate clap;

#[macro_use]
extern crate lazy_static;

use std::process;
use std::time::Duration;

use mio::{Events, Poll, PollOpt, Ready, Token};
use std::io::prelude::*;
use std::io::ErrorKind;

use mio::net::TcpStream;
use std::net::{SocketAddr, TcpListener};

use bus::Bus;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

use clap::{Arg, App};

mod slippi;

const MAIN_THREAD_CYCLE: Duration = Duration::from_millis(10);

/// Helper function for timing.
fn timestamp() -> f64 {
    let timespec = time::get_time();
    let mills: f64 = timespec.sec as f64 + (timespec.nsec as f64 / 1000.0 / 1000.0 / 1000.0);
    mills
}

/// A global buffer for messages from console.
/// Mutable access to this buffer must be serialized with a Mutex.
lazy_static! {
    static ref GLOBAL_BUF: Mutex<Vec<Vec<u8>>> = Mutex::new(vec![vec![]]);
}

/// A global list of all running consumer threads.
/// Mutable access to this list must be serialized with a Mutex.
lazy_static! {
    static ref CONSUMER_LIST: Mutex<Vec<usize>> = Mutex::new(vec![]);
}

/// A channel from the list thread to N consumer threads.
/// Mutable access to this object must be serialized with a Mutex.
lazy_static! {
    static ref CONSUMER_BUS: Arc<Mutex<Bus<u8>>> = Arc::new(Mutex::new(Bus::new(32)));
}

/// Spawn the 'console thread,' for receiving data from a console.
/// 'SocketAddr' is the address::port of the remote host.
/// 'rx' is the mpsc::Receiver<usize> end of an mpsc channel.
///
/// If a connection to the remote host can't be established, this function
/// will terminate the program with return code -1.
fn spawn_console_thread(addr: SocketAddr, rx: mpsc::Receiver<usize>) {
    // Register objects for handling asynchronous I/O
    let poll = Poll::new().unwrap();
    let mut events = Events::with_capacity(1024);

    // Connect to the console stream
    let mut stream = TcpStream::connect(&addr).unwrap();

    // Register the console stream with poll
    poll.register(&stream, Token(0), Ready::all(), PollOpt::edge())
        .unwrap();

    // Check if we connected, otherwise terminate the program.
    poll.poll(&mut events, None).unwrap();
    for event in &events {
        if event.token() == Token(0) && event.readiness().is_hup() {
            println!("[console]\tCouldn't connect to the console!");
            stream.shutdown(mio::tcp::Shutdown::Both).unwrap();
            process::exit(-1);
        }
    }

    thread::spawn(move || {
        println!("[console]\tStarted console socket!");

        'console_loop: loop {
            poll.poll(&mut events, None).unwrap();
            for event in &events {
                // If the socket is readable
                if event.token() == Token(0) && event.readiness().is_readable() {
                    // If the client hung up, break out of this loop
                    if event.readiness().is_hup() {
                        stream.shutdown(mio::tcp::Shutdown::Both).unwrap();
                        println!("[console]\tThe console hung up our connection.");
                        break 'console_loop;
                    }

                    // Read the message into a vector
                    let mut message = vec![];
                    match stream.read_to_end(&mut message) {
                        Ok(_) => {}
                        Err(y) => {
                            if y.kind() != ErrorKind::WouldBlock {
                                panic!("[console]\tI/O error ({})", y);
                            }
                        }
                    };

                    // Parse Slippi commands within the message
                    let msg = slippi::parse_message(&message);

                    // Push a new message onto the global buffer
                    GLOBAL_BUF.lock().unwrap().push(message);

                    // Emit a channel message to all consumer threads
                    CONSUMER_BUS.lock().unwrap().broadcast(msg);

                    match msg {
                        slippi::GAME_END => {
                            // Acquire lock, make a copy of the thread list
                            let mut consumers = CONSUMER_LIST.lock().unwrap().to_vec();

                            // Block until all consumers have checked in
                            while consumers.len() != 0 {
                                let tid = rx.recv().unwrap();
                                consumers.retain(|x| x != &tid);
                            }

                            // Free up all messages from this session
                            GLOBAL_BUF.lock().unwrap().clear();
                            println!("[console]\tFlushed memory from session");
                        }
                        slippi::GAME_START => {
                            println!("[console]\tGame started");
                        }
                        _ => {}
                    };
                }
            }
        }
    });
}

/// Spawn a 'consumer thread' for sending data to a client.
/// 'tid' is a unique thread ID.
/// 'stream' is the client stream managed by this thread.
/// 'tx' is a clone of the mpsc::SyncSender<usize> end of an mpsc channel.
fn spawn_consumer_thread(tid: usize, mut stream: TcpStream, tx: mpsc::SyncSender<usize>) {
    thread::spawn(move || {
        // Put this thread on the list and register ourselves on the bus
        CONSUMER_LIST.lock().unwrap().push(tid);
        let mut rx = CONSUMER_BUS.lock().unwrap().add_rx();

        // Keep a thread-local read cursor (unique to the client)
        let mut read_cur = 0;

        let threadname = format!("consumer-{}", tid);
        println!("[{}]\tConsumer thread spawned", threadname);

        'consumer_loop: loop {

            // Wait off-CPU until we get a message
            let state = rx.recv().unwrap();

            // Block until we acquire the lock and unwrap the buffer
            let buffer = GLOBAL_BUF.lock().unwrap();
            let write_cur = buffer.len();

            // Send messages until we are synchronized with the write cursor
            while read_cur < write_cur {
                match stream.write(buffer.get(read_cur).unwrap()) {

                    // Increment the cursor when we successfully send()
                    Ok(_) => {
                        read_cur += 1;
                    },

                    Err(y) => {
                        println!("[{}]\tError kind: {:?}", threadname, y.kind());

                        // Remove this thread from the list and terminate
                        if y.kind() == ErrorKind::BrokenPipe {
                            println!("[{}]\tClient disconnected", threadname);
                            CONSUMER_LIST.lock().unwrap().retain(|x| x != &tid);
                            break 'consumer_loop;
                        }
                        //if (y.kind() == ErrorKind::WouldBlock) ...
                    }
                };
            }

            // If the next batch of messages contains a GAME_END, tell the
            // console thread when we've finished sending to the client.
            match state {
                slippi::GAME_END => {
                    tx.send(tid).unwrap();
                    read_cur = 0;
                    println!("[{}]\tReset local cursor", threadname);
                }
                _ => {}
            };
        }
        println!("[{}]\tConsumer thread exited", threadname);
    });
}

/// The main loop. Dispatches a thread for managing data from console, and
/// then, dispatches a consumer thread whenever a client connects.
fn main() {
    let matches = App::new("andross")
        .version("0.1.0")
        .about("A relay for mirroring with Project Slippi, written in Rust!\n
        The server binds to tcp/666 by default, so you may need adequate
        permissions unless you select a different port number.")
        .arg(Arg::with_name("host")
            .help("IP address of the ingress data stream.")
            .required(true))
        .arg(Arg::with_name("port")
             .help("TCP port of the ingress data stream.\n(default=666)")
             .short("P")
             .takes_value(true)
             .long("port"))
        .arg(Arg::with_name("server-ip")
             .help("Starts the server on the given IP address.\n(default=127.0.0.1)")
             .takes_value(true)
             .short("l")
             .long("server-ip"))
        .arg(Arg::with_name("server-port")
             .help("Binds the server on the given TCP port.\n(default=666)")
             .short("p")
             .takes_value(true)
             .long("server-port"))
        .arg(Arg::with_name("max-conn")
             .help("Sets the maximum number of concurrent connections.\n(default=Unlimited)")
             .short("M")
             .takes_value(true)
             .long("client-limit")
        ).get_matches();

    let c_host = matches.value_of("host").unwrap();
    let c_port = matches.value_of("port").unwrap_or("666");
    let c_addr: String = String::from(format!("{}:{}", c_host, c_port));

    let s_host = matches.value_of("server-ip").unwrap_or("127.0.0.1");
    let s_port = matches.value_of("server-port").unwrap_or("666");
    let s_addr: String = String::from(format!("{}:{}", s_host, s_port));

    // Create the console thread
    let addr: SocketAddr = c_addr.parse().unwrap();
    let (m_tx, m_rx) = mpsc::sync_channel(0);
    spawn_console_thread(addr, m_rx);

    // Create the relay server thread
    thread::spawn(move || {
        let server = TcpListener::bind(&s_addr).unwrap();
        let mut tid = 1;

        println!("[relay]\t\tListening on {}", &s_addr);

        // Wait in a loop until we accept() a new client
        for s in server.incoming() {
            let stream = TcpStream::from_stream(s.unwrap()).unwrap();
            stream.set_nodelay(true).unwrap();

            // Spawn a consumer client thread to handle this client
            println!("[relay]\t\tCreating new thread for client");
            spawn_consumer_thread(tid, stream, m_tx.clone());

            tid += 1;
        }
    });

    // Let the main thread just wait around, for now
    loop {
        thread::sleep(MAIN_THREAD_CYCLE);
    }
}
