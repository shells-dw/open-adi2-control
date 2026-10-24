//! The ADI-2's USB-MIDI port through the ALSA sequencer.
//!
//! The sequencer lets several programs use a MIDI port at once, so this works next to PipeWire
//! (whose MIDI bridge holds the raw MIDI device open). The app creates its own sequencer client
//! "Open ADI-2 Control", finds the ADI-2's port by name, subscribes to what the unit sends and sends
//! SysEx directly to the unit (not to other subscribers). System announcements tell it when the
//! unit comes or goes.

use crate::device::{model_hint, In, LinkEvent, Transport};
use crate::model::Problem;
use alsa::seq::{Addr, ClientIter, Event, EventType, PortCap, PortInfo, PortIter, PortSubscribe, PortType, Seq};
use alsa::{poll, Direction, PollDescriptors};
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CLIENT_NAME: &str = "Open ADI-2 Control";
const RESCAN_EVERY: Duration = Duration::from_secs(3);
const REOPEN_EVERY: Duration = Duration::from_secs(5);

struct Shared {
    seq: Option<Seq>,
    my_port: i32,
    adi: Option<Addr>,
}

pub struct SeqTransport {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
}

impl Transport for SeqTransport {
    fn write(&self, buf: &[u8]) -> bool {
        let sh = self.shared.lock().unwrap();
        let (Some(seq), Some(dest)) = (sh.seq.as_ref(), sh.adi) else { return false };
        let mut ev = Event::new_ext(EventType::Sysex, buf);
        ev.set_source(sh.my_port);
        ev.set_dest(dest);
        ev.set_direct();
        seq.event_output_direct(&mut ev).is_ok()
    }

