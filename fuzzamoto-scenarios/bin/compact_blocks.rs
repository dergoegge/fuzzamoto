use fuzzamoto::{
    connections::Transport,
    fuzzamoto_main,
    runners::Runner,
    scenarios::{
        IgnoredCharacterization, Scenario, ScenarioInput, ScenarioResult, generic::GenericScenario,
    },
    targets::{BitcoinCoreTarget, Target},
    test_utils,
};

use bitcoin::{
    Amount, BlockHash,
    bip152::{BlockTransactions, HeaderAndShortIds, PrefilledTransaction, ShortId},
    consensus::encode::{self, Decodable, Encodable, VarInt},
    p2p::message::NetworkMessage,
    p2p::{
        message_blockdata::Inventory,
        message_compact_blocks::{BlockTxn, CmpctBlock},
    },
};

use std::io::{self, Read, Write};

// Create a newtype wrapper around Vec<u16>
struct TxIndices(Vec<u16>);

enum Action {
    /// Construct a new block for relay
    ConstructBlock {
        /// Id of the connection that will send the block
        from: u16,
        /// Id of the previous block
        prev: u16,
        /// Id of the block containing the funding coinbase
        funding: u16,
        /// Number of transactions in the block
        num_txs: u16,
    },
    /// Send an `inv` message to the target node for a previously constructed block
    SendInv { block: u16 },
    /// Send a `headers` message to the target node for a previously constructed block
    SendHeaders { block: u16 },
    /// Send a `cmpctblock` message to the target node for a previously constructed block
    SendCmpctBlock {
        block: u16,
        prefilled_txs: TxIndices,
    },
    /// Send a `block` message to the target node for a previously constructed block
    SendBlock { block: u16 },
    /// Send a `tx` message to the target node for a previously constructed block
    SendTxFromBlock { block: u16, tx: u16 },
    /// Send a `blocktxn` message to the target node for a previously constructed block
    SendBlockTxn { block: u16, txs: TxIndices },
    /// Advance the mocktime of the target node
    AdvanceTime { seconds: u16 },
}

struct TestCase {
    actions: Vec<Action>,
}

impl ScenarioInput for TestCase {
    fn decode(bytes: &[u8]) -> Result<Self, String> {
        TestCase::consensus_decode(&mut &bytes[..]).map_err(|e| e.to_string())
    }
}

/// `CompactBlocksScenario` is a scenario that tests the compact block relay protocol.
///
/// The scenario setup creates a couple of connections to the target node and mines a chain of 200
/// blocks. Testcases simulate the processing of a series of compact block protocol messages by the
/// target node, i.e. each testcase represents a series of different types of actions:
///
/// 1. Construct a new block for relay
/// 2. Send an `inv` message to the target node for a previously constructed block
/// 3. Send a `headers` message to the target node for a previously constructed block
/// 4. Send a `cmpctblock` message to the target node for a previously constructed block
/// 5. Send a `block` message to the target node for a previously constructed block
/// 6. Send a `tx` message to the target node for a previously constructed block
/// 7. Send a `blocktxn` message to the target node for a previously constructed block
/// 8. Advance the mocktime of the target node
struct CompactBlocksScenario<TX: Transport, T: Target<TX>> {
    inner: GenericScenario<TX, T>,
}

