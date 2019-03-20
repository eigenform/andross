use byteorder::{BigEndian, ReadBytesExt};
use std::io::prelude::*;
use std::io::Cursor;
use std::io::SeekFrom;

use std::collections::HashMap;
use std::sync::Mutex;

/// A global hash map, mapping Slippi commands to their sizes.
/// Mutable access to the map must be serialized with a Mutex.
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

// Slippi commands
pub const EVENT_PAYLOADS: u8 = 0x35;
pub const GAME_START: u8 = 0x36;
pub const PRE_FRAME: u8 = 0x37;
pub const POST_FRAME: u8 = 0x38;
pub const GAME_END: u8 = 0x39;

/// Parses a packet with Slippi data. The packet may contain multiple Slippi
/// messages/commands.
///
/// Returns 0 if nothing interesting happens.
/// Returns GAME_END if the packet contains a GAME_END message.
pub fn parse_message(msg: &Vec<u8>) -> u8 {
    let mut res = 0;
    let len = msg.len() as u64;

    let mut rdr = Cursor::new(msg);
    println!("[console] Unwrapping message (len=0x{:x})", len);

    while rdr.position() < len {
        let cmd = rdr.read_u8().unwrap();
        match cmd {
            EVENT_PAYLOADS => {
                let size = rdr.read_u8().unwrap();
                let num = (size - 1) / 0x3;
                for _ in 0..num {
                    let k = rdr.read_u8().unwrap();
                    let l = rdr.read_u16::<BigEndian>().unwrap();
                    SLIP_CMD.lock().unwrap().insert(k, l);
                }
            }
            GAME_START => {
                let mlen = *SLIP_CMD.lock().unwrap().get(&cmd).unwrap() as i64;
                rdr.seek(SeekFrom::Current(mlen)).unwrap();
                res = GAME_START;
            }
            GAME_END => {
                let mlen = *SLIP_CMD.lock().unwrap().get(&cmd).unwrap() as i64;
                rdr.seek(SeekFrom::Current(mlen)).unwrap();
                res = GAME_END;
            }
            PRE_FRAME => {
                let mlen = *SLIP_CMD.lock().unwrap().get(&cmd).unwrap() as i64;
                rdr.seek(SeekFrom::Current(mlen)).unwrap();
            }
            POST_FRAME => {
                let mlen = *SLIP_CMD.lock().unwrap().get(&cmd).unwrap() as i64;
                rdr.seek(SeekFrom::Current(mlen)).unwrap();
            }
            _ => {}
        };
    }
    res
}