    fn close(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// An ADI-2 port seen on the sequencer.
#[derive(Debug, Clone)]
pub struct Found {
    pub addr: Addr,
    pub name: String,
}

fn open_client() -> alsa::Result<(Seq, i32)> {
    let seq = Seq::open(None, None, true)?;
    seq.set_client_name(&CString::new(CLIENT_NAME).unwrap())?;
    let mut pi = PortInfo::empty()?;
    pi.set_capability(PortCap::READ | PortCap::WRITE | PortCap::SUBS_WRITE);
    // (READ: this port sends to the unit through a connection; WRITE: it receives from it)
    pi.set_type(PortType::MIDI_GENERIC | PortType::APPLICATION);
    pi.set_name(&CString::new("Remote").unwrap());
    seq.create_port(&pi)?;
    let port = pi.get_port();
    // client/port start and exit announcements
    let me = Addr { client: seq.client_id()?, port };
    let sub = PortSubscribe::empty()?;
    sub.set_sender(Addr::system_announce());
    sub.set_dest(me);
    seq.subscribe_port(&sub)?;
    Ok((seq, port))
}

/// The ADI-2's port: a port that can be read and written, of a client named like the unit
/// (the kernel client of its USB card). Bridges such as a2jmidid name their ports after the
/// unit too; those are only used when there's nothing better.
pub fn find_adi(seq: &Seq) -> Option<Found> {
    let me = seq.client_id().ok();
    let mut fallback = None;
    for c in ClientIter::new(seq) {
        let cid = c.get_client();
        if Some(cid) == me || cid == 0 {
            continue;
        }
        let cname = c.get_name().unwrap_or("").to_string();
        for pinfo in PortIter::new(seq, cid) {
            let caps = pinfo.get_capability();
            if !(caps.contains(PortCap::READ | PortCap::SUBS_READ) && caps.contains(PortCap::WRITE)) {
                continue;
            }
            let pname = pinfo.get_name().unwrap_or("").to_string();
            if model_hint(&cname).is_some() {
                return Some(Found { addr: pinfo.addr(), name: cname });
            }
            if fallback.is_none() && model_hint(&pname).is_some() {
                fallback = Some(Found { addr: pinfo.addr(), name: pname });
            }
        }
    }
    fallback
}

pub fn start(engine: Sender<In>) -> SeqTransport {
    let shared = Arc::new(Mutex::new(Shared { seq: None, my_port: 0, adi: None }));
    let stop = Arc::new(AtomicBool::new(false));
    let (sh, st) = (shared.clone(), stop.clone());
    std::thread::Builder::new()
        .name("alsa-seq".into())
        .spawn(move || run(sh, st, engine))
        .expect("seq thread");
    SeqTransport { shared, stop }
}

fn send(engine: &Sender<In>, ev: LinkEvent) -> bool {
    engine.send(In::Link(ev)).is_ok()
}

fn run(shared: Arc<Mutex<Shared>>, stop: Arc<AtomicBool>, engine: Sender<In>) {
    let mut last_problem: Option<String> = None;
    let mut problem = |engine: &Sender<In>, p: Option<Problem>| {
        let code = p.as_ref().map(|p| p.code.clone());
        if code != last_problem {
            last_problem = code;
            send(engine, LinkEvent::Problem(p));
        }
    };

    while !stop.load(Ordering::SeqCst) {
        let (seq, port) = match open_client() {
            Ok(x) => x,
            Err(e) => {
                problem(
                    &engine,
                    Some(Problem::new("noseq", format!("Can't open the ALSA sequencer /dev/snd/seq (error {}). Is the snd-seq kernel module loaded?", e.errno()))),
                );
                sleep_unless(&stop, REOPEN_EVERY);
                continue;
            }
        };
        {
            let mut s = shared.lock().unwrap();
            s.seq = Some(seq);
            s.my_port = port;
            s.adi = None;
        }
        let fds = {
            let s = shared.lock().unwrap();
            let seq = s.seq.as_ref().unwrap();
            (seq, Some(Direction::Capture)).get().unwrap_or_default()
        };
        let mut rescan = true;
        let mut last_scan = Instant::now() - RESCAN_EVERY;
        let mut buf_events: Vec<Vec<u8>> = Vec::new();

        'connected: while !stop.load(Ordering::SeqCst) {
            // (re)connect to the unit
            if rescan || (shared.lock().unwrap().adi.is_none() && last_scan.elapsed() >= RESCAN_EVERY) {
                rescan = false;
                last_scan = Instant::now();
                let mut s = shared.lock().unwrap();
                let seq = s.seq.as_ref().unwrap();
                let found = find_adi(seq);
                match (found, s.adi) {
                    (Some(f), None) => {
                        let me = Addr { client: seq.client_id().unwrap_or(0), port: s.my_port };
                        // both directions: the kernel opens the unit's MIDI output only while a
                        // port is connected to it, so SysEx sent without this connection is dropped
                        let connect = |from: Addr, to: Addr| -> alsa::Result<()> {
                            let sub = PortSubscribe::empty()?;
                            sub.set_sender(from);
                            sub.set_dest(to);
                            match seq.subscribe_port(&sub) {
                                Err(e) if e.errno() == 16 => Ok(()), // EBUSY: already connected
                                r => r,
                            }
                        };
                        match connect(f.addr, me).and_then(|_| connect(me, f.addr)) {
                            Ok(()) => {
                                s.adi = Some(f.addr);
                                drop(s);
                                problem(&engine, None);
                                let hint = model_hint(&f.name).map(|h| h.to_string());
                                send(&engine, LinkEvent::Open { port: format!("{} (ALSA {}:{})", f.name, f.addr.client, f.addr.port), hint });
                            }
                            Err(e) => {
                                drop(s);
                                problem(&engine, Some(Problem::new("busy", format!("Can't connect to {} ({e})", f.name))));
                            }
                        }
                    }
                    (None, Some(_)) => {
                        s.adi = None;
                        drop(s);
                        send(&engine, LinkEvent::Closed("unplugged".into()));
                    }
                    (None, None) => {
                        drop(s);
                        problem(
                            &engine,
                            Some(Problem::new("notfound", "No ADI-2 found. Connect it by USB and set MIDI Control to ON.")),
                        );
                    }
                    _ => {}
                }
            }

            // wait for input
            let mut pfds = fds.clone();
            match poll::poll(&mut pfds, 200) {
                Ok(0) => continue,
                Ok(_) => {}
                Err(_) => continue,
            }
            let mut s = shared.lock().unwrap();
            let adi = s.adi;
            let seq = s.seq.as_mut().unwrap();
            let mut input = seq.input();
            loop {
                match input.event_input() {
                    Ok(ev) => match ev.get_type() {
                        EventType::Sysex => {
                            if Some(ev.get_source()) == adi {
                                if let Some(d) = ev.get_ext() {
                                    buf_events.push(d.to_vec());
                                }
                            }
                        }
                        EventType::ClientStart | EventType::PortStart => rescan = true,
                        EventType::ClientExit | EventType::PortExit | EventType::PortUnsubscribed => rescan = true,
                        _ => {}
                    },
                    Err(e) if e.errno() == libc_eagain() => break,
                    Err(e) if e.errno() == libc_enospc() => {
                        // input buffer overrun: the kernel dropped events; a re-read fixes the model
                        continue;
                    }
                    Err(_) => {
                        drop(input);
                        s.seq = None;
                        drop(s);
                        if adi.is_some() {
                            send(&engine, LinkEvent::Closed("sequencer error".into()));
                        }
                        break 'connected;
                    }
                }
            }
            drop(input);
            drop(s);
            for d in buf_events.drain(..) {
                if !send(&engine, LinkEvent::Data(d)) {
                    return;
                }
            }
        }
        sleep_unless(&stop, Duration::from_millis(500));
    }
    let mut s = shared.lock().unwrap();
    s.adi = None;
    s.seq = None;
}

fn libc_eagain() -> i32 {
    11
}
fn libc_enospc() -> i32 {
    28
}

fn sleep_unless(stop: &AtomicBool, d: Duration) {
    let until = Instant::now() + d;
    while Instant::now() < until && !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(100));
    }
}
