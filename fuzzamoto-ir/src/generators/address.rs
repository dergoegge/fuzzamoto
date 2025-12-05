use std::net::Ipv6Addr;

use bitcoin::p2p::ServiceFlags;
use rand::{Rng, RngCore, seq::SliceRandom};

use crate::{
    AddrNetwork, AddrRecord, Generator, GeneratorResult, Operation, PerTestcaseMetadata,
    ProgramBuilder,
};

/// Generates address relay sequences (`SendAddr`).
#[derive(Clone, Default)]
pub struct AddrRelayGenerator {
    addresses: Vec<AddrRecord>,
}

impl AddrRelayGenerator {
    pub fn new(addresses: Vec<AddrRecord>) -> Self {
        Self { addresses }
    }
}

impl<R: RngCore> Generator<R> for AddrRelayGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        _meta: Option<&mut PerTestcaseMetadata>,
    ) -> GeneratorResult {
        let v1_context: Vec<_> = self
            .addresses
            .iter()
            .filter_map(|addr| matches!(addr, AddrRecord::V1 { .. }).then_some(addr.clone()))
            .collect();
        let conn_var = builder.get_or_create_random_connection(rng);
        let mut_list = builder.force_append_expect_output(vec![], Operation::BeginBuildAddrList);

        let timestamp = builder.context().timestamp.min(u32::MAX as u64) as u32;
        let count = rng.gen_range(1..=MAX_ADDR_ENTRIES);

        for _ in 0..count {
            let addr = pick_or_generate_v1(&v1_context, rng, timestamp);
            let addr_var = builder.force_append_expect_output(vec![], Operation::LoadAddr(addr));
            builder.force_append(vec![mut_list.index, addr_var.index], Operation::AddAddr);
        }

        let list_var =
            builder.force_append_expect_output(vec![mut_list.index], Operation::EndBuildAddrList);
        builder.force_append(vec![conn_var.index, list_var.index], Operation::SendAddr);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "AddrRelayGenerator"
    }
}

/// Generates address relay sequences using `addrv2`.
#[derive(Clone, Default)]
pub struct AddrRelayV2Generator {
    addresses: Vec<AddrRecord>,
}

impl AddrRelayV2Generator {
    pub fn new(addresses: Vec<AddrRecord>) -> Self {
        Self { addresses }
    }
}

impl<R: RngCore> Generator<R> for AddrRelayV2Generator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
        _meta: Option<&mut PerTestcaseMetadata>,
    ) -> GeneratorResult {
        let v2_context: Vec<_> = self
            .addresses
            .iter()
            .filter_map(|addr| match addr {
                AddrRecord::V2 {
                    network: AddrNetwork::TorV2,
                    ..
                } => None,
                AddrRecord::V2 { .. } => Some(addr.clone()),
                _ => None,
            })
            .collect();
        let conn_var = builder.get_or_create_random_connection(rng);
        let mut_list = builder.force_append_expect_output(vec![], Operation::BeginBuildAddrListV2);

        let timestamp = builder.context().timestamp.min(u32::MAX as u64) as u32;
        let count = rng.gen_range(1..=MAX_ADDR_ENTRIES);

        for _ in 0..count {
            let addr = pick_or_generate_v2(&v2_context, rng, timestamp);
            let addr_var = builder.force_append_expect_output(vec![], Operation::LoadAddr(addr));
            builder.force_append(vec![mut_list.index, addr_var.index], Operation::AddAddrV2);
        }

        let list_var =
            builder.force_append_expect_output(vec![mut_list.index], Operation::EndBuildAddrListV2);
        builder.force_append(vec![conn_var.index, list_var.index], Operation::SendAddrV2);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "AddrRelayV2Generator"
    }
}

/// Fuzzing cap; BIP155 allows up to 1,000 entries per message.
const MAX_ADDR_ENTRIES: usize = 16;
pub(crate) const MAX_UNKNOWN_ADDR_PAYLOAD: usize = 512;