impl<TX: Transport, T: Target<TX>> Scenario<TestCase, IgnoredCharacterization, TX, T>
    for CompactBlocksScenario<TX, T>
{
    fn new(target: &mut T) -> Result<Self, String> {
        let inner = GenericScenario::new(target)?;

        Ok(Self { inner })
    }

    fn run(
        &mut self,
        target: &mut T,
        testcase: TestCase,
    ) -> ScenarioResult<IgnoredCharacterization> {
        let mut constructed = Vec::new();

        let mut prevs: Vec<(u32, BlockHash, bitcoin::OutPoint)> = self
            .inner
            .block_tree
            .iter()
            .map(|(hash, (block, height))| {
                (
                    *height,
                    *hash,
                    bitcoin::OutPoint::new(block.txdata[0].txid(), 0),
                )
            })
            .collect();

        prevs.sort_by_key(|(height, _, _)| *height);

        for action in testcase.actions {
            match action {
                Action::ConstructBlock {
                    from,
                    prev,
                    funding,
                    num_txs,
                } => {
                    let prev = prevs[180..][prev as usize % (prevs.len() - 180)];
                    let Ok(mut block) = test_utils::mining::mine_block(
                        prev.1,
                        prev.0 + 1,
                        self.inner.time as u32 + 1,
                    ) else {
                        continue;
                    };

                    // Create a chain of `num_txs` transactions, each spending the previous one (one in one out).
                    let funding_outpoint = prevs[1..100][funding as usize % 100].2;
                    let mut avaliable_outpoints =
                        vec![(funding_outpoint, Amount::from_int_btc(25))];
                    for _ in 0..num_txs {
                        let Ok(tx) =
                            test_utils::create_consolidation_tx(avaliable_outpoints.as_slice())
                        else {
                            break;
                        };
                        block.txdata.push(tx);

                        let tx = block.txdata.last().unwrap();
                        let outpoint = bitcoin::OutPoint::new(tx.txid(), 0);

                        avaliable_outpoints.pop();
                        avaliable_outpoints.push((outpoint, tx.output[0].value));
                    }

                    test_utils::mining::fixup_commitments(&mut block);
                    test_utils::mining::fixup_proof_of_work(&mut block);

                    constructed.push((from as usize % self.inner.connections.len(), block));
                }
                Action::SendInv { block } => {
                    if constructed.is_empty() {
                        continue;
                    }
                    let idx = block as usize % constructed.len();
                    let block_hash = constructed[idx].1.block_hash();
                    let inv = NetworkMessage::Inv(vec![Inventory::Block(block_hash)]);
                    let _ = self.inner.connections[constructed[idx].0]
                        .send(&("inv".to_string(), encode::serialize(&inv)));
                }
                Action::SendHeaders { block } => {
                    if constructed.is_empty() {
                        continue;
                    }
                    let idx = block as usize % constructed.len();
                    let header = constructed[idx].1.header;
                    let headers = NetworkMessage::Headers(vec![header]);
                    let _ = self.inner.connections[constructed[idx].0]
                        .send(&("headers".to_string(), encode::serialize(&headers)));
                }
                Action::SendCmpctBlock {
                    block,
                    prefilled_txs,
                } => {
                    if constructed.is_empty() {
                        continue;
                    }
                    let idx = block as usize % constructed.len();
                    let block = &constructed[idx].1;

                    // Sort and deduplicate the prefilled transaction indices
                    let mut sorted_prefilled_txs: Vec<usize> = prefilled_txs
                        .0
                        .iter()
                        .map(|tx| *tx as usize % block.txdata.len())
                        .collect();
                    sorted_prefilled_txs.sort();
                    sorted_prefilled_txs.dedup();

                    // Create prefilled transactions with differential encoding
                    let mut prev_idx = 0;
                    let prefilled_transactions: Vec<PrefilledTransaction> = sorted_prefilled_txs
                        .clone()
                        .into_iter()
                        .map(|tx_idx| {
                            // Calculate differential index
                            let diff_idx = if tx_idx == 0 {
                                0 // First index is not differential
                            } else {
                                tx_idx - prev_idx - 1
                            };
                            prev_idx = tx_idx;

                            PrefilledTransaction {
                                idx: diff_idx as u16,
                                tx: block.txdata[tx_idx].clone(),
                            }
                        })
                        .collect();

                    let nonce = 0u64;
                    let siphash_keys = ShortId::calculate_siphash_keys(&block.header, nonce);

                    // Collect short IDs for all transactions except prefilled ones
                    let short_ids: Vec<ShortId> = block
                        .txdata
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| sorted_prefilled_txs.binary_search(i).is_err())
                        .map(|(_, tx)| ShortId::with_siphash_keys(&tx.wtxid(), siphash_keys))
                        .collect();

                    let header_and_short_ids = HeaderAndShortIds {
                        header: block.header,
                        nonce,
                        short_ids,
                        prefilled_txs: prefilled_transactions,
                    };

                    let cmpctblock = NetworkMessage::CmpctBlock(CmpctBlock {
                        compact_block: header_and_short_ids,
                    });
                    let _ = self.inner.connections[constructed[idx].0]
                        .send(&("cmpctblock".to_string(), encode::serialize(&cmpctblock)));
                }
                Action::SendBlock { block } => {
                    if constructed.is_empty() {
                        continue;
                    }
                    let idx = block as usize % constructed.len();
                    let _ = self.inner.connections[constructed[idx].0]
                        .send(&("block".to_string(), encode::serialize(&constructed[idx].1)));
                }
                Action::SendTxFromBlock { block, tx } => {
                    if constructed.is_empty() {
                        continue;
                    }
                    let idx = block as usize % constructed.len();
                    let block = &constructed[idx].1;
                    let tx = tx as usize % block.txdata.len();
                    let _ = self.inner.connections[constructed[idx].0]
                        .send(&("tx".to_string(), encode::serialize(&block.txdata[tx])));
                }
                Action::SendBlockTxn { block, txs } => {
                    if constructed.is_empty() {
                        continue;
                    }
                    let idx = block as usize % constructed.len();
                    let block = &constructed[idx].1;
                    let txs_indices: Vec<usize> = txs
                        .0
                        .iter()
                        .map(|tx| *tx as usize % block.txdata.len())
                        .collect();
                    let blocktxn = NetworkMessage::BlockTxn(BlockTxn {
                        transactions: BlockTransactions {
                            block_hash: block.block_hash(),
                            transactions: txs_indices
                                .iter()
                                .map(|tx| block.txdata[*tx].clone())
                                .collect(),
                        },
                    });
                    let _ = self.inner.connections[constructed[idx].0]
                        .send(&("blocktxn".to_string(), encode::serialize(&blocktxn)));
                }
                Action::AdvanceTime { seconds } => {
                    self.inner.time += seconds as u64;
                    let _ = target.set_mocktime(self.inner.time);
                }
            }
        }

        for connection in self.inner.connections.iter_mut() {
            let _ = connection.ping();
        }

        if let Err(e) = target.is_alive() {
            return ScenarioResult::Fail(format!("Target is not alive: {}", e));
        }

        ScenarioResult::Ok(IgnoredCharacterization)
    }
}

