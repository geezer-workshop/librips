use std::io;
use std::net::Ipv4Addr;
use std::collections::HashMap;
use std::convert::From;
use std::time::SystemTime;
use std::sync::{Arc, Mutex};

use pnet::packet::ip::IpNextHeaderProtocol;
use pnet::packet::ipv4::{Ipv4Packet, MutableIpv4Packet, checksum};
use pnet::packet::ethernet::{EtherType, EtherTypes, EthernetPacket, MutableEthernetPacket};
use pnet::packet::{MutablePacket, Packet};
use pnet::util::MacAddr;

use ipnetwork::{self, Ipv4Network};

use ethernet::{Ethernet, EthernetListener};
use arp::Arp;

/// Represents an error in an `IpConf`.
#[derive(Debug)]
pub enum IpConfError {
    /// The given network configuration was not valid. For example invalid
    /// prefix.
    InvalidNetwork(ipnetwork::IpNetworkError),

    /// The gateway is not inside the local network.
    GwNotInNetwork,
}

impl From<ipnetwork::IpNetworkError> for IpConfError {
    fn from(e: ipnetwork::IpNetworkError) -> Self {
        IpConfError::InvalidNetwork(e)
    }
}

/// IP settings for one `Ipv4` instance
#[derive(Clone)]
pub struct Ipv4Config {
    /// The ip of the local host represented by this `Ipv4Config`
    pub ip: Ipv4Addr,

    gw: Ipv4Addr,
    net: Ipv4Network,
}

impl Ipv4Config {
    /// Creates a new `Ipv4Config`.
    /// Checks so the gateways is inside the network, returns None otherwise.
    pub fn new(ip: Ipv4Addr, prefix: u8, gw: Ipv4Addr) -> Result<Ipv4Config, IpConfError> {
        let net = try!(Ipv4Network::new(ip, prefix));
        if !net.contains(gw) {
            Err(IpConfError::GwNotInNetwork)
        } else {
            Ok(Ipv4Config {
                ip: ip,
                gw: gw,
                net: net,
            })
        }
    }
}

/// Anyone interested in receiving IPv4 packets from `Ipv4` must implement this.
pub trait Ipv4Listener: Send {
    /// Called by the library to deliver an `Ipv4Packet` to a listener.
    fn recv(&mut self, time: SystemTime, packet: Ipv4Packet);
}

pub type IpListenerLookup = HashMap<Ipv4Addr, HashMap<IpNextHeaderProtocol, Box<Ipv4Listener>>>;

/// Struct listening for ethernet frames containing IPv4 packets.
pub struct Ipv4EthernetListener {
    listeners: Arc<Mutex<IpListenerLookup>>,
}

impl Ipv4EthernetListener {
    pub fn new(listeners: Arc<Mutex<IpListenerLookup>>) -> Box<EthernetListener> {
        let this = Ipv4EthernetListener { listeners: listeners };
        Box::new(this) as Box<EthernetListener>
    }
}

impl EthernetListener for Ipv4EthernetListener {
    fn recv(&mut self, time: SystemTime, pkg: &EthernetPacket) {
        let ip_pkg = Ipv4Packet::new(pkg.payload()).unwrap();
        let dest_ip = ip_pkg.get_destination();
        let next_level_protocol = ip_pkg.get_next_level_protocol();
        println!("Ipv4 got a packet to {}!", dest_ip);
        let mut listeners = self.listeners.lock().unwrap();
        if let Some(mut listeners) = listeners.get_mut(&dest_ip) {
            if let Some(mut listener) = listeners.get_mut(&next_level_protocol) {
                listener.recv(time, ip_pkg);
            } else {
                println!("Ipv4, no one was listening to {:?} :(", next_level_protocol);
            }
        } else {
            println!("Ipv4 is not listening to {} on this interface", dest_ip);
        }
    }

    fn get_ethertype(&self) -> EtherType {
        EtherTypes::Ipv4
    }
}

/// One IPv4 configuration on one ethernet interface.
// TODO: Remove concept of gateway. Instead make it able to communicate with the routing table
#[derive(Clone)]
pub struct Ipv4 {
    /// Configuration for this `Ipv4` instance
    pub config: Ipv4Config,

    ethernet: Ethernet,
    arp: Arp,
}

impl Ipv4 {
    /// Returns a new `Ipv4` instance for a given `Ethernet` interface.
    /// Does not have to be done manually, use a `Ipv4Factory` instead.
    pub fn new(ethernet: Ethernet, arp: Arp, config: Ipv4Config) -> Ipv4 {
        Ipv4 {
            config: config,
            ethernet: ethernet,
            arp: arp,
        }
    }

    /// Sends an IPv4 packet to the network. If the given `dst_ip` is within
    /// the local network it will be sent directly to the MAC of that IP (taken
    /// from arp), otherwise it will be sent to the MAC of the configured
    /// gateway.
    pub fn send<T>(&mut self,
                   dst_ip: Ipv4Addr,
                   payload_size: u16,
                   mut builder: T)
                   -> Option<io::Result<()>>
        where T: FnMut(&mut MutableIpv4Packet)
    {
        let total_size = Ipv4Packet::minimum_packet_size() as u16 + payload_size;
        // Get destination MAC before locking `eth` since the arp lookup might take
        // time.
        let dst_mac = self.get_dst_mac(dst_ip);
        let src_ip = self.config.ip;
        let mut builder_wrapper = |eth_pkg: &mut MutableEthernetPacket| {
            eth_pkg.set_destination(dst_mac);
            eth_pkg.set_ethertype(EtherTypes::Ipv4);
            {
                let mut ip_pkg = MutableIpv4Packet::new(eth_pkg.payload_mut()).unwrap();
                ip_pkg.set_version(4);
                ip_pkg.set_header_length(5); // 5 is for no option fields
                ip_pkg.set_dscp(0); // https://en.wikipedia.org/wiki/Differentiated_services
                ip_pkg.set_ecn(0); // https://en.wikipedia.org/wiki/Explicit_Congestion_Notification
                ip_pkg.set_total_length(total_size as u16);
                ip_pkg.set_identification(0); // Use when implementing fragmentation
                ip_pkg.set_flags(0x010); // Hardcoded to DF (don't fragment)
                ip_pkg.set_fragment_offset(0);
                ip_pkg.set_ttl(40);
                ip_pkg.set_source(src_ip);
                ip_pkg.set_destination(dst_ip);
                // ip_pkg.set_options(vec![]); // We currently don't support options in the
                // header
                builder(&mut ip_pkg);
                ip_pkg.set_checksum(0);
                let checksum = checksum(&ip_pkg.to_immutable());
                ip_pkg.set_checksum(checksum);
            }
        };
        self.ethernet.send(1, total_size as usize, &mut builder_wrapper)
    }

    /// Computes to what MAC to send a packet.
    /// If `ip` is within the local network directly get the MAC, otherwise
    /// gateway MAC.
    fn get_dst_mac(&mut self, ip: Ipv4Addr) -> MacAddr {
        let local_dst_ip = if self.config.net.contains(ip) {
            ip
        } else {
            // Destination outside our network, send to default gateway
            self.config.gw
        };
        self.arp.get(self.config.ip, local_dst_ip)
    }
}
