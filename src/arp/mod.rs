#[allow(unused_imports)]

use std::io;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::net::Ipv4Addr;

use pnet::util::MacAddr;

use ethernet::EthernetListener;
use VersionedTx;

mod arp_rx;
mod arp_tx;

pub use self::arp_rx::ArpRx;
pub use self::arp_tx::{ArpBuilder, ArpTx};

pub struct TableData {
    table: HashMap<Ipv4Addr, MacAddr>,
    listeners: HashMap<Ipv4Addr, Vec<Sender<MacAddr>>>,
}

impl TableData {
    pub fn new() -> Self {
        TableData {
            table: HashMap::new(),
            listeners: HashMap::new(),
        }
    }
}

impl Default for TableData {
    fn default() -> Self {
        Self::new()
    }
}

/// The main Arp table struct. Contains the actual data behind a `Mutex` so it
/// can be shared
/// with `ArpRx` instances.
#[derive(Clone)]
pub struct ArpTable {
    data: Arc<Mutex<TableData>>,
}

impl ArpTable {
    /// Creates a new `ArpTable` with no entries in it.
    pub fn new() -> ArpTable {
        let data = Arc::new(Mutex::new(TableData::new()));
        ArpTable { data: data }
    }

    /// Creates a new `ArpRx` cast to a `Box<EthernetListener>` so that it can
    /// easily be added
    /// to a `Vec` and passed to `EthernetRx` as a listener.
    /// The `ArpRx` created here will share the table with this `ArpTable`.
    /// The given `VersionedTx` will have its revision bumped upon incoming Arp
    /// packet.
    pub fn arp_rx(&self, vtx: Arc<Mutex<VersionedTx>>) -> Box<EthernetListener> {
        Box::new(ArpRx::new(self.data.clone(), vtx)) as Box<EthernetListener>
    }

    /// Queries the table for a MAC. If it does not exist a request is sent and
    /// the call is blocked
    /// until a reply has arrived
    pub fn get(&mut self, target_ip: Ipv4Addr) -> Result<MacAddr, Receiver<MacAddr>> {
        let mut data = self.data.lock().unwrap();
        if let Some(mac) = data.table.get(&target_ip) {
            return Ok(*mac);
        }
        Err(Self::add_listener(&mut data, target_ip))
    }

    /// Manually insert an IP -> MAC mapping into this Arp table
    // TODO: This should also invalidate the Tx
    pub fn insert(&mut self, ip: Ipv4Addr, mac: MacAddr) {
        let mut data = self.data.lock().expect("Unable to lock Arp::table for writing");
        data.table.insert(ip, mac);
    }

    fn add_listener(data: &mut TableData, ip: Ipv4Addr) -> Receiver<MacAddr> {
        let (tx, rx) = channel();
        data.listeners.entry(ip).or_insert(vec![]).push(tx);
        rx
    }
}

impl Default for ArpTable {
    fn default() -> Self {
        Self::new()
    }
}
