extern crate futures;
extern crate tokio_core;
extern crate tokio_proto;
extern crate tokio_service;
extern crate hyper;
extern crate base64;
extern crate clap;
extern crate byteorder;

use std::io;
use std::cell::{Cell, RefCell};
use std::sync::Arc;
use futures::{future, Future, BoxFuture, Stream, Async};
use hyper::{header::ContentLength, Client, Uri, Method, Request, StatusCode};
use tokio_core::{io::{Io, Codec, Framed, EasyBuf}, reactor::Handle};
use tokio_proto::TcpServer;
use tokio_proto::pipeline::ServerProto;
use tokio_service::Service;
use byteorder::{ByteOrder, LittleEndian};

#[macro_use]
extern crate log;
extern crate env_logger;

#[derive(Clone, Debug, PartialEq)]
pub struct BgbControlFlags{
    /* bit 0 */ // should be 1
    /* bit 1 */ high_speed:   bool,
    /* bit 2 */ double_speed: bool,
    /* bit 3 */ // should be zero
    /* bit 4 */ // should be zero
    /* bit 5 */ // should be zero
    /* bit 6 */ // should be zero
    /* bit 7 */ // should be 1
}

impl BgbControlFlags{
    pub fn from_u8(b: u8) -> BgbControlFlags{
        BgbControlFlags{
            high_speed:   (b & 0b010) != 0, // true if bit 1 is set
            double_speed: (b & 0b100) != 0, // true if bit 2 is set
        }
    }
    pub fn to_u8(&self) -> u8{
        let mut b = 0u8; // meh imperative will do
        if self.high_speed{
            b |= 0b010;
        }
        if self.double_speed{
            b |= 0b100;
        }
        b
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BgbStatusFlags{
    /* bit 0 */ running: bool,
    /* bit 1 */ paused: bool,
    /* bit 2 */ support_reconnect: bool,
    // rest zero
}

impl BgbStatusFlags{
    pub fn from_u8(b: u8) -> BgbStatusFlags{
        BgbStatusFlags{
            running:           (b & 0b001) != 0, // true if bit 0 is set
            paused:            (b & 0b010) != 0, // true if bit 1 is set
            support_reconnect: (b & 0b100) != 0, // true if bit 2 is set
        }
    }
    pub fn to_u8(&self) -> u8{
        let mut b = 0u8;
        if self.running{
            b |= 0b001;
        }
        if self.paused{
            b |= 0b010;
        }
        if self.support_reconnect{
            b |= 0b100;
        }
        b
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BgbJoypadUpdate{
    button: u8,
    pressed: bool,
}

impl BgbJoypadUpdate{
    pub fn from_u8(b: u8) -> BgbJoypadUpdate{
        BgbJoypadUpdate{
            button:  (b & 0b0111), // value of bits 0-2 inclusive
            pressed: (b & 0b1000) != 0, // true if bit 3 is set
        }
    }
    pub fn to_u8(&self) -> u8{
        let mut b = self.button;
        if self.pressed{
            b |= 0b1000;
        }
        b
    }
}

#[allow(non_snake_case)]
mod CommandConsts{
    pub const VERSION:        u8 =   1;
    pub const JOYPAD:         u8 = 101;
    pub const SYNC1:          u8 = 104;
    pub const SYNC2:          u8 = 105;
    pub const SYNC3:          u8 = 106;
    pub const STATUS:         u8 = 108;
    pub const WANTDISCONNECT: u8 = 109;
}

const BGB_COMMAND_LEN: usize = 4;
#[derive(Clone, Debug, PartialEq)]
pub enum BgbCommand{
    /*   1 */ Version{ major: u8, minor: u8, patch: u8 }, // timestamp should be zero
    /* 101 */ Joypad{ update: BgbJoypadUpdate }, // b3, b4 = 0, timestamp should be zero
    /* 104 */ Sync1{ data: u8, control: BgbControlFlags }, // b4 always 0
    /* 105 */ Sync2{ data: u8 }, // b3 should be 0x80, timestamp should be zero
    /* 106 */ Sync3A, // b2 should be 1, b3,b4 are "deprecated", assuming zero. Timestamp should be zero
    /* 106 */ Sync3B, // b2 should be zero, assuming same for b3,b4. Timestamp should be populated.
    /* 108 */ Status{ flags: BgbStatusFlags }, // b3, b4 = 0, timestamp should be zero
    /* 109 */ WantDisconnect, // all fields should be zero
    Empty, // send nothing to the client
}

impl BgbCommand{
    pub fn from_slice(s: &[u8]) -> BgbCommand{
        assert!(s.len() >= 4,
            "BgbCommands are {} bytes, tried to decode slice smaller than {} bytes",
            BGB_COMMAND_LEN, BGB_COMMAND_LEN);
        Self::from_bytes(s[0], s[1], s[2], s[3])
    }
    pub fn from_bytes(b1: u8, b2: u8, b3: u8, b4: u8) -> BgbCommand{
        use BgbCommand::*;
        use CommandConsts::*;
        match b1{
            VERSION => Version{ major: b2, minor: b3, patch: b4 },
            JOYPAD  => Joypad{ update: BgbJoypadUpdate::from_u8(b2) },
            SYNC1   => Sync1{ data: b2, control: BgbControlFlags::from_u8(b3) },
            SYNC2   => Sync2{ data: b2 },
            SYNC3   => if b2 == 1 { Sync3A } else { Sync3B },
            STATUS  => Status{ flags: BgbStatusFlags::from_u8(b2) },
            _       => panic!("unknown bgb command {}", b1)
        }
    }
    pub fn to_array(self) -> [u8; 4]{
        use BgbCommand::*;
        use CommandConsts::*;

        match self{
            Version{ major, minor, patch } => [VERSION,         major,           minor, patch],
            Joypad{ update }               => [ JOYPAD,update.to_u8(),             0u8,   0u8],
            Sync1{ data, control }         => [  SYNC1,          data, control.to_u8(),   0u8],
            Sync2{ data }                  => [  SYNC2,          data,            0x80,   0u8],
            Sync3A                         => [  SYNC3,             1,               0,     0],
            Sync3B                         => [  SYNC3,             0,               0,     0],
            Status{ flags }                => [ STATUS, flags.to_u8(),               0,     0],
            WantDisconnect                 => [WANTDISCONNECT,      0,               0,     0],
            Empty /* shouldn't get here */ => unreachable!()
        }
    }
    pub fn to_packet(self, ts: BgbTimestamp) -> BgbPacket{
        if self.should_send_timestamp(){
            BgbPacket{
                command: self,
                timestamp: ts
            }
        }
        else{
            BgbPacket{
                command: self,
                timestamp: BgbTimestamp(0)
            }
        }
    }
    pub fn should_send_timestamp(&self) -> bool{
        use BgbCommand::*;
        match *self{
            Sync1{..} => true,
            Sync3B    => true,
            _         => false
        }
    }
}

const BGB_TIMESTAMP_LEN: usize = 4;
#[derive(Copy, Clone, Debug)]
pub struct BgbTimestamp(u32); // timestamp in 2MHz clocks, MSB always zero

impl BgbTimestamp{
    pub fn from_network(s: &[u8]) -> BgbTimestamp{
        BgbTimestamp(byteorder::LittleEndian::read_u32(s))
    }
    pub fn to_network(&self, s: &mut[u8]){ // result in s
        let BgbTimestamp(x) = *self;
        // BGB protocol uses little-endian on the wire
        byteorder::LittleEndian::write_u32(s, x);
    }
    pub fn to_network_array(&self) -> [u8; 4]{
        let mut a = [0u8, 0u8, 0u8, 0u8];
        self.to_network(&mut a);
        a
    }
}

const BGB_PACKET_LEN: usize = BGB_COMMAND_LEN + BGB_TIMESTAMP_LEN;
pub struct BgbPacket{
    command: BgbCommand,
    timestamp: BgbTimestamp
}

impl BgbPacket{
    pub fn from_slice(s: &[u8]) -> BgbPacket{
        assert!(s.len() >= BGB_PACKET_LEN, "BgbPackets are {} bytes, tried to decode a slice smaller than that", BGB_PACKET_LEN);
        BgbPacket{
            command:   BgbCommand::from_slice(&s[0..BGB_COMMAND_LEN]),
            timestamp: BgbTimestamp::from_network(&s[BGB_COMMAND_LEN..BGB_PACKET_LEN])
        }
    }
}

#[derive(Default)]
pub struct BgbCodec{
    sent_version: bool
}

impl Codec for BgbCodec{
    type In  = BgbPacket;
    type Out = BgbPacket;

    fn decode(&mut self, buf: &mut EasyBuf) -> Result<Option<Self::Out>, io::Error> {
        if buf.len() < BGB_PACKET_LEN {
            Ok(None) // buffer not ready yet
        }
        else{
            let packet = buf.drain_to(BGB_PACKET_LEN);
            Ok(Some(BgbPacket::from_slice(packet.as_slice())))
        }
    }

    fn decode_eof(&mut self, buf: &mut EasyBuf) -> io::Result<Self::Out>{
        let amt = buf.len();
        Ok(BgbPacket::from_slice(buf.drain_to(amt).as_slice()))
    }

    fn encode(&mut self, item: Self::In, into: &mut Vec<u8>) -> io::Result<()>{
        // if haven't already sent Version, send it before the first outgoing message.
        // Ideally, we'd send this immediately without waiting for Version from the client,
        // but that's kind of difficult to do with tokio_proto right now. So just hope that
        // the client promptly sends us Version first, so we can tack it on before the Status
        // reply.
        if !self.sent_version{
            // always claim to be 1.4.0
            let version = BgbCommand::Version{ major: 1, minor: 4, patch: 0 };
            let vpacket = version.to_packet(BgbTimestamp(0));
            into.extend_from_slice(&vpacket.command.to_array()[..]);
            into.extend_from_slice(&vpacket.timestamp.to_network_array()[..]);
            self.sent_version = true;
        }
        if item.command == BgbCommand::Empty{
            // don't do anything when item.command is empty
            return Ok(())
        }
        into.extend_from_slice(&item.command.to_array()[..]);
        into.extend_from_slice(&item.timestamp.to_network_array()[..]);
        Ok(())
    }
}

pub struct BgbProto;
impl<T: Io + 'static> ServerProto<T> for BgbProto {
    type Request = BgbPacket;
    type Response = BgbPacket;
    type Transport = Framed<T, BgbCodec>;
    type BindTransport = Result<Self::Transport, io::Error>;

    fn bind_transport(&self, io: T) -> Self::BindTransport {
        Ok(io.framed(BgbCodec::default()))
    }
}

const MAX_REQ_SIZE: usize = 1280; // from serial_deobfuscated.js
pub enum ZzazzSerialState{
    Sync  (SyncState),
    GetLen(GetLenState),
    GetReq(GetReqState),
    Send  (SendState),
    Poll  (PollState),
    Recv  (RecvState)
}

impl Default for ZzazzSerialState{
    fn default() -> Self{
        ZzazzSerialState::Sync(SyncState::default())
    }
}

#[derive(Default)]
pub struct SyncState{ i: usize }
#[derive(Default)]
pub struct GetLenState{ l: Vec<u8> }
// cannot have a default for this
pub struct GetReqState{ d: Vec<u8>, s: usize }
#[derive(Default)]
pub struct SendState{ d: Vec<u8> }
// cannot have a default for this
pub struct PollState{ f: Box<Future<Item=String, Error=()>> }
// cannot have a default for this
pub struct RecvState{ d: Vec<u8>, i: usize }

impl SyncState{
    pub fn update(self, byte: u8) -> (ZzazzSerialState, u8){
        use ZzazzSerialState::*;
        const IN_SYNC: [u8;3] = [218, 207, 235];
        const OUT_SYNC:[u8;3] = [165,  90,  10];

        let i = self.i;

        if byte == IN_SYNC[i] {
            info!("sync {} match", i);
            let out = OUT_SYNC[i];
            if (i + 1) >= OUT_SYNC.len() {
                (GetLen(GetLenState::default()), out) // advance to GetLen
            }
            else{
                (Sync(SyncState{ i: (i + 1) }), out) // next handshake phase
            }
        }
        else{
            (Sync(SyncState{ i: 0 }), 0u8) // reset handshake
        }
    }
}

impl GetLenState{
    pub fn update(self, byte: u8) -> (ZzazzSerialState, u8){
        use ZzazzSerialState::*;
        const SUCCESS:u8 = 204;
        const FAIL:   u8 = 0;

        let mut l = self.l;

        l.push(byte);
        if l.len() >= 2 { // sizeof int16le
            let req_size = LittleEndian::read_u16(&l[..]);
            info!("got req_size {}", req_size);
            if req_size as usize > MAX_REQ_SIZE{
                error!("excess size {} is > {}", req_size, MAX_REQ_SIZE);
                // size exceeded, reset to Sync (unlike JS version)
                return (Sync(SyncState::default()), FAIL);
            }
            // I can't work out why from the source, but it works if the first two elems of d are
            // the size.
            (GetReq(GetReqState{ d: l, s: req_size as usize }), SUCCESS) // advance to GetReq
        }
        else{
            (GetLen(GetLenState{l}), SUCCESS)
        }
    }
}

impl GetReqState{
    pub fn update(self, byte: u8) -> (ZzazzSerialState, u8){
        use ZzazzSerialState::*;
        const SUCCESS:u8 = 204;

        let mut d = self.d;
        let s = self.s;
        
        d.push(byte);
        if d.len() >= s { // if reached size
            info!("got {} req bytes: {:?}", s, d);
            (Send(SendState{ d }), SUCCESS)
        }
        else{
            (GetReq(GetReqState{ d, s }), SUCCESS)
        }
    }
}

impl SendState{
    pub fn update(self, handle: &Handle, uri: &Uri, byte: u8) -> (ZzazzSerialState, u8){
        use ZzazzSerialState::*;
        // in either case, return 102
        const RETURN:u8 = 102;

        let d = self.d;
        
        if byte != 85 {
            // if byte isn't 85, cancel and reset to Sync
            info!("byte {} in SendState wasn't 85, resetting", byte);
            (Sync(SyncState::default()), RETURN)
        }
        else{
            let mut req = Request::new(Method::Post, uri.clone());
            let b64 = base64::encode(&d[..]);
            info!("cli -> srv: [{}]", b64);
            req.headers_mut().set(ContentLength(b64.len() as u64));
            req.set_body(b64);

            (Poll(PollState{ f: Box::new(Client::new(handle)
                                .request(req)
                                .map_err(|_| ())
                                .and_then(|resp|{
                                    if resp.status() == StatusCode::Ok{
                                        info!("got OK response from server, awaiting body");
                                    }
                                    resp.body()
                                        .concat2()
                                        .map_err(|_|())
                                        .map(|chunk|{
                                            let v = chunk.to_vec();
                                            String::from_utf8_lossy(&v).to_string()
                                        })
                                }))
                  }), RETURN)
        }
    }
}

impl PollState{
    pub fn update(self, byte: u8) -> (ZzazzSerialState, u8){
        use ZzazzSerialState::*;
        const SUCCESS: u8 = 51;
        const NOTREADY:u8 = 102;
        const FAIL:    u8 = 255;

        let mut f = self.f;
        
        if byte != 85 {
            // if byte isn't 85, cancel and reset to Sync
            info!("byte {} in PollState wasn't 85, resetting", byte);
            return (Sync(SyncState::default()), FAIL);
        } 
        match f.poll(){
            Ok(Async::Ready(resp)) => {
                if true{//let Ok(b64) = resp{
                    let b64 = resp;
                    info!("srv -> cli: [{}]", b64);
                    let data = base64::decode(&b64[..]).expect("Failed to decode base64");

                    (Recv(RecvState{ d: data, i: 0 }), SUCCESS) // 51 on transition to Recv
                }
                else{
                    // reset to Sync on Err
                    info!("Response status code not OK in PollState, resetting");
                    (Sync(SyncState::default()), FAIL)
                }
            },
            Ok(Async::NotReady) => {
                (Poll(PollState{ f }), NOTREADY)
            },
            Err(_) => {
                // reset to Sync on Err
                info!("Request error in PollState, resetting");
                (Sync(SyncState::default()), FAIL)
            }
        }
    }
}

impl RecvState{
    pub fn update(self, byte: u8) -> (ZzazzSerialState, u8){
        use ZzazzSerialState::*;
        let d = self.d;
        let i = self.i;

        let out = d[i]; // always return this
        if (i + 1) >= d.len() || byte != 204 { // if done or byte wasn't 204, reset to sync
            info!("done receiving");
            return (Sync(SyncState::default()), out);
        }
        else{
            return (Recv(RecvState{ d, i: i+1 }), out);
        }
    }
}

impl ZzazzSerialState{
    pub fn update(self, handle: &Handle, uri: &Uri, byte: u8) -> (Self, u8){
        use ZzazzSerialState::*;
        match self{
            Sync(sy)   => sy.update(byte),
            GetLen(gl) => gl.update(byte),
            GetReq(gr) => gr.update(byte),
            Send(se)   => se.update(handle, uri, byte),
            Poll(po)   => po.update(byte),
            Recv(re)   => re.update(byte)
        }
    }
}

pub struct BgbToHttp{
    timestamp: Cell<BgbTimestamp>,
    handle:    Handle,
    uri:       Uri,
    state:     RefCell<ZzazzSerialState>
}

impl Service for BgbToHttp{
    type Request  = BgbPacket;
    type Response = BgbPacket;
    type Error    = io::Error;
    type Future   = BoxFuture<Self::Response, Self::Error>;

    fn call(&self, req: Self::Request) -> Self::Future{
        use BgbCommand::*;
        use ZzazzSerialState::Sync;
        trace!("got req {:?} timestamp {:?}", req.command, req.timestamp);

        let status = || Status{
            flags: BgbStatusFlags{
                running: true,
                paused:  false,
                support_reconnect: false
            }
        };


        let resp = match req.command {
            Version{ .. } => status(),
            Joypad{..} => {
                // actually I have NFI what we should do here, don't send anything back
                Empty
            },
            Sync1{ data, .. } => {
                self.timestamp.set(req.timestamp);
                // ugly hack to let us move state out of self.state
                let mut state = self.state.replace(Sync(SyncState::default()));
                let (new_state, data) = state.update(&self.handle, &self.uri, data);
                self.state.replace(new_state);
                Sync2{ data }
            },
            Sync2{ data } => {
                error!("Got a sync2 from the client which is supposed to be a master! Responding with a sync2 anyway");
               
                // ugly hack to let us move state out of self.state
                let mut state = self.state.replace(Sync(SyncState::default()));
                let (new_state, data) = state.update(&self.handle, &self.uri, data);
                self.state.replace(new_state);
                Sync2{ data }
            },
            Sync3A => {
                // actually I have NFI what we should do here, don't send anything back
                Empty
            },
            Sync3B => {
                self.timestamp.set(req.timestamp);
                Sync3B
            },
            Status{..} => Sync3B, // idk actually
            WantDisconnect => WantDisconnect,
            Empty => Empty // not possible for client to send Empty
        };
        trace!("sending resp {:?}", resp);
        future::finished(resp.to_packet(self.timestamp.get())).boxed()
    }
}

fn main() {
    use clap::Arg;

    env_logger::init();

    let args = clap::App::new("BGB2HTTP")
        .version("0.1")
        .about("Adapter between BGB emulator's TCP/IP link protocol and TheZZAZZGlitch's HTTP POST-based April Fools server")
        .author("selenologist")
        .arg(Arg::with_name("server")
             .short("s")
             .help("IP:PORT of remote HTTP server"))
        .arg(Arg::with_name("port")
             .short("p")
             .help("port of local server for BGB to connect to"))
        .arg(Arg::with_name("id")
             .short("i")
             .index(1)
             .required(true)
             .help("d_sessid pulled from LocalStorage from the online client (sorry, i'm not sure how to obtain it from username/password)"))
        .get_matches();

    let server = args.value_of("server").unwrap_or("167.99.192.164:12709"); // default to TheZZAZZGlitch's server
    let port   = args.value_of("port").unwrap_or("8765"); // default to the same as the Windows adapter according to the screenshot
    let sessid = args.value_of("id").expect("sessid required but not provided");
 
    // assumes server and sessid are well formed
    let uri: Arc<Uri> = Arc::new(format!("http://{}/req/{}", server, sessid)
        .parse()
        .expect("Failed to parse HTTP server request URI, check server and id parameters"));
    println!("Using {} as remote HTTP server URI", uri);
    
    let addr = format!("127.0.0.1:{}", port)
        .parse()
        .expect("Failed to parse BGB listening address, check port parameter");
    
    println!("Listening on {:?}", addr);
    TcpServer::new(BgbProto, addr)
        .with_handle(move |handle: &Handle|{
            // hack around not being able to pass Handle across threads
            let remote = Arc::new(handle.remote().clone());
            let uri = uri.clone();
            move || Ok(BgbToHttp{
                timestamp: Cell::new(BgbTimestamp(0)),
                handle: remote.handle().unwrap(),
                uri: (*uri).clone(),
                state: RefCell::new(ZzazzSerialState::default())
            })
        });
}
