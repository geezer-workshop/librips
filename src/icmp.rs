use std::net::Ipv4Addr;
use std::io;

use pnet::packet::ip::IpNextHeaderProtocols;
use pnet::packet::icmp::{IcmpPacket, MutableIcmpPacket, checksum, icmp_types};
use pnet::packet::icmp::echo_request::{EchoRequestPacket, MutableEchoRequestPacket, icmp_codes};
use pnet::packet::ipv4::{Ipv4Packet, MutableIpv4Packet};
use pnet::packet::{Packet, MutablePacket};

use ipv4::{Ipv4, Ipv4Listener};

/// Struct used for listening on incoming Icmp packets
pub struct IcmpIpv4Listener;

impl Ipv4Listener for IcmpIpv4Listener {
    fn recv(&mut self, packet: Ipv4Packet) {
        let icmp_pkg = IcmpPacket::new(packet.payload()).unwrap();
        println!("Icmp got a packet with {} bytes!", icmp_pkg.payload().len());
    }
}

/// An Icmp communication struct.
#[derive(Clone)]
pub struct Icmp {
    ipv4: Ipv4,
}

impl Icmp {
    /// !
    pub fn new(ipv4: Ipv4) -> Icmp {
        let listener = IcmpIpv4Listener;
        ipv4.set_listener(IpNextHeaderProtocols::Icmp, listener);
        Icmp { ipv4: ipv4 }
    }

    /// !
    pub fn send<T>(&mut self,
                   dst_ip: Ipv4Addr,
                   payload_size: u16,
                   mut builder: T)
                   -> Option<io::Result<()>>
        where T: FnMut(&mut MutableIcmpPacket)
    {
        let total_size = IcmpPacket::minimum_packet_size() as u16 + payload_size;
        let mut builder_wrapper = |ip_pkg: &mut MutableIpv4Packet| {
            ip_pkg.set_next_level_protocol(IpNextHeaderProtocols::Icmp);

            let mut icmp_pkg = MutableIcmpPacket::new(ip_pkg.payload_mut()).unwrap();
            builder(&mut icmp_pkg);
            let checksum = checksum(&icmp_pkg.to_immutable());
            icmp_pkg.set_checksum(checksum);
        };
        self.ipv4.send(dst_ip, total_size, &mut builder_wrapper)
    }
}

/// !
pub struct Echo {
    icmp: Icmp,
}

impl Echo {
    /// !
    pub fn new(icmp: Icmp) -> Echo {
        Echo { icmp: icmp }
    }

    /// !
    pub fn send(&mut self, dst_ip: Ipv4Addr, payload: &[u8]) -> Option<io::Result<()>> {
        let total_size = (EchoRequestPacket::minimum_packet_size() -
                          IcmpPacket::minimum_packet_size() +
                          payload.len()) as u16;
        let mut builder_wrapper = |icmp_pkg: &mut MutableIcmpPacket| {
            icmp_pkg.set_icmp_type(icmp_types::EchoRequest);
            icmp_pkg.set_icmp_code(icmp_codes::NoCode);
            let mut echo_pkg = MutableEchoRequestPacket::new(icmp_pkg.packet_mut()).unwrap();
            echo_pkg.set_payload(payload);
        };
        self.icmp.send(dst_ip, total_size, &mut builder_wrapper)
    }
}