fn pick_or_generate_v1<R: RngCore>(
    context: &[AddrRecord],
    rng: &mut R,
    timestamp: u32,
) -> AddrRecord {
    // Reuse a context entry roughly 30% of the time if one exists.
    if !context.is_empty() && rng.gen_bool(0.3) {
        context.choose(rng).cloned().unwrap()
    } else {
        random_addr_v1(rng, timestamp)
    }
}

fn pick_or_generate_v2<R: RngCore>(
    context: &[AddrRecord],
    rng: &mut R,
    timestamp: u32,
) -> AddrRecord {
    // Reuse a context entry roughly 30% of the time if one exists.
    if !context.is_empty() && rng.gen_bool(0.3) {
        context.choose(rng).cloned().unwrap()
    } else {
        random_addr_v2(rng, timestamp)
    }
}

/// Build an addr (v1) record with mostly routable IPs and service flags.
pub(crate) fn random_addr_v1<R: RngCore>(rng: &mut R, timestamp: u32) -> AddrRecord {
    let (ip_bytes, port) = if rng.gen_bool(0.6) {
        let ipv4 = random_public_ipv4(rng);
        (ipv4_to_ipv6_mapped(ipv4), random_port(rng, None))
    } else {
        (random_global_ipv6(rng), random_port(rng, None))
    };

    AddrRecord::V1 {
        time: random_time(rng, timestamp),
        services: random_services(rng, false),
        ip: ip_bytes,
        port,
    }
}

/// Build an addrv2 record that keeps payload size within the per-network limit.
pub(crate) fn random_addr_v2<R: RngCore>(rng: &mut R, timestamp: u32) -> AddrRecord {
    let network = random_addr_network(rng);
    let services = random_services(
        rng,
        matches!(network, AddrNetwork::IPv4 | AddrNetwork::IPv6),
    );
    let payload = random_payload_for_network(rng, &network);

    AddrRecord::V2 {
        time: random_time(rng, timestamp),
        services,
        network,
        payload,
        port: random_port(rng, None),
    }
}

/// Pick a time close to the snapshot value to avoid identical timestamps.
pub(crate) fn random_time<R: RngCore>(rng: &mut R, base: u32) -> u32 {
    let delta = rng.gen_range(0..=86_400); // one day window
    if base == 0 {
        rng.r#gen()
    } else if rng.gen_bool(0.5) {
        base.saturating_add(delta)
    } else {
        base.saturating_sub(delta)
    }
}

/// Toggle common service flags, optionally preferring P2P v2 support.
pub(crate) fn random_services<R: RngCore>(rng: &mut R, prefer_v2: bool) -> u64 {
    let mut flags = ServiceFlags::NETWORK;

    for candidate in [
        ServiceFlags::WITNESS,
        ServiceFlags::COMPACT_FILTERS,
        ServiceFlags::NETWORK_LIMITED,
        ServiceFlags::GETUTXO,
        ServiceFlags::BLOOM,
        ServiceFlags::P2P_V2,
    ] {
        let bias = if prefer_v2 && candidate == ServiceFlags::P2P_V2 {
            0.8
        } else {
            0.5
        };
        if rng.gen_bool(bias) {
            flags |= candidate;
        }
    }

    flags.to_u64()
}

/// Pick a port, preferring Bitcoin defaults but allowing other values.
pub(crate) fn random_port<R: RngCore>(rng: &mut R, prefer: Option<u16>) -> u16 {
    if let Some(port) = prefer {
        if rng.gen_bool(0.3) {
            return port;
        }
    }

    rng.gen_range(1024..=65535)
}

/// Convert an IPv4 address into the IPv6-mapped representation used by addr v1.
pub(crate) fn ipv4_to_ipv6_mapped(ipv4: [u8; 4]) -> [u8; 16] {
    let mut ipv6 = [0u8; 16];
    ipv6[10] = 0xff;
    ipv6[11] = 0xff;
    ipv6[12..16].copy_from_slice(&ipv4);
    ipv6
}

