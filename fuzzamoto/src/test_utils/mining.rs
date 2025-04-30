use bitcoin::{
    Amount, Block, BlockHash, CompactTarget, OutPoint, Sequence, Transaction, TxIn, TxMerkleNode,
    TxOut, Witness, block,
    blockdata::opcodes::{OP_0, OP_TRUE},
    hash_types::{WitnessMerkleNode, Wtxid},
    hashes::Hash,
    script::ScriptBuf,
    transaction,
};

use bitcoin_hashes::sha256;

// Consists of OP_RETURN, OP_PUSHBYTES_36, and four "witness header" bytes.
const WITNESS_COMMITMENT_MAGIC: [u8; 6] = [0x6a, 0x24, 0xaa, 0x21, 0xa9, 0xed];

fn create_witness_commitment_output(witness_merkle_root: WitnessMerkleNode) -> TxOut {
    let commitment = Block::compute_witness_commitment(&witness_merkle_root, &[0u8; 32]);

    let mut script_pubkey: Vec<u8> = WITNESS_COMMITMENT_MAGIC.to_vec();
    script_pubkey.extend(commitment.as_byte_array());

    TxOut {
        value: Amount::from_int_btc(0),
        script_pubkey: bitcoin::ScriptBuf::from_bytes(script_pubkey),
    }
}

fn find_witness_commitment_output(coinbase: &Transaction) -> Option<usize> {
    for (i, output) in coinbase.output.iter().enumerate() {
        if output.script_pubkey.len() >= 38
            && output.script_pubkey.as_bytes()[0..6] == WITNESS_COMMITMENT_MAGIC
        {
            return Some(i);
        }
    }

    None
}

pub fn fixup_commitments(block: &mut Block) {
    if let Some(output_index) =
        find_witness_commitment_output(block.txdata.get(0).expect("block should not be empty"))
    {
        let witness_merkle_root = Block::witness_root(block).unwrap();
        let coinbase = block.txdata.get_mut(0).unwrap();
        coinbase.output[output_index] = create_witness_commitment_output(witness_merkle_root);
    }

    block.header.merkle_root = block.compute_merkle_root().unwrap();
}

pub fn fixup_proof_of_work(block: &mut Block) {
    if cfg!(feature = "reduced_pow") {
        let mut block_hash = block.header.block_hash();
        while block_hash.as_raw_hash()[31] & 0x80 != 0 {
            block.header.nonce += 1;
            block_hash = block.header.block_hash();
        }
    } else {
        let target = block.header.target();
        while block.header.validate_pow(target).is_err() {
            block.header.nonce += 1;
        }
    }
}

pub fn mine_block(prev_hash: BlockHash, height: u32, time: u32) -> Result<Block, String> {
    let mut p2wsh_optrue_spk = vec![OP_0.to_u8(), 32];
    let op_true_hash = sha256::Hash::hash(&[OP_TRUE.to_u8()]);
    p2wsh_optrue_spk.extend(op_true_hash.as_byte_array().as_slice());

    let mut witness = Witness::new();
    witness.push([0u8; 32]);

    // Create a coinbase transaction
    let coinbase = Transaction {
        version: transaction::Version(1),
        lock_time: bitcoin::absolute::LockTime::from_height(0).unwrap(),
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::builder()
                .push_int(height as i64)
                .push_int(0xFFFFFFFF)
                .as_script()
                .into(),
            sequence: Sequence(0xFFFFFFFF),
            witness,
        }],
        output: vec![
            TxOut {
                value: Amount::from_int_btc(25),
                script_pubkey: p2wsh_optrue_spk.into(),
            },
            create_witness_commitment_output(WitnessMerkleNode::from_raw_hash(
                Wtxid::all_zeros().into(),
            )),
        ],
    };

    // Create the block
    let mut block = Block {
        header: block::Header {
            version: block::Version::from_consensus(5),
            prev_blockhash: prev_hash,
            merkle_root: TxMerkleNode::from_raw_hash(*coinbase.compute_txid().as_raw_hash()),
            time,
            bits: CompactTarget::from_consensus(0x207fffff),
            nonce: 0,
        },
        txdata: vec![coinbase],
    };

    fixup_proof_of_work(&mut block);

    Ok(block)
}

