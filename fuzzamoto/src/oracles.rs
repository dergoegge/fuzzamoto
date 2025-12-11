use crate::{
    connections::Transport,
    targets::{
        ConnectableTarget, HasGetBestBlockHash, HasGetTxOutSetInfo, Target,
        bitcoin_core::TxOutSetInfo,
    },
};
use std::{
    marker::PhantomData,
    time::{Duration, Instant},
};

pub enum OracleResult {
    Pass,
    Fail(String),
}

pub trait Oracle<C> {
    fn evaluate(&self, context: &C) -> OracleResult;
    fn name(&self) -> &str;
}

/// `CrashOracle` checks if a given target is still alive
pub struct CrashOracle<TX>(PhantomData<TX>);

impl<TX> Default for CrashOracle<TX> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<T, TX> Oracle<T> for CrashOracle<TX>
where
    TX: Transport,
    T: Target<TX>,
{
    fn evaluate(&self, target: &T) -> OracleResult {
        match target.is_alive() {
            Ok(_) => OracleResult::Pass,
            Err(err) => OracleResult::Fail(format!("Target is not alive: {}", err)),
        }
    }

    fn name(&self) -> &str {
        "CrashOracle"
    }
}

/// `ConsensusContext` is the context for the `ConsensusOracle`
pub struct ConsensusContext<'a, T1, T2> {
    pub primary: &'a T1,
    pub reference: &'a T2,
    pub consensus_timeout: Duration,
    pub poll_interval: Duration,
}

/// `ConsensusOracle` checks if two full node targets reach consensus on the chain tip hash
pub struct ConsensusOracle<TX1, TX2>(PhantomData<TX1>, PhantomData<TX2>);

impl<TX1, TX2> Default for ConsensusOracle<TX1, TX2> {
    fn default() -> Self {
        Self(PhantomData, PhantomData)
    }
}

impl<'a, T1, T2, TX1, TX2> Oracle<ConsensusContext<'a, T1, T2>> for ConsensusOracle<TX1, TX2>
where
    TX1: Transport,
    TX2: Transport,
    T1: Target<TX1> + HasGetBestBlockHash,
    T2: Target<TX2> + HasGetBestBlockHash,
{
    fn evaluate(&self, context: &ConsensusContext<'a, T1, T2>) -> OracleResult {
        let start = Instant::now();

        let mut primary_tip = None;
        let mut reference_tip = None;

        while start.elapsed() < context.consensus_timeout {
            primary_tip = context.primary.getbestblockhash();
            reference_tip = context.reference.getbestblockhash();

            if primary_tip.is_some() && primary_tip == reference_tip {
                // Consensus is reached if tips match and are not None.
                return OracleResult::Pass;
            }

            std::thread::sleep(context.poll_interval);
        }

        OracleResult::Fail(format!(
            "Nodes did not reach consensus within {:?}. Primary: {primary_tip:?}, Reference: {reference_tip:?}",
            context.consensus_timeout
        ))
    }

    fn name(&self) -> &str {
        "ConsensusOracle"
    }
}

pub struct NetSplitContext<'a, T1, T2> {
    pub primary: &'a T1,
    pub reference: &'a T2,
}

/// `NetSplitOracle` checks if two full node targets are connected
pub struct NetSplitOracle<TX1, TX2>(PhantomData<TX1>, PhantomData<TX2>);

impl<TX1, TX2> Default for NetSplitOracle<TX1, TX2> {
    fn default() -> Self {
        Self(PhantomData, PhantomData)
    }
}

impl<'a, T1, T2, TX1, TX2> Oracle<NetSplitContext<'a, T1, T2>> for NetSplitOracle<TX1, TX2>
where
    TX1: Transport,
    TX2: Transport,
    T1: Target<TX1> + ConnectableTarget,
    T2: Target<TX2> + ConnectableTarget,
{
    fn evaluate(&self, context: &NetSplitContext<'a, T1, T2>) -> OracleResult {
        match context.reference.is_connected_to(context.primary) {
            false => OracleResult::Fail("Nodes are no longer connected!".to_string()),
            true => OracleResult::Pass,
        }
    }

    fn name(&self) -> &str {
        "NetSplitOracle"
    }
}