impl Encodable for TxIndices {
    fn consensus_encode<W: Write + ?Sized>(&self, s: &mut W) -> Result<usize, io::Error> {
        let mut len = 0;
        len += VarInt(self.0.len() as u64).consensus_encode(s)?;
        for item in &self.0 {
            len += item.consensus_encode(s)?;
        }
        Ok(len)
    }
}

impl Decodable for TxIndices {
    fn consensus_decode<D: Read + ?Sized>(d: &mut D) -> Result<Self, encode::Error> {
        let len = VarInt::consensus_decode(d)?.0;
        if len > 10000 {
            return Err(encode::Error::ParseFailed("too many items"));
        }
        let mut items = Vec::with_capacity(len as usize);
        for _ in 0..len {
            items.push(u16::consensus_decode(d)?);
        }
        Ok(TxIndices(items))
    }
}

impl Encodable for Action {
    fn consensus_encode<W: Write + ?Sized>(&self, s: &mut W) -> Result<usize, io::Error> {
        match self {
            Action::ConstructBlock {
                from,
                prev,
                funding,
                num_txs,
            } => {
                let mut len = 0;
                len += 0u8.consensus_encode(s)?; // Tag for ConstructBlock
                len += from.consensus_encode(s)?;
                len += prev.consensus_encode(s)?;
                len += funding.consensus_encode(s)?;
                len += num_txs.consensus_encode(s)?;
                Ok(len)
            }
            Action::SendInv { block } => {
                let mut len = 0;
                len += 1u8.consensus_encode(s)?; // Tag for SendInv
                len += block.consensus_encode(s)?;
                Ok(len)
            }
            Action::SendHeaders { block } => {
                let mut len = 0;
                len += 2u8.consensus_encode(s)?; // Tag for SendHeaders
                len += block.consensus_encode(s)?;
                Ok(len)
            }
            Action::SendCmpctBlock {
                block,
                prefilled_txs,
            } => {
                let mut len = 0;
                len += 3u8.consensus_encode(s)?; // Tag for SendCmpctBlock
                len += block.consensus_encode(s)?;
                len += prefilled_txs.consensus_encode(s)?;
                Ok(len)
            }
            Action::SendBlock { block } => {
                let mut len = 0;
                len += 4u8.consensus_encode(s)?; // Tag for SendBlock
                len += block.consensus_encode(s)?;
                Ok(len)
            }
            Action::SendTxFromBlock { block, tx } => {
                let mut len = 0;
                len += 5u8.consensus_encode(s)?; // Tag for SendTxFromBlock
                len += block.consensus_encode(s)?;
                len += tx.consensus_encode(s)?;
                Ok(len)
            }
            Action::SendBlockTxn { block, txs } => {
                let mut len = 0;
                len += 6u8.consensus_encode(s)?; // Tag for SendBlockTxn
                len += block.consensus_encode(s)?;
                len += txs.consensus_encode(s)?;
                Ok(len)
            }
            Action::AdvanceTime { seconds } => {
                let mut len = 0;
                len += 7u8.consensus_encode(s)?; // Tag for AdvanceTime
                len += seconds.consensus_encode(s)?;
                Ok(len)
            }
        }
    }
}

