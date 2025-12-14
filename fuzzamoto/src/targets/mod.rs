pub mod bitcoin_core;
pub mod libbitcoin;

pub use bitcoin_core::BitcoinCoreTarget;
pub use libbitcoin::LibbitcoinTarget;

use crate::{
    connections::{Connection, ConnectionType, Transport},
    targets::bitcoin_core::TxOutSetInfo,
};
use std::net::SocketAddrV4;

/// `Target` is the interface that the test harness will use to interact with the target Bitcoin
/// implementation (e.g. Bitcoin Core, btcd, etc).
pub trait Target<T: Transport>: Sized {
    /// Create target from path to executable.
    fn from_path(path: &str) -> Result<Self, String>;

    /// Create a new network connection to the target.
    ///
    /// # Arguments
    ///
    /// * `connection_type` - The type of connection to create (either inbound or outbound)
    fn connect(&mut self, connection_type: ConnectionType) -> Result<Connection<T>, String>;

    /// Connect the target to another target.
    ///
    /// # Arguments
    ///
    /// * `other` - The other target to connect to.
    fn connect_to<O: ConnectableTarget>(&mut self, other: &O) -> Result<(), String>;

    /// Set the mocktime for the target.
    ///
    /// This is used to simulate time advancement in the target.
    ///
    /// # Arguments
    ///
    /// * `time` - The new mocktime to set.
    fn set_mocktime(&mut self, time: u64) -> Result<(), String>;

    /// Check if the target is still alive.
    fn is_alive(&self) -> Result<(), String>;
}

pub trait ConnectableTarget {
    fn get_addr(&self) -> Option<SocketAddrV4> {
        None
    }

    fn is_connected_to<O: ConnectableTarget>(&self, other: &O) -> bool;
}

pub trait HasTipHash {
    fn get_tip_hash(&self) -> Option<[u8; 32]>;
}

pub trait HasTxOutSetInfo {
    fn tx_out_set_info(&self) -> Result<TxOutSetInfo, String>;
}
