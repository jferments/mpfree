//! A server on loopback in a thread, a relay that can lose a datagram, and
//! a wait that never depends on a deadline.

#![allow(dead_code)]

use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use mpfree_offload::client::Remote;
use mpfree_offload::node_by_name;
use mpfree_offload::server::{self, Server};

pub struct Running {
    pub addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// A server for these nodes (by name), on 127.0.0.1.
pub fn server(nodes: &[&str]) -> Running {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = socket.local_addr().unwrap();
    let names: Vec<String> = nodes.iter().map(|s| s.to_string()).collect();
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let thread = std::thread::spawn(move || {
        let mut srv = Server::new(44_100);
        for n in &names {
            srv.add(node_by_name(n).unwrap());
        }
        server::serve(&socket, &mut srv, &s2).unwrap();
    });
    Running {
        addr,
        stop,
        thread: Some(thread),
    }
}

/// A relay between one client and `to` that loses the client's request
/// number `lose` (counting from 0).
pub fn relay(to: SocketAddr, lose: usize) -> Running {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_millis(20)))
        .unwrap();
    let addr = socket.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let thread = std::thread::spawn(move || {
        let mut client = None;
        let mut requests = 0;
        let mut buf = [0u8; 2048];
        while !s2.load(Ordering::Relaxed) {
            let Ok((n, from)) = socket.recv_from(&mut buf) else {
                continue;
            };
            if from == to {
                if let Some(c) = client {
                    let _ = socket.send_to(&buf[..n], c);
                }
            } else {
                client = Some(from);
                if requests != lose {
                    let _ = socket.send_to(&buf[..n], to);
                }
                requests += 1;
            }
        }
    });
    Running {
        addr,
        stop,
        thread: Some(thread),
    }
}

/// Waits (up to 10 s, which only a hang reaches) until block `seq`'s answer
/// is in. Tests use this instead of a deadline, so a busy machine can't
/// fail them.
pub fn wait_for(remote: &Remote, seq: u64) {
    let give_up = Instant::now() + Duration::from_secs(10);
    while !remote.has_answer(seq) {
        assert!(
            Instant::now() < give_up,
            "no answer for block {seq} in 10 s"
        );
        std::thread::sleep(Duration::from_micros(200));
    }
}

/// A test signal: noise bursts.
pub fn signal(k: u64, l: &mut [f32], r: &mut [f32]) {
    let mut x = k.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    for (i, (a, b)) in l.iter_mut().zip(r.iter_mut()).enumerate() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let on = (k / 40).is_multiple_of(2);
        let v = ((x >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * if on { 0.6 } else { 0.0 };
        *a = v;
        *b = v * if i % 2 == 0 { 0.5 } else { -0.5 };
    }
}