impl Decodable for Action {
    fn consensus_decode<D: Read + ?Sized>(d: &mut D) -> Result<Self, encode::Error> {
        let tag = u8::consensus_decode(d)? % 8;
        match tag {
            0 => {
                let from = u16::consensus_decode(d)?;
                let prev = u16::consensus_decode(d)?;
                let funding = u16::consensus_decode(d)?;
                let num_txs = u16::consensus_decode(d)?;
                Ok(Action::ConstructBlock {
                    from,
                    prev,
                    funding,
                    num_txs,
                })
            }
            1 => {
                let block = u16::consensus_decode(d)?;
                Ok(Action::SendInv { block })
            }
            2 => {
                let block = u16::consensus_decode(d)?;
                Ok(Action::SendHeaders { block })
            }
            3 => {
                let block = u16::consensus_decode(d)?;
                let prefilled_txs = TxIndices::consensus_decode(d)?;
                Ok(Action::SendCmpctBlock {
                    block,
                    prefilled_txs,
                })
            }
            4 => {
                let block = u16::consensus_decode(d)?;
                Ok(Action::SendBlock { block })
            }
            5 => {
                let block = u16::consensus_decode(d)?;
                let tx = u16::consensus_decode(d)?;
                Ok(Action::SendTxFromBlock { block, tx })
            }
            6 => {
                let block = u16::consensus_decode(d)?;
                let txs = TxIndices::consensus_decode(d)?;
                Ok(Action::SendBlockTxn { block, txs })
            }
            7 => {
                let seconds = u16::consensus_decode(d)?;
                Ok(Action::AdvanceTime { seconds })
            }
            _ => Err(encode::Error::ParseFailed("Invalid Action tag")),
        }
    }
}

impl Decodable for TestCase {
    fn consensus_decode<D: Read + ?Sized>(d: &mut D) -> Result<Self, encode::Error> {
        let len = VarInt::consensus_decode(d)?.0;
        if len > 100 {
            return Err(encode::Error::ParseFailed("too many actions"));
        }
        let mut actions = Vec::with_capacity(len as usize);
        for _ in 0..len {
            actions.push(Action::consensus_decode(d)?);
        }
        Ok(TestCase { actions })
    }
}

impl Encodable for TestCase {
    fn consensus_encode<W: Write + ?Sized>(&self, s: &mut W) -> Result<usize, io::Error> {
        let mut len = 0;
        len += VarInt(self.actions.len() as u64).consensus_encode(s)?;
        for action in &self.actions {
            len += action.consensus_encode(s)?;
        }
        Ok(len)
    }
}

fuzzamoto_main!(CompactBlocksScenario, BitcoinCoreTarget, TestCase);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produce_simple_test_case() {
        let mut test_case = TestCase {
            actions: Vec::new(),
        };
        test_case.actions.push(Action::ConstructBlock {
            from: 0,
            prev: 199,
            funding: 0,
            num_txs: 0,
        });
        test_case.actions.push(Action::SendInv { block: 0 });
        test_case.actions.push(Action::SendHeaders { block: 0 });
        test_case.actions.push(Action::SendCmpctBlock {
            block: 0,
            prefilled_txs: TxIndices(vec![0]),
        });
        let mut file = std::fs::File::create("compact_blocks_test_case.bin").unwrap();
        file.write_all(&encode::serialize(&test_case)).unwrap();
    }
}