/// Sample an IPv4 address, sometimes using non-routable ranges to exercise edge cases.
pub(crate) fn random_public_ipv4<R: RngCore>(rng: &mut R) -> [u8; 4] {
    if rng.gen_bool(0.2) {
        return rng.r#gen();
    }

    loop {
        let octets: [u8; 4] = rng.r#gen();
        if is_routable_ipv4(octets) {
            return octets;
        }
    }
}

/// Filter out loopback, private, multicast and documentation IPv4 ranges.
pub(crate) fn is_routable_ipv4(octets: [u8; 4]) -> bool {
    // Covers RFC1918, RFC5735, RFC6598, link-local, multicast, and doc ranges.
    match octets {
        [0, ..] => false,
        [10, ..] => false,
        [100, b2, ..] if (b2 & 0b1100_0000) == 0b0100_0000 => false, // 100.64/10
        [127, ..] => false,
        [169, 254, ..] => false,
        [172, b2 @ 16..=31, ..] if b2 >= 16 && b2 <= 31 => false,
        [192, 0, 0, ..] => false,
        [192, 0, 2, ..] => false,
        [192, 88, 99, ..] => false,
        [192, 168, ..] => false,
        [198, 18..=19, ..] => false,
        [198, 51, 100, ..] => false,
        [203, 0, 113, ..] => false,
        [224..=239, ..] => false,
        [240..=255, ..] => false,
        [_, 0, 0, 0] => false,
        _ => true,
    }
}

/// Sample an IPv6 address in the global unicast space.
pub(crate) fn random_global_ipv6<R: RngCore>(rng: &mut R) -> [u8; 16] {
    loop {
        let mut octets = [0u8; 16];
        rng.fill_bytes(&mut octets);
        // Force into global unicast range 2000::/3
        octets[0] = 0x20 | (octets[0] & 0x1f);

        if octets != Ipv6Addr::LOCALHOST.octets() {
            return octets;
        }
    }
}

/// Pick a network type, covering both common and less common transports (tor, i2p, etc.).
pub(crate) fn random_addr_network<R: RngCore>(rng: &mut R) -> AddrNetwork {
    let options = [
        AddrNetwork::IPv4,
        AddrNetwork::IPv6,
        AddrNetwork::TorV3,
        AddrNetwork::I2p,
        AddrNetwork::Cjdns,
        AddrNetwork::Yggdrasil,
        AddrNetwork::Unknown(random_unknown_network_id(rng)),
    ];
    options.choose(rng).cloned().unwrap()
}

/// Generate a network ID outside the reserved 0x01–0x07 range.
fn random_unknown_network_id<R: RngCore>(rng: &mut R) -> u8 {
    loop {
        let id: u8 = rng.r#gen();
        if !(1..=7).contains(&id) {
            return id;
        }
    }
}

/// Produce a payload of the correct length for the chosen network.
pub(crate) fn random_payload_for_network<R: RngCore>(
    rng: &mut R,
    network: &AddrNetwork,
) -> Vec<u8> {
    match network {
        AddrNetwork::IPv4 => random_public_ipv4(rng).to_vec(),
        AddrNetwork::IPv6 => random_global_ipv6(rng).to_vec(),
        AddrNetwork::TorV3 | AddrNetwork::I2p => {
            let mut buf = vec![0u8; 32];
            rng.fill_bytes(&mut buf);
            buf
        }
        AddrNetwork::Cjdns | AddrNetwork::Yggdrasil => {
            let mut buf = vec![0u8; 16];
            rng.fill_bytes(&mut buf);
            buf
        }
        AddrNetwork::Unknown(_) => {
            let len = rng.gen_range(1..=MAX_UNKNOWN_ADDR_PAYLOAD);
            let mut buf = vec![0u8; len];
            rng.fill_bytes(&mut buf);
            buf
        }
        AddrNetwork::TorV2 => unreachable!("torv2 records must not be generated"),
    }
}