#[derive(Debug, Copy, Clone)]
pub struct InflationOracle<TX> {
    phantom: PhantomData<TX>,
}

impl<TX> Default for InflationOracle<TX> {
    fn default() -> Self {
        Self {
            phantom: PhantomData,
        }
    }
}

use bitcoin::Amount;

/// compute the (maximum possible) total number of bitcoins that has been produced at the given height
pub fn total_coins_until(height: u64) -> Result<Amount, String> {
    let initial: Amount = Amount::from_int_btc(50); // 50 btc in satoshis; can't fail for sure
    const HALVING_INTERVAL: u64 = 150; // 150 for `-regtest` mode
    const MAX_HALVINGS: u64 = 64;

    let mut total: Amount = Amount::ZERO;
    let mut subsidy = initial;

    for halving in 0..MAX_HALVINGS {
        if subsidy == Amount::ZERO {
            break;
        }

        let start = halving * HALVING_INTERVAL;
        let end = start + HALVING_INTERVAL;

        if height < start {
            break;
        }

        // How many blocks in this halving period are included?
        // we compute them and add to the total subsidy iteratively for each period
        let blocks_in_period = if height >= end {
            HALVING_INTERVAL
        } else {
            height - start + 1
        };

        total += subsidy
            .checked_mul(blocks_in_period)
            .ok_or_else(|| "Failed to compute the total amount of coins mined".to_string())?;

        subsidy = subsidy
            .checked_div(2)
            .ok_or_else(|| "Failed to compute the total amount of coins mined".to_string())?;
    }
    Ok(total)
}

impl<TX> InflationOracle<TX> {
    pub fn is_amount_valid(&self, info: &TxOutSetInfo) -> Result<bool, String> {
        Ok(info.amount() <= total_coins_until(info.height())?)
    }
}

impl<T, TX> Oracle<T> for InflationOracle<TX>
where
    TX: Transport,
    T: Target<TX> + HasGetTxOutSetInfo,
{
    fn evaluate(&self, target: &T) -> OracleResult {
        let txoutsetinfo = match target.gettxoutsetinfo() {
            Ok(info) => info,
            Err(_) => return OracleResult::Fail("Failed to retrieve txoutsetinfo".to_string()),
        };

        if let Ok(boolean) = self.is_amount_valid(&txoutsetinfo) {
            if boolean {
                return OracleResult::Pass;
            } else {
                return OracleResult::Fail(
                    "total_amount exceeds the current maximum possible bitcoin supply".to_string(),
                );
            }
        } else {
            return OracleResult::Fail(
                "Failed to compute the total amount of coins mined".to_string(),
            );
        }
    }

    fn name(&self) -> &str {
        "InflationOracle"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correct_total_coins() {
        let bitcoin_core_subsidy = |height| {
            // https://github.com/bitcoin/bitcoin/blob/238c1c8933b1f7479a9bca2b7cb207d26151c39d/src/validation.cpp#L1919
            let halvings = height / 150;
            if halvings >= 64 {
                return Amount::ZERO;
            }
            let mut subsidy = Amount::from_int_btc(50);
            for _ in 0..halvings {
                subsidy = subsidy.checked_div(2).unwrap();
            }
            return subsidy;
        };

        for i in 0..1000 {
            let total = total_coins_until(i).unwrap();
            let mut expected_total = Amount::ZERO;
            for h in 0..=i {
                expected_total = expected_total.checked_add(bitcoin_core_subsidy(h)).unwrap();
            }
            println!(
                "Height: {}, Total: {}, Expected: {}",
                i, total, expected_total
            );
            assert_eq!(total, expected_total);
        }
    }
}
