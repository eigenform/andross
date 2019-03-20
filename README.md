# andross
An [experimental] Project Slippi client/relay, written in Rust!

## TODO 

- Flush console data for a session to a `.slp` file on disk 
- Clean up buffers in-between sessions and reset consumer threads
- Devise some nice way to handle client/console disconnect
- Go faster, optimize timing, etc.
- Expose some interface for sending real-time, "higher-level" messages to clients?


## Implementation Notes
I haven't written a lot of Rust, so I feel compelled to have some _very pedantic_
rambling about how things work right now (mostly for my own reference, but I
guess it might be useful for you too, especially if you're also new to Rust)!
If anything in these notes seems incorrect, please correct me with a pull
request! Also, as of right now, this program is written for Linux, and any
compatibility with other platforms is unknown.

There are two main things that this program has to do _very, very well_.
Maintaining guarantees about _timely delivery of messages_ is a priority:

- We always need to ingest data from a console _as soon as it is available_
  on a socket. Assuming we're connected over Ethernet, we have reasonable
  guarantees that this will happen every frame. At ~60fps, this means new
  data should arrive every ~16ms.

- We always need to emit [raw] data to consumers _as soon as it has been
  ingested from a console._ If we cannot instantaneously deliver data for a
  particular frame, we _must_ strive to deliver data before we receive the
  next frame's data. Under ideal network conditions, this means that we have
  a ~16ms window where we can deliver data without necessarily introducing
  an extra frame of lag downstream during mirroring.

Right now, this program dedicates separate threads to handling the work
associated with remote clients: one thread for ingesting data from a console
(the console thread), and N unique threads for emitting data to clients
(consumer threads).

### I/O Behaviour
I/O needs to be serialized in the following manner:

1. Receive data from the console for `frame X`
2. Emit data for `frame X` to all clients
3. Receive data from the console for `frame X+1`
4. Emit data for `frame X+1` to all clients
5. ...

All I/O needs to be non-blocking. Particularly, for the console thread, we
need to be able to go off-CPU in-between messages. This allows us to put some
consumer threads on-CPU in the time between frames on console.

The console thread uses the `mio` crate to deal with receive messages without
blocking. On Linux, `mio::Poll` uses the `epoll()` API, which allows the kernel
to register file descriptors and watch them for "events" while putting the
current process to sleep (basically, when the kernel doesn't see a relevant
"event" on a file descriptor, it puts us off-CPU by setting us to
`TASK_INTERRUPTIBLE`; see [fs/eventpoll.c](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/tree/fs/eventpoll.c#n1900)).

Currently, consumer threads block when calling `send()`. I'm actually not sure
how non-blocking writes to sockets work right now, or how they'd be useful
in this situation.

### Safe Access to Shared Resources
**TODO**: _How is Arc/Mutex implemented in Rust? What are the low-level details?_

In this model, all threads need to share access to some memory where console
data is stored. On top of this, we also need some way to serialize the order
in which threads are put on-CPU. "Fortunately" for us, Rust's compiler will
literally refuse to spit out code if we haven't convinced it that everything
important has been serialized, and that accesses to shared memory are all
accounted for.

Currently, because messages (and the number of messages) received from console
might be variable-length, we use a `Vec<Vec<u8>>` to have some dynamic list
of messages on the heap. In order to have guarantees that accesses to the
buffer are race-free, we need to wrap this up with two of Rust's synchronization
primitives: an `Arc`, and a `Mutex`.

In Rust, `std::sync::Arc` seems to be the idiomatic way to keep track of all
references to shared memory. `Arc` is basically a pointer to some memory which,
when cloned, increases a reference count. This gives us ("us" meaning, the
compiler) a way to reason about ownership to the underlying data during
compile-time.

Additionally, in order to have _mutable_ shared references to some memory, we
also need to wrap the buffer up with `std::sync::Mutex`. In order to use the
buffer, a thread must first acquire the lock. Due to the nice properties of
Rust's memory model (i.e. "ownership," "lifetime," things being implicitly
immutable, etc), this lock should be automatically freed when the compiler
notices the reference moving out-of-scope. On the other hand, if there is a
situation where another thread is put on-CPU and attempts to acquire the lock,
it will necessarily block until the lock is freed up.


### Control over Thread Scheduling
**TODO**: _How are channels implemented in Rust?_

In order to reason about things, it's nice to have some way of "sending a message"
from one thread to another: particularly, we need the console thread to send
some message to all consumer threads, which causes them to move on-CPU.

The `bus` crate seems to offer an alternative to Rust's `std::sync::mpsc::channel`
which implements the notion of "broadcasting" some message to all receiving
ends of a channel. My current understanding of this situation is:

- Since we need the console thread to broadcast (which requires a mutable
  reference), _and_ we need the main thread to create and clone new "receiving
  ends" on the bus (which also requires a mutable reference), we need to create
  `Arc<Mutex<Bus>>` so that changes to the `Bus` are always serialized.

- Whenever the console thread receives data from a frame, we emit a message to
  all consumer threads

- Consumer threads start by calling `.recv()`, which causes them to go off-CPU
  until a message is consumed.

