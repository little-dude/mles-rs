#[macro_use]
extern crate serde_derive;


extern crate futures;
extern crate tokio_core;
extern crate serde_cbor;
extern crate byteorder;

use std::env;
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::thread;

use futures::{Sink, Future, Stream};
use futures::sync::mpsc;
use tokio_core::reactor::Core;
use tokio_core::io::{Io, EasyBuf, Codec};
use tokio_core::net::TcpStream;
use std::io::Cursor;
use byteorder::{BigEndian, WriteBytesExt, ReadBytesExt};

#[derive(Serialize, Deserialize, Debug)]
pub struct Msg {
    message: Vec<Vec<u8>>,
}

pub fn message_encode(msg: &Msg) -> Vec<u8> {
    let encoded = serde_cbor::to_vec(msg);
    match encoded {
        Ok(encoded) => encoded,
        Err(err) => {
            println!("Error on encode: {}", err);
            Vec::new()
        }
    }
}

pub fn message_decode(slice: &[u8]) -> Msg {
    let value = serde_cbor::from_slice(slice);
    match value {
        Ok(value) => value,
        Err(err) => {
            println!("Error on decode: {}", err);
            Msg { message: Vec::new() } // return empty vec in case of error
        }
    }
}


fn read_n<R>(reader: R, bytes_to_read: u64) -> Vec<u8>
where R: Read,
{
    let mut buf = vec![];
    let mut chunk = reader.take(bytes_to_read);
    let status = chunk.read_to_end(&mut buf);
    match status {
        Ok(n) => assert_eq!(bytes_to_read as usize, n),
            _ => return vec![]
    }
    buf
 }

fn read_hdr_type(hdr: &[u8]) -> u32 { 
    let mut buf = Cursor::new(&hdr[..]);
    let num = buf.read_u32::<BigEndian>().unwrap();
    num >> 24
}

fn read_hdr_len(hdr: &[u8]) -> usize { 
    let mut buf = Cursor::new(&hdr[..]);
    let num = buf.read_u32::<BigEndian>().unwrap();
    (num & 0xfff) as usize
}

fn write_hdr(len: usize) -> Vec<u8> {
    let hdr = (('M' as u32) << 24) | len as u32;
    let mut msgv = vec![];
    msgv.write_u32::<BigEndian>(hdr).unwrap();
    msgv
}



fn main() {
    // Parse what address we're going to connect to
    //let addr = env::args().nth(1).unwrap_or_else(|| {
    //    panic!("this program requires at least one argument")
    //});
    let addr = "127.0.0.1:8081";
    let addr = addr.parse::<SocketAddr>().unwrap();

    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let tcp = TcpStream::connect(&addr, &handle);

    // Handle stdin in a separate thread 
    let (stdin_tx, stdin_rx) = mpsc::channel(0);
    thread::spawn(|| read_stdin(stdin_tx));
    let stdin_rx = stdin_rx.map_err(|_| panic!()); // errors not possible on rx

    let mut stdout = io::stdout();
    let client = tcp.and_then(|stream| {
        let (sink, stream) = stream.framed(Bytes).split();
        let send_stdin = stdin_rx.forward(sink);
        let write_stdout = stream.for_each(move |mut buf| {
            let payload = buf.split_off(4); // strip header
            let len = payload.len();
            println!("Got payload len {}", len);
            let decoded = message_decode(payload.as_slice());
            let mut msg = "".to_string();
            if 0 == decoded.message.len() {
                println!("Error happened");
            }
            else {
                msg.push_str(String::from_utf8_lossy(decoded.message[0].as_slice()).into_owned().as_str());
                msg.push_str(":");
                msg.push_str(String::from_utf8_lossy(decoded.message[2].as_slice()).into_owned().as_str());
            }
            stdout.write_all(&msg.into_bytes())
        });

        send_stdin.map(|_| ())
        .select(write_stdout.map(|_| ()))
        .then(|_| Ok(()))
    });

    core.run(client).unwrap();
}

struct Bytes;

impl Codec for Bytes {
    type In = EasyBuf;
    type Out = Vec<u8>;

    fn decode(&mut self, buf: &mut EasyBuf) -> io::Result<Option<EasyBuf>> {
        if buf.len() >= 4 { // 4 is header min size
            if read_hdr_type(buf.as_slice()) != 'M' as u32 {
                return Ok(None);  //TODO proper error handling here 
            }
            let mut hdr_len = read_hdr_len(buf.as_slice()); 
            if 0 == hdr_len {
                return Ok(None);  //TODO proper error handling here 
            }
            let mut len = buf.len();
            if len < (4 + hdr_len) {
                return Ok(None); 
            }
            if 4 + hdr_len < len { 
                println!("Hdr len {}", hdr_len);
                return Ok(Some(buf.drain_to(4 + hdr_len)));
            }
            Ok(Some(buf.drain_to(len)))
        } else {
            Ok(None)
        }
    }

    fn encode(&mut self, data: Vec<u8>, buf: &mut Vec<u8>) -> io::Result<()> {
        buf.extend(data);
        Ok(())
    }
}

fn read_stdin(mut rx: mpsc::Sender<Vec<u8>>) {
    let mut stdin = io::stdin();
    let mut stdout = io::stdout();
    /* Set user */
    stdout.write_all(b"User name?\n");
    let mut buf = vec![0; 80];
    let n = match stdin.read(&mut buf) {
        Err(_) |
            Ok(0) => return,
            Ok(n) => n,
    };
    buf.truncate(n-1);
    let user = buf.clone();
    let userstr = String::from_utf8_lossy(buf.clone().as_slice()).into_owned();

    /* Set channel */
    stdout.write_all(b"Channel?\n");
    let mut buf = vec![0; 80];
    let n = match stdin.read(&mut buf) {
        Err(_) |
            Ok(0) => return,
            Ok(n) => n,
    };
    buf.truncate(n-1);
    let channel = buf.clone();
    let channelstr = String::from_utf8_lossy(buf.clone().as_slice()).into_owned();

    let mut msg = String::from_utf8_lossy(user.as_slice()).into_owned();
    msg += "::";
    let str =  String::from_utf8_lossy(channel.clone().as_slice()).into_owned();
    msg += str.as_str();

    let mut welcome = "Welcome to ".to_string();
    welcome += msg.as_str();
    welcome += "!\n";
    stdout.write_all(welcome.as_bytes());

    let mut msgvec: Vec<Vec<u8>> = Vec::new();
    msgvec.push(userstr.into_bytes());
    msgvec.push(channelstr.into_bytes());
    let msg = message_encode(&Msg { message: msgvec.clone() });
    println!("Payload len {}", msg.len());
    let mut msgv = write_hdr(msg.len());
    msgv.extend(msg);
    println!("Msgv {:?}", msgv);
    rx = rx.send(msgv).wait().unwrap();

    loop {
        let mut buf = vec![0;80];
        let mut msgv: Vec<Vec<u8>> = msgvec.clone();
        let n = match stdin.read(&mut buf) {
            Err(_) |
                Ok(0) => break,
                Ok(n) => n,
        };
        buf.truncate(n);
        let str =  String::from_utf8_lossy(buf.as_slice()).into_owned();
        msgv.push(str.into_bytes());
        let msg = message_encode(&Msg { message: msgv });
        println!("Payload len {}", msg.len());
        let mut msgv = write_hdr(msg.len());
        msgv.extend(msg);
        println!("Msgv {:?}", msgv);
        rx = rx.send(msgv).wait().unwrap();
    }
}

