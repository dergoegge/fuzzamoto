pub mod bitcoin_core;
pub use bitcoin_core::BitcoinCoreTarget;

use crate::{
    connections::{Connection, ConnectionType, Transport},
    targets::bitcoin_core::TxOutSetInfo,
};
use bitcoin::{Block, BlockHash};
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

pub trait HasGetBestBlockHash {
    fn getbestblockhash(&self) -> Option<BlockHash>;
}

pub trait HasGetBlockCount {
    fn getblockcount(&self) -> Option<u64>;
}

pub trait HasGetBlockHash {
    fn getblockhash(&self, height: u64) -> Option<BlockHash>;
}

pub trait HasGetBlock {
    fn getblock(&self, hash: BlockHash) -> Option<Block>;
}

pub trait HasGetTxOutSetInfo {
    fn gettxoutsetinfo(&self) -> Result<TxOutSetInfo, String>;
}

pub trait HasBlockChainRPC:
    HasGetBlockCount + HasGetBlockHash + HasGetBlock + HasGetTxOutSetInfo + HasGetBestBlockHash
{
}

// blanket impl
impl<
    Target: HasGetBlockCount + HasGetBlockHash + HasGetBlock + HasGetTxOutSetInfo + HasGetBestBlockHash,
> HasBlockChainRPC for Target
{
}
