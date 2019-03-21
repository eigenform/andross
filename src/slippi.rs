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
                parse_game_start(&mut rdr);
                res = GAME_START;
            }
            GAME_END => {
                parse_game_end(&mut rdr);
                res = GAME_END;
            }
            PRE_FRAME => {
                parse_pre_frame(&mut rdr);
            }
            POST_FRAME => {
                parse_post_frame(&mut rdr);
            }
            _ => {}
        };
    }
    res
}

fn parse_game_start(buf: &mut Cursor<&Vec<u8>>) {
    let mlen = *SLIP_CMD.lock().unwrap().get(&GAME_START).unwrap() as i64;

    // ...

    buf.seek(SeekFrom::Current(mlen)).unwrap();
}

fn parse_game_end(buf: &mut Cursor<&Vec<u8>>) {
    let mlen = *SLIP_CMD.lock().unwrap().get(&GAME_END).unwrap() as i64;

    //let gameEndMethod = buf.read_u8().unwrap();

    buf.seek(SeekFrom::Current(mlen)).unwrap();
}

fn parse_pre_frame(buf: &mut Cursor<&Vec<u8>>) {
    let mlen = *SLIP_CMD.lock().unwrap().get(&PRE_FRAME).unwrap() as i64;

    //let frame = buf.read_i32::<BigEndian>().unwrap();
    //let playerIndex = buf.read_u8().unwrap();
    //let isFollower = buf.read_u8().unwrap();
    //let seed = buf.read_u32::<BigEndian>().unwrap();
    //let actionState = buf.read_u16::<BigEndian>().unwrap();
    //let positionX = buf.read_f32::<BigEndian>().unwrap();
    //let positionY = buf.read_f32::<BigEndian>().unwrap();
    //let facingDirection = buf.read_f32::<BigEndian>().unwrap();
    //let joystickX = buf.read_f32::<BigEndian>().unwrap();
    //let joystickY = buf.read_f32::<BigEndian>().unwrap();
    //let cStickX = buf.read_f32::<BigEndian>().unwrap();
    //let cStickY = buf.read_f32::<BigEndian>().unwrap();
    //let trigger = buf.read_f32::<BigEndian>().unwrap();
    //let buttons = buf.read_u32::<BigEndian>().unwrap();
    //let physicalButtons = buf.read_u16::<BigEndian>().unwrap();
    //let physicalLTrigger = buf.read_f32::<BigEndian>().unwrap();
    //let physicalRTrigger = buf.read_f32::<BigEndian>().unwrap();
    //let rawJoystickX = buf.read_u8().unwrap();
    //let percent = buf.read_f32::<BigEndian>().unwrap();

    buf.seek(SeekFrom::Current(mlen)).unwrap();
}

fn parse_post_frame(buf: &mut Cursor<&Vec<u8>>) {
    let mlen = *SLIP_CMD.lock().unwrap().get(&POST_FRAME).unwrap() as i64;

    //let frame = buf.read_i32::<BigEndian>().unwrap();
    //let playerIndex = buf.read_u8().unwrap();
    //let isFollower = buf.read_u8().unwrap();
    //let internalCharID = buf.read_u8().unwrap();
    //let actionState = buf.read_u16::<BigEndian>().unwrap();
    //let positionX = buf.read_f32::<BigEndian>().unwrap();
    //let positionY = buf.read_f32::<BigEndian>().unwrap();
    //let facingDirection = buf.read_f32::<BigEndian>().unwrap();
    //let percent = buf.read_f32::<BigEndian>().unwrap();
    //let shieldSize = buf.read_f32::<BigEndian>().unwrap();
    //let lastAttackLanded = buf.read_u8().unwrap();
    //let currentComboCount = buf.read_u8().unwrap();
    //let lastHitBy = buf.read_u8().unwrap();
    //let stocksRemaining = buf.read_u8().unwrap();
    //let actionStateCounter = buf.read_f32::<BigEndian>().unwrap();

    buf.seek(SeekFrom::Current(mlen)).unwrap();
}
